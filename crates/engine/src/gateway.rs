//! A gateway on this computer that hands yt-dlp's and gallery-dl's
//! connections on to a proxy server.
//!
//! The helper programs could be given the proxy's address with its password
//! in it, but a command line can be read by any program running as the same
//! user. They are given this gateway instead, which only listens on
//! 127.0.0.1 and lets in nobody without the random key it was started with
//! (asked for the way an HTTP proxy asks for a password). The gateway then
//! connects through the proxy itself, signing in to it, or directly to the
//! sites on the proxy's bypass list, looked up with secure DNS when that is
//! on, as the app's own downloads are.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::dns::{for_origin, target, SecureDns};
use crate::proxy::{ProxyKind, Upstream};

const HEAD_LIMIT: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// The user name helpers sign in to the gateway with; the key is the password.
const USER: &str = "rush";

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
type Stream = Box<dyn Io>;

/// A running gateway for one proxy.
#[derive(Clone)]
pub(crate) struct Gateway {
    addr: SocketAddr,
    key: String,
}

impl Gateway {
    /// What to give a helper program's `--proxy`.
    pub(crate) fn proxy_arg(&self) -> String {
        format!("http://{USER}:{}@{}", self.key, self.addr)
    }

    /// Starts a gateway for `upstream`. Must be called inside the Tokio
    /// runtime; it runs until the app closes.
    pub(crate) fn start(upstream: Upstream, dns: Arc<SecureDns>) -> std::io::Result<Gateway> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let listener = TcpListener::from_std(listener)?;
        let key = random_key();
        let expected = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{key}")));
        let upstream = Arc::new(upstream);
        tracing::info!(proxy = ?upstream, %addr, "proxy gateway started");
        tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else { return };
                let (upstream, dns, expected) = (upstream.clone(), dns.clone(), expected.clone());
                tokio::spawn(async move {
                    if let Err(e) = serve(client, &upstream, &dns, &expected).await {
                        tracing::debug!(proxy = ?upstream, "gateway connection ended: {e}");
                    }
                });
            }
        });
        Ok(Gateway { addr, key })
    }
}

fn random_key() -> String {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 24];
    ring::rand::SystemRandom::new().fill(&mut bytes).expect("the system's random numbers");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The value of a header in a request's head, if it is there.
fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.split("\r\n").skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

/// Compares without stopping at the first difference, so how long an
/// answer takes says nothing about how much of a guess was right.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn serve(mut client: TcpStream, upstream: &Upstream, dns: &SecureDns, expected: &str) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = client.read(&mut chunk).await?;
        if n == 0 || head.len() > HEAD_LIMIT {
            return Ok(());
        }
        head.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    if !header(&text, "proxy-authorization").is_some_and(|given| same(given, expected)) {
        return client
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"RushDownload\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
    }
    let first = text.lines().next().unwrap_or_default().to_string();
    let Some((host, port, tunnel)) = target(&first) else {
        return client.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n").await;
    };

    let opened = tokio::time::timeout(CONNECT_TIMEOUT, open(upstream, dns, &host, port, tunnel, &head)).await;
    let (mut server, leftover) = match opened {
        Ok(Ok(opened)) => opened,
        Ok(Err(why)) => return refuse(&mut client, &why).await,
        Err(_) => return refuse(&mut client, "the proxy did not answer in time").await,
    };
    if tunnel {
        client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await?;
        if !leftover.is_empty() {
            client.write_all(&leftover).await?;
        }
    }
    tokio::io::copy_bidirectional(&mut client, &mut server).await.map(|_| ())
}

async fn refuse(client: &mut TcpStream, why: &str) -> std::io::Result<()> {
    tracing::warn!("proxy gateway could not connect: {why}");
    let body = why.replace(['\r', '\n'], " ");
    let answer = format!(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(answer.as_bytes()).await
}

/// Opens the way to `host:port`: directly for a bypassed site, else through
/// the proxy. For a plain (not tunnelled) request the request is sent on
/// already. Returns the connection and any bytes that came after the
/// proxy's answer to a tunnel.
async fn open(
    up: &Upstream,
    dns: &SecureDns,
    host: &str,
    port: u16,
    tunnel: bool,
    head: &[u8],
) -> Result<(Stream, Vec<u8>), String> {
    if up.bypass.matches(host) {
        let mut server: Stream = Box::new(dns.connect(host, port).await?);
        if !tunnel {
            server.write_all(&for_origin(head)).await.map_err(|e| e.to_string())?;
        }
        return Ok((server, Vec::new()));
    }
    let to_proxy = dns
        .connect(&up.host, up.port)
        .await
        .map_err(|e| format!("could not reach the proxy {}: {e}", up.authority()))?;
    match up.kind {
        ProxyKind::Http | ProxyKind::Https => {
            let mut proxy: Stream = if up.kind == ProxyKind::Https {
                Box::new(tls(&up.host, to_proxy).await?)
            } else {
                Box::new(to_proxy)
            };
            if tunnel {
                let leftover = http_connect(&mut proxy, up, host, port).await?;
                Ok((proxy, leftover))
            } else {
                proxy.write_all(&for_proxy(head, up)).await.map_err(|e| e.to_string())?;
                Ok((proxy, Vec::new()))
            }
        }
        ProxyKind::Socks4 | ProxyKind::Socks4a | ProxyKind::Socks5 | ProxyKind::Socks5h => {
            let mut server: Stream = socks(up, dns, to_proxy, host, port).await?;
            if !tunnel {
                server.write_all(&for_origin(head)).await.map_err(|e| e.to_string())?;
            }
            Ok((server, Vec::new()))
        }
    }
}

async fn socks(up: &Upstream, dns: &SecureDns, socket: TcpStream, host: &str, port: u16) -> Result<Stream, String> {
    use tokio_socks::tcp::{Socks4Stream, Socks5Stream};
    let fail = |e: tokio_socks::Error| match e {
        tokio_socks::Error::PasswordAuthFailure(_) | tokio_socks::Error::AuthorizationRequired => {
            "the proxy did not accept its user name and password".to_string()
        }
        other => format!("the proxy refused the connection: {other}"),
    };
    // SOCKS4 and SOCKS5 without the "h" are given an address looked up here.
    let resolved;
    let target: (&str, u16) = if up.kind.remote_dns() || host.parse::<std::net::IpAddr>().is_ok() {
        (host, port)
    } else {
        let ips = dns.lookup(host).await?;
        let ip = if up.kind == ProxyKind::Socks4 { ips.iter().find(|ip| ip.is_ipv4()) } else { ips.first() };
        resolved = ip.ok_or_else(|| format!("no address for {host} that the proxy can use"))?.to_string();
        (resolved.as_str(), port)
    };
    let user = up.username.as_str();
    Ok(match up.kind {
        ProxyKind::Socks5 | ProxyKind::Socks5h if !user.is_empty() => {
            let pass = up.password.as_deref().unwrap_or_default();
            Box::new(Socks5Stream::connect_with_password_and_socket(socket, target, user, pass).await.map_err(fail)?)
        }
        ProxyKind::Socks5 | ProxyKind::Socks5h => Box::new(Socks5Stream::connect_with_socket(socket, target).await.map_err(fail)?),
        _ if !user.is_empty() => Box::new(Socks4Stream::connect_with_userid_and_socket(socket, target, user).await.map_err(fail)?),
        _ => Box::new(Socks4Stream::connect_with_socket(socket, target).await.map_err(fail)?),
    })
}

fn proxy_auth(up: &Upstream) -> Option<String> {
    (!up.username.is_empty()).then(|| {
        let pair = format!("{}:{}", up.username, up.password.as_deref().unwrap_or_default());
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(pair))
    })
}

/// Asks an HTTP proxy for a tunnel to `host:port`.
async fn http_connect(proxy: &mut Stream, up: &Upstream, host: &str, port: u16) -> Result<Vec<u8>, String> {
    let to = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
    let mut ask = format!("CONNECT {to} HTTP/1.1\r\nHost: {to}\r\n");
    if let Some(auth) = proxy_auth(up) {
        ask.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    ask.push_str("\r\n");
    proxy.write_all(ask.as_bytes()).await.map_err(|e| e.to_string())?;
    let mut answer = Vec::new();
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(at) = answer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if answer.len() > HEAD_LIMIT {
            return Err("the proxy's answer was too long".into());
        }
        let n = proxy.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("the proxy closed the connection".into());
        }
        answer.extend_from_slice(&chunk[..n]);
    };
    let status = String::from_utf8_lossy(&answer[..end]).lines().next().unwrap_or_default().to_string();
    match status.split_whitespace().nth(1) {
        Some("200") => Ok(answer[end + 4..].to_vec()),
        Some("407") => Err("the proxy did not accept its user name and password".into()),
        _ => Err(format!("the proxy answered \"{}\"", status.trim())),
    }
}

/// A plain request as an HTTP proxy expects it: the whole address on the
/// first line, kept, the gateway's own sign-in swapped for the proxy's.
fn for_proxy(head: &[u8], up: &Upstream) -> Vec<u8> {
    let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") else {
        return head.to_vec();
    };
    let text = String::from_utf8_lossy(&head[..end]);
    let mut lines = text.split("\r\n");
    let mut out = format!("{}\r\n", lines.next().unwrap_or_default());
    for line in lines {
        let name = line.split(':').next().unwrap_or_default().trim().to_ascii_lowercase();
        if !matches!(name.as_str(), "proxy-authorization" | "proxy-connection" | "connection") && !line.is_empty() {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    if let Some(auth) = proxy_auth(up) {
        out.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");
    let mut out = out.into_bytes();
    out.extend_from_slice(&head[end + 4..]);
    out
}

/// The TLS settings for connections the app makes itself (an HTTPS proxy,
/// FTPS): the usual public roots, checked as browsers check them.
pub(crate) fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("the default TLS versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(config)
    })
    .clone()
}

async fn tls(host: &str, socket: TcpStream) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| format!("{host} is not a name TLS can check"))?;
    tokio_rustls::TlsConnector::from(tls_config())
        .connect(name, socket)
        .await
        .map_err(|e| format!("could not open a secure connection to the proxy: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{ProxyProfile, Upstream};

    fn upstream(kind: ProxyKind, user: &str, pass: Option<&str>) -> Upstream {
        let profile = ProxyProfile {
            id: 1,
            name: "Test".into(),
            kind,
            host: "proxy.test".into(),
            port: 3128,
            username: user.into(),
            has_password: pass.is_some(),
            bypass: vec![],
        };
        Upstream::new(&profile, pass.map(String::from))
    }

    #[test]
    fn reads_headers_by_name() {
        let head = "CONNECT a.test:443 HTTP/1.1\r\nHost: a.test:443\r\nproxy-AUTHORIZATION:  Basic abc \r\n\r\n";
        assert_eq!(header(head, "Proxy-Authorization"), Some("Basic abc"));
        assert_eq!(header(head, "cookie"), None);
        assert!(same("Basic abc", "Basic abc"));
        assert!(!same("Basic abc", "Basic abd"));
        assert!(!same("Basic abc", "Basic ab"));
    }

    #[test]
    fn a_plain_request_signs_in_to_the_proxy_not_the_gateway() {
        let sent = b"GET http://site.test/a HTTP/1.1\r\nHost: site.test\r\nProxy-Authorization: Basic gatewaykey\r\nProxy-Connection: keep-alive\r\n\r\nbody";
        let up = upstream(ProxyKind::Http, "me", Some("pw"));
        let out = String::from_utf8(for_proxy(sent, &up)).unwrap();
        assert!(out.starts_with("GET http://site.test/a HTTP/1.1\r\nHost: site.test\r\n"));
        assert!(!out.contains("gatewaykey"));
        assert!(out.contains("Proxy-Authorization: Basic bWU6cHc=\r\n"));
        assert!(out.ends_with("Connection: close\r\n\r\nbody"));
        let open = upstream(ProxyKind::Http, "", None);
        assert!(!String::from_utf8(for_proxy(sent, &open)).unwrap().contains("Proxy-Authorization"));
    }

    #[test]
    fn keys_are_long_and_never_repeat() {
        let (a, b) = (random_key(), random_key());
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
    }
}
