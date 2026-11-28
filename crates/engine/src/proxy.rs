//! Proxy servers. One model is shared by every way the app goes online:
//!
//! - the engine's own HTTP client (direct files, file sets, web seeds, the
//!   app's own look-ups of a link) takes a [`reqwest::Proxy`] built here;
//! - yt-dlp and gallery-dl are pointed at a gateway on this computer
//!   (gateway.rs) that passes their connections on to the proxy, so the
//!   proxy's password never appears on a command line;
//! - torrents use the proxy for peers and HTTP trackers when it is SOCKS5,
//!   the only kind the torrent library can use.
//!
//! The list of proxies, and which one is the default, is kept with the
//! downloads (downloads.json); their passwords are kept in the system's
//! credential store (secrets.rs).

use std::net::IpAddr;
use std::sync::Arc;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};

/// How the app talks to a proxy server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    Http,
    /// HTTP proxy reached over TLS.
    Https,
    /// Site names are looked up on this computer.
    Socks4,
    /// Site names are looked up by the proxy.
    Socks4a,
    /// Site names are looked up on this computer.
    Socks5,
    /// Site names are looked up by the proxy.
    Socks5h,
}

impl ProxyKind {
    pub fn scheme(self) -> &'static str {
        match self {
            ProxyKind::Http => "http",
            ProxyKind::Https => "https",
            ProxyKind::Socks4 => "socks4",
            ProxyKind::Socks4a => "socks4a",
            ProxyKind::Socks5 => "socks5",
            ProxyKind::Socks5h => "socks5h",
        }
    }

    fn is_socks4(self) -> bool {
        matches!(self, ProxyKind::Socks4 | ProxyKind::Socks4a)
    }

    /// The proxy, not this computer, looks site names up.
    pub(crate) fn remote_dns(self) -> bool {
        matches!(self, ProxyKind::Http | ProxyKind::Https | ProxyKind::Socks4a | ProxyKind::Socks5h)
    }
}

/// One saved proxy server. Its password is not part of it: whether it has
/// one is (`has_password`), and the password itself is in the system's
/// credential store under [`ProxyProfile::secret_key`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyProfile {
    pub id: u64,
    pub name: String,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub has_password: bool,
    /// Sites reached directly rather than through the proxy (see [`Bypass`]).
    #[serde(default)]
    pub bypass: Vec<String>,
}

impl ProxyProfile {
    pub(crate) fn secret_key(id: u64) -> String {
        format!("proxy:{id}")
    }

    /// Checks what the user typed and tidies it (spaces, brackets around an
    /// IPv6 address, empty bypass lines).
    pub fn checked(mut self) -> Result<ProxyProfile, String> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            return Err("Give the proxy a name".into());
        }
        if self.name.chars().count() > 60 {
            return Err("Keep the proxy's name to 60 characters".into());
        }
        let host = self.host.trim().trim_start_matches('[').trim_end_matches(']').to_string();
        if host.is_empty() {
            return Err("Enter the proxy's address".into());
        }
        if host.len() > 253
            || host.contains("://")
            || host.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '@' | '?' | '#'))
        {
            return Err("Enter only the proxy's host name or IP address, without http:// or a port".into());
        }
        if host.contains(':') && host.parse::<IpAddr>().is_err() {
            return Err("Enter the port in its own box, not after the address".into());
        }
        self.host = host;
        if self.port == 0 {
            return Err("Enter the proxy's port (1 to 65535)".into());
        }
        self.username = self.username.trim().to_string();
        if self.username.len() > 255 || self.username.chars().any(char::is_control) {
            return Err("That user name cannot be used".into());
        }
        if self.kind.is_socks4() && self.has_password {
            return Err("SOCKS4 proxies have no passwords; use SOCKS5 for a proxy that asks for one".into());
        }
        if self.has_password && self.username.is_empty() {
            return Err("Enter the user name that goes with the password".into());
        }
        let mut bypass = Vec::new();
        for line in &self.bypass {
            let rule = line.trim().to_ascii_lowercase();
            if rule.is_empty() {
                continue;
            }
            Rule::parse(&rule).ok_or_else(|| format!("\"{rule}\" is not a site, address or address range"))?;
            if !bypass.contains(&rule) {
                bypass.push(rule);
            }
        }
        self.bypass = bypass;
        Ok(self)
    }
}

/// Which connection a download uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyChoice {
    /// The default chosen in Settings (a proxy, or a direct connection).
    #[default]
    Default,
    /// No proxy, whatever the default is.
    Direct,
    /// This saved proxy.
    Profile(u64),
}

/// A change to a proxy's saved password.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordChange {
    #[default]
    Keep,
    Clear,
    Set(String),
}

/// The saved proxies and the default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyBook {
    #[serde(default)]
    pub next_id: u64,
    /// Used by downloads set to the default connection; `None` = direct.
    #[serde(default)]
    pub default: Option<u64>,
    #[serde(default)]
    pub items: Vec<ProxyProfile>,
}

impl ProxyBook {
    pub(crate) fn get(&self, id: u64) -> Option<&ProxyProfile> {
        self.items.iter().find(|p| p.id == id)
    }

    /// The proxy a choice comes to, `None` for a direct connection, or why
    /// it cannot be used.
    pub(crate) fn pick(&self, choice: &ProxyChoice) -> Result<Option<&ProxyProfile>, String> {
        let id = match choice {
            ProxyChoice::Direct => return Ok(None),
            ProxyChoice::Default => match self.default {
                Some(id) => id,
                None => return Ok(None),
            },
            ProxyChoice::Profile(id) => *id,
        };
        self.get(id)
            .map(Some)
            .ok_or_else(|| "The proxy this download uses has been deleted. Choose another connection for it".into())
    }

    /// Drops a default that names no saved proxy.
    pub(crate) fn normalized(mut self) -> Self {
        if self.default.is_some_and(|id| self.get(id).is_none()) {
            self.default = None;
        }
        let top = self.items.iter().map(|p| p.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(top + 1);
        self
    }
}

/// A proxy ready to be used: its settings and password.
#[derive(Clone)]
pub(crate) struct Upstream {
    pub name: String,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub bypass: Arc<Bypass>,
}

impl std::fmt::Debug for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // never the password
        write!(f, "{} ({}://{}:{})", self.name, self.kind.scheme(), self.host, self.port)
    }
}

impl Upstream {
    pub(crate) fn new(profile: &ProxyProfile, password: Option<String>) -> Upstream {
        Upstream {
            name: profile.name.clone(),
            kind: profile.kind,
            host: profile.host.clone(),
            port: profile.port,
            username: profile.username.clone(),
            password: password.filter(|_| profile.has_password),
            bypass: Arc::new(Bypass::new(&profile.bypass)),
        }
    }

    /// `host:port`, with brackets around an IPv6 address.
    pub(crate) fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// The proxy's address with its user name and password in it, the form
    /// HTTP clients take. Kept inside the app: never logged or shown.
    pub(crate) fn url(&self) -> String {
        let mut auth = String::new();
        if !self.username.is_empty() {
            auth.push_str(&utf8_percent_encode(&self.username, NON_ALPHANUMERIC).to_string());
            if let Some(password) = &self.password {
                auth.push(':');
                auth.push_str(&utf8_percent_encode(password, NON_ALPHANUMERIC).to_string());
            }
            auth.push('@');
        }
        format!("{}://{auth}{}", self.kind.scheme(), self.authority())
    }

    /// Everything that makes one proxy connection differ from another, to
    /// reuse clients and gateways made for the same one.
    pub(crate) fn fingerprint(&self) -> String {
        let secret = self.password.as_deref().unwrap_or_default();
        let digest = ring::digest::digest(&ring::digest::SHA256, secret.as_bytes());
        let tag: String = digest.as_ref()[..8].iter().map(|b| format!("{b:02x}")).collect();
        format!("{}|{}|{}|{}|{}|{}", self.kind.scheme(), self.authority(), self.username, tag, self.bypass.rules.len(), self.bypass.source)
    }

    /// The proxy for the engine's HTTP client: every request goes through
    /// it, except those to sites on the bypass list and to this computer.
    pub(crate) fn reqwest_proxy(&self) -> Result<reqwest::Proxy, String> {
        let url = reqwest::Url::parse(&self.url()).map_err(|_| format!("The proxy \"{}\" has an address that cannot be used", self.name))?;
        let bypass = self.bypass.clone();
        Ok(reqwest::Proxy::custom(move |target| match target.host_str() {
            Some(host) if bypass.matches(host) => None,
            _ => Some(url.clone()),
        }))
    }

    /// The address the torrent library takes (SOCKS5 only), or why this
    /// proxy cannot carry torrents.
    pub(crate) fn torrent_url(&self) -> Result<String, String> {
        match self.kind {
            ProxyKind::Socks5 | ProxyKind::Socks5h => {
                let mut url = self.url();
                // the library knows "socks5" only, and always lets the proxy
                // look peers' addresses up (they are addresses already)
                if self.kind == ProxyKind::Socks5h {
                    url = url.replacen("socks5h://", "socks5://", 1);
                }
                Ok(url)
            }
            _ => Err(format!(
                "Torrents can only go through a SOCKS5 proxy, and \"{}\" is {}. Choose a SOCKS5 proxy or a direct connection for this download",
                self.name,
                self.kind.scheme().to_ascii_uppercase()
            )),
        }
    }
}

/// Says, in words, why a request through a proxy failed.
pub(crate) fn error_text(e: &reqwest::Error) -> String {
    let mut chain = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        chain = format!("{chain}: {s}");
        src = s.source();
    }
    let lower = chain.to_ascii_lowercase();
    let says = |signs: &[&str]| signs.iter().any(|s| lower.contains(s));
    if says(&["407", "proxy authentication", "password authentication", "authorization required", "credentials not accepted"]) {
        "The proxy did not accept the user name and password".into()
    } else if says(&["connection refused", "actively refused", "10061"]) {
        "Nothing answered at the proxy's address and port".into()
    } else if says(&["dns error", "could not find", "no such host", "failed to lookup", "getaddrinfo"]) {
        "The proxy's address could not be found".into()
    } else if e.is_timeout() || says(&["timed out"]) {
        "The proxy did not answer in time".into()
    } else if says(&["certificate", "tls", "handshake"]) {
        format!("A secure connection could not be opened: {chain}")
    } else {
        format!("The proxy could not be used: {chain}")
    }
}

/// Sites reached without the proxy. Each line is one of:
///
/// - `example.com`: that site and every site under it (`www.example.com`);
/// - `*.example.com`: only the sites under it;
/// - `192.168.1.20` or `2001:db8::1`: that address;
/// - `10.0.0.0/8`: every address in that range;
/// - `<local>`: names without a dot, and `.local` names;
/// - `*`: everything.
///
/// This computer itself (`localhost`, 127.0.0.1, ::1) is always reached
/// directly.
#[derive(Debug, Default)]
pub(crate) struct Bypass {
    rules: Vec<Rule>,
    /// The lines it was made from, for the fingerprint.
    source: String,
}

#[derive(Debug, PartialEq)]
enum Rule {
    All,
    Local,
    /// The site and everything under it.
    Domain(String),
    /// Only what is under the site.
    Under(String),
    Addr(IpAddr),
    Range(IpAddr, u8),
}

impl Rule {
    fn parse(line: &str) -> Option<Rule> {
        let line = line.trim().trim_end_matches('.').to_ascii_lowercase();
        if line == "*" {
            return Some(Rule::All);
        }
        if line == "<local>" {
            return Some(Rule::Local);
        }
        if let Some((addr, bits)) = line.split_once('/') {
            let addr: IpAddr = addr.trim_matches(['[', ']']).parse().ok()?;
            let bits: u8 = bits.parse().ok()?;
            let max = if addr.is_ipv4() { 32 } else { 128 };
            return (bits <= max).then_some(Rule::Range(addr, bits));
        }
        if let Ok(addr) = line.trim_matches(['[', ']']).parse::<IpAddr>() {
            return Some(Rule::Addr(addr));
        }
        let (under, name) = match line.strip_prefix("*.") {
            Some(rest) => (true, rest),
            None => (false, line.trim_start_matches('.')),
        };
        let valid = !name.is_empty()
            && name.len() <= 253
            && name.split('.').all(|label| {
                !label.is_empty() && label.len() <= 63 && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            });
        valid.then(|| if under { Rule::Under(name.to_string()) } else { Rule::Domain(name.to_string()) })
    }
}

fn in_range(ip: IpAddr, net: IpAddr, bits: u8) -> bool {
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let mask = if bits == 0 { 0 } else { u32::MAX << (32 - u32::from(bits)) };
            u32::from(a) & mask == u32::from(b) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let mask = if bits == 0 { 0 } else { u128::MAX << (128 - u32::from(bits)) };
            u128::from(a) & mask == u128::from(b) & mask
        }
        _ => false,
    }
}

impl Bypass {
    pub(crate) fn new(lines: &[String]) -> Bypass {
        Bypass { rules: lines.iter().filter_map(|l| Rule::parse(l)).collect(), source: lines.join(",") }
    }

    /// Whether `host` (a name or an address) is reached directly.
    pub(crate) fn matches(&self, host: &str) -> bool {
        let host = host.trim_matches(['[', ']']).trim_end_matches('.').to_ascii_lowercase();
        let ip = host.parse::<IpAddr>().ok();
        if host == "localhost" || host.ends_with(".localhost") || ip.is_some_and(|ip| ip.is_loopback()) {
            return true;
        }
        self.rules.iter().any(|rule| match rule {
            Rule::All => true,
            Rule::Local => ip.is_none() && (!host.contains('.') || host.ends_with(".local")),
            Rule::Domain(d) => host == *d || host.ends_with(&format!(".{d}")),
            Rule::Under(d) => host.ends_with(&format!(".{d}")),
            Rule::Addr(a) => ip == Some(*a),
            Rule::Range(net, bits) => ip.is_some_and(|ip| in_range(ip, *net, *bits)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(kind: ProxyKind) -> ProxyProfile {
        ProxyProfile {
            id: 1,
            name: " Office ".into(),
            kind,
            host: " proxy.example.com ".into(),
            port: 8080,
            username: String::new(),
            has_password: false,
            bypass: vec![],
        }
    }

    #[test]
    fn what_the_user_typed_is_checked_and_tidied() {
        let p = profile(ProxyKind::Http).checked().unwrap();
        assert_eq!((p.name.as_str(), p.host.as_str()), ("Office", "proxy.example.com"));

        let mut v6 = profile(ProxyKind::Socks5);
        v6.host = "[2001:db8::1]".into();
        assert_eq!(v6.checked().unwrap().host, "2001:db8::1");

        for (host, why) in [
            ("", "address"),
            ("http://proxy.test", "without http://"),
            ("proxy.test:8080", "port in its own box"),
            ("user@proxy.test", "without http://"),
        ] {
            let mut p = profile(ProxyKind::Http);
            p.host = host.into();
            let err = p.checked().unwrap_err();
            assert!(err.contains(why), "{host}: {err}");
        }
        let mut no_port = profile(ProxyKind::Http);
        no_port.port = 0;
        assert!(no_port.checked().is_err());

        let mut socks4 = profile(ProxyKind::Socks4);
        socks4.username = "me".into();
        socks4.has_password = true;
        assert!(socks4.checked().unwrap_err().contains("SOCKS4"));

        let mut nameless = profile(ProxyKind::Socks5);
        nameless.has_password = true;
        assert!(nameless.checked().unwrap_err().contains("user name"));

        let mut rules = profile(ProxyKind::Http);
        rules.bypass = vec![" Example.com ".into(), "".into(), "example.com".into(), "10.0.0.0/8".into()];
        assert_eq!(rules.checked().unwrap().bypass, vec!["example.com", "10.0.0.0/8"]);
        let mut bad = profile(ProxyKind::Http);
        bad.bypass = vec!["not a site!".into()];
        assert!(bad.checked().is_err());
        let mut wide = profile(ProxyKind::Http);
        wide.bypass = vec!["10.0.0.0/33".into()];
        assert!(wide.checked().is_err());
    }

    #[test]
    fn the_address_carries_the_sign_in_safely() {
        let mut p = profile(ProxyKind::Socks5h).checked().unwrap();
        p.username = "me@home".into();
        p.has_password = true;
        let up = Upstream::new(&p, Some("p:ss/w@rd".into()));
        assert_eq!(up.url(), "socks5h://me%40home:p%3Ass%2Fw%40rd@proxy.example.com:8080");
        let parsed = reqwest::Url::parse(&up.url()).unwrap();
        assert_eq!(parsed.host_str(), Some("proxy.example.com"));
        assert!(!format!("{up:?}").contains("ss"), "the password is never printed");

        // a password saved for a profile that no longer has one is not used
        let mut open = p.clone();
        open.has_password = false;
        assert_eq!(Upstream::new(&open, Some("old".into())).url(), "socks5h://me%40home@proxy.example.com:8080");

        let mut v6 = profile(ProxyKind::Http);
        v6.host = "2001:db8::1".into();
        assert_eq!(Upstream::new(&v6, None).url(), "http://[2001:db8::1]:8080");
    }

    #[test]
    fn torrents_take_socks5_only() {
        let socks = Upstream::new(&profile(ProxyKind::Socks5h).checked().unwrap(), None);
        assert_eq!(socks.torrent_url().unwrap(), "socks5://proxy.example.com:8080");
        let http = Upstream::new(&profile(ProxyKind::Http), None);
        assert!(http.torrent_url().unwrap_err().contains("SOCKS5"));
    }

    #[test]
    fn a_choice_comes_to_a_proxy_or_a_direct_connection() {
        let mut book = ProxyBook { next_id: 0, default: Some(1), items: vec![profile(ProxyKind::Http)] };
        assert_eq!(book.pick(&ProxyChoice::Default).unwrap().map(|p| p.id), Some(1));
        assert!(book.pick(&ProxyChoice::Direct).unwrap().is_none());
        assert!(book.pick(&ProxyChoice::Profile(7)).unwrap_err().contains("deleted"));
        book.default = Some(9);
        let book = book.normalized();
        assert_eq!(book.default, None, "a default that names no proxy is dropped");
        assert_eq!(book.next_id, 2);
        assert!(book.pick(&ProxyChoice::Default).unwrap().is_none());
    }

    #[test]
    fn choices_are_saved_in_a_readable_form() {
        assert_eq!(serde_json::to_string(&ProxyChoice::Default).unwrap(), "\"default\"");
        assert_eq!(serde_json::to_string(&ProxyChoice::Profile(3)).unwrap(), "{\"profile\":3}");
        let back: ProxyChoice = serde_json::from_str("{\"profile\":3}").unwrap();
        assert_eq!(back, ProxyChoice::Profile(3));
        let change: PasswordChange = serde_json::from_str("{\"set\":\"x\"}").unwrap();
        assert!(matches!(change, PasswordChange::Set(s) if s == "x"));
        assert!(matches!(serde_json::from_str::<PasswordChange>("\"keep\"").unwrap(), PasswordChange::Keep));
    }

    #[test]
    fn bypassed_sites_go_direct() {
        let b = Bypass::new(&[
            "example.com".into(),
            "*.internal.test".into(),
            "192.168.1.20".into(),
            "10.0.0.0/8".into(),
            "fd00::/8".into(),
            "<local>".into(),
        ]);
        for host in ["example.com", "www.example.com", "EXAMPLE.COM.", "a.internal.test", "192.168.1.20", "10.9.8.7", "[fd12::1]", "nas", "printer.local", "localhost", "127.0.0.1", "::1"] {
            assert!(b.matches(host), "{host} goes direct");
        }
        for host in ["badexample.com", "internal.test", "192.168.1.21", "11.0.0.1", "2001:db8::1", "site.test"] {
            assert!(!b.matches(host), "{host} goes through the proxy");
        }
        assert!(Bypass::new(&["*".into()]).matches("anything.test"));
        assert!(!Bypass::default().matches("site.test"));
        assert!(Bypass::default().matches("localhost"), "this computer is always direct");
    }
}
