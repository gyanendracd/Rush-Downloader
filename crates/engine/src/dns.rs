//! Secure DNS. Site names are looked up at Cloudflare's public resolver
//! (1.1.1.1) over HTTPS instead of at the network's own resolver, the way a
//! browser's "secure DNS" setting does. The helper programs (yt-dlp,
//! gallery-dl) and the embedded browser cannot do that themselves, so they
//! are sent through a small proxy on this computer that looks names up the
//! same way.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Cloudflare's resolver, reached by address so no lookup is needed first.
const RESOLVERS: [&str; 2] = ["https://1.1.1.1/dns-query", "https://1.0.0.1/dns-query"];
const KEEP: Duration = Duration::from_secs(300);

pub(crate) struct SecureDns {
    enabled: AtomicBool,
    client: reqwest::Client,
    cache: Mutex<HashMap<String, (Vec<IpAddr>, Instant)>>,
    proxy: OnceLock<SocketAddr>,
}

impl SecureDns {
    pub(crate) fn new(enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(enabled),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap_or_default(),
            cache: Mutex::new(HashMap::new()),
            proxy: OnceLock::new(),
        })
    }

    pub(crate) fn set(&self, enabled: bool) {
        self.enabled.store(enabled, Relaxed);
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Relaxed)
    }

    /// Names that only mean something on this computer or network.
    fn is_local(host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        host == "localhost" || !host.contains('.') || host.ends_with(".localhost") || host.ends_with(".local")
    }

    async fn system_lookup(host: &str) -> Result<Vec<IpAddr>, String> {
        tokio::net::lookup_host((host, 0))
            .await
            .map(|found| found.map(|a| a.ip()).collect())
            .map_err(|e| format!("could not find {host}: {e}"))
    }

    /// The addresses of `host`: from Cloudflare when secure DNS is on,
    /// otherwise from the system as usual.
    pub(crate) async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        if !self.enabled() || Self::is_local(host) {
            return Self::system_lookup(host).await;
        }
        if let Some((ips, at)) = self.cache.lock().unwrap().get(host) {
            if at.elapsed() < KEEP {
                return Ok(ips.clone());
            }
        }
        let mut last = String::from("no answer");
        let mut reached = false;
        // Address records first; the newer kind only if there are none.
        for kind in ["A", "AAAA"] {
            for resolver in RESOLVERS {
                match self.ask(resolver, host, kind).await {
                    Ok(ips) if !ips.is_empty() => {
                        self.cache.lock().unwrap().insert(host.to_string(), (ips.clone(), Instant::now()));
                        return Ok(ips);
                    }
                    Ok(_) => {
                        reached = true;
                        break;
                    }
                    Err(e) => last = e,
                }
            }
        }
        if !reached {
            // Cloudflare itself cannot be reached from here (some networks
            // block it): the network's own resolver is better than nothing.
            if let Ok(ips) = Self::system_lookup(host).await {
                if !ips.is_empty() {
                    return Ok(ips);
                }
            }
        }
        Err(format!("secure DNS could not find {host}: {last}"))
    }

    async fn ask(&self, resolver: &str, host: &str, kind: &str) -> Result<Vec<IpAddr>, String> {
        let body = self
            .client
            .get(resolver)
            .query(&[("name", host), ("type", kind)])
            .header("accept", "application/dns-json")
            .send()
            .await
            .map_err(|e| e.to_string())?
            .bytes()
            .await
            .map_err(|e| e.to_string())?;
        let answer: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
        Ok(read_answer(&answer))
    }

    /// The local proxy the helper programs and the embedded browser are sent
    /// through while secure DNS is on. Started on first use; it only listens
    /// on this computer.
    pub(crate) fn proxy(self: &Arc<Self>) -> Option<SocketAddr> {
        if !self.enabled() {
            return None;
        }
        if let Some(addr) = self.proxy.get() {
            return Some(*addr);
        }
        // RUSHDOWNLOAD_DNS_PROXY_PORT fixes the port, for tests.
        let port = std::env::var("RUSHDOWNLOAD_DNS_PROXY_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(0);
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).ok()?;
        listener.set_nonblocking(true).ok()?;
        let addr = listener.local_addr().ok()?;
        let listener = TcpListener::from_std(listener).ok()?;
        if self.proxy.set(addr).is_err() {
            return self.proxy.get().copied();
        }
        let dns = self.clone();
        tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else { return };
                let dns = dns.clone();
                tokio::spawn(async move {
                    let _ = relay(client, &dns).await;
                });
            }
        });
        Some(addr)
    }

    async fn connect(&self, host: &str, port: u16) -> Result<TcpStream, String> {
        let mut last = format!("no address for {host}");
        for ip in self.lookup(host).await? {
            match tokio::time::timeout(Duration::from_secs(12), TcpStream::connect((ip, port))).await {
                Ok(Ok(stream)) => return Ok(stream),
                Ok(Err(e)) => last = e.to_string(),
                Err(_) => last = "timed out".into(),
            }
        }
        Err(last)
    }
}

/// The addresses in a DNS-over-HTTPS answer (JSON form).
fn read_answer(answer: &serde_json::Value) -> Vec<IpAddr> {
    answer
        .get("Answer")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter(|record| matches!(record.get("type").and_then(|t| t.as_u64()), Some(1 | 28)))
        .filter_map(|record| record.get("data")?.as_str()?.parse().ok())
        .collect()
}

/// Where a proxied request wants to go, from its first line.
fn target(first_line: &str) -> Option<(String, u16, bool)> {
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    let to = parts.next()?;
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = to.rsplit_once(':')?;
        return Some((host.trim_matches(['[', ']']).to_string(), port.parse().ok()?, true));
    }
    let url = reqwest::Url::parse(to).ok()?;
    (url.scheme() == "http").then(|| {
        Some((
            url.host_str()?.trim_matches(['[', ']']).to_string(),
            url.port_or_known_default()?,
            false,
        ))
    })?
}

/// A plain request as its own server expects it. A proxy is sent the whole
/// address on the first line ("GET http://site/page"); servers want only the
/// path, and not every server accepts the long form. The connection is closed
/// after this one request, so a later one cannot slip through unchanged.
fn for_origin(head: &[u8]) -> Vec<u8> {
    let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") else {
        return head.to_vec();
    };
    let text = String::from_utf8_lossy(&head[..end]);
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or_default();
    let mut parts = first.splitn(3, ' ');
    let (Some(method), Some(to), Some(version)) = (parts.next(), parts.next(), parts.next()) else {
        return head.to_vec();
    };
    let Ok(url) = reqwest::Url::parse(to) else {
        return head.to_vec();
    };
    let mut path = url.path().to_string();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let mut out = format!("{method} {path} {version}\r\n");
    for line in lines {
        let name = line.split(':').next().unwrap_or_default().trim().to_ascii_lowercase();
        if name != "proxy-connection" && name != "connection" && !line.is_empty() {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    out.push_str("Connection: close\r\n\r\n");
    let mut out = out.into_bytes();
    // Anything already read past the headers (a request body) follows as it is.
    out.extend_from_slice(&head[end + 4..]);
    out
}

/// One connection through the proxy: find where it is going, connect there
/// with the secure lookup, then pass bytes both ways.
async fn relay(mut client: TcpStream, dns: &SecureDns) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = client.read(&mut chunk).await?;
        if n == 0 || head.len() > 64 * 1024 {
            return Ok(());
        }
        head.extend_from_slice(&chunk[..n]);
    }
    let first = String::from_utf8_lossy(&head).lines().next().unwrap_or_default().to_string();
    let Some((host, port, tunnel)) = target(&first) else {
        return client.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n").await;
    };
    let mut server = match dns.connect(&host, port).await {
        Ok(stream) => stream,
        Err(_) => return client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n").await,
    };
    if tunnel {
        client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await?;
    } else {
        server.write_all(&for_origin(&head)).await?;
    }
    tokio::io::copy_bidirectional(&mut client, &mut server).await.map(|_| ())
}

/// Plugs the lookup into the app's own HTTP client.
pub(crate) struct Resolver(pub(crate) Arc<SecureDns>);

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        let dns = self.0.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let ips = dns.lookup(&host).await?;
            let addrs: Addrs = Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_addresses_and_skips_aliases() {
        let answer: serde_json::Value = serde_json::from_str(
            r#"{"Status":0,"Answer":[
                {"name":"www.site.test","type":5,"TTL":60,"data":"site.test."},
                {"name":"site.test","type":1,"TTL":60,"data":"203.0.113.7"},
                {"name":"site.test","type":28,"TTL":60,"data":"2001:db8::7"}]}"#,
        )
        .unwrap();
        let ips = read_answer(&answer);
        assert_eq!(ips, vec!["203.0.113.7".parse::<IpAddr>().unwrap(), "2001:db8::7".parse().unwrap()]);
        assert!(read_answer(&serde_json::json!({"Status": 3})).is_empty());
    }

    #[test]
    fn finds_where_a_proxied_request_is_going() {
        assert_eq!(target("CONNECT site.test:443 HTTP/1.1"), Some(("site.test".into(), 443, true)));
        assert_eq!(target("GET http://site.test/a/b?c=1 HTTP/1.1"), Some(("site.test".into(), 80, false)));
        assert_eq!(target("GET http://127.0.0.1:8766/x HTTP/1.1"), Some(("127.0.0.1".into(), 8766, false)));
        assert_eq!(target("GET /relative HTTP/1.1"), None);
        assert_eq!(target("nonsense"), None);
    }

    #[test]
    fn plain_requests_reach_the_server_in_the_form_it_expects() {
        let sent = b"GET http://site.test:8080/watch/demo?x=1 HTTP/1.1\r\nHost: site.test:8080\r\nProxy-Connection: keep-alive\r\nRange: bytes=0-9\r\n\r\n";
        let got = String::from_utf8(for_origin(sent)).unwrap();
        assert_eq!(
            got,
            "GET /watch/demo?x=1 HTTP/1.1\r\nHost: site.test:8080\r\nRange: bytes=0-9\r\nConnection: close\r\n\r\n"
        );
        // a body that came with the headers is kept
        let post = b"POST http://site.test/api HTTP/1.1\r\nHost: site.test\r\nContent-Length: 3\r\n\r\nabc";
        assert!(String::from_utf8(for_origin(post)).unwrap().ends_with("Connection: close\r\n\r\nabc"));
    }

    #[test]
    fn local_names_stay_with_the_system() {
        assert!(SecureDns::is_local("localhost"));
        assert!(SecureDns::is_local("my-nas"));
        assert!(SecureDns::is_local("printer.local"));
        assert!(!SecureDns::is_local("example.com"));
    }
}
