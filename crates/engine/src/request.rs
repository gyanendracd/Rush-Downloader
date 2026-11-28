//! How a download's requests are made, when the user sets it: a browser's
//! name (User-Agent), the page it came from (Referer), headers of their
//! own, cookies, a sign-in (Basic, Digest or a bearer token) and timeouts.
//! Set for one download, or kept as a named profile; one profile can be
//! the default for every download that has none of its own.
//!
//! Private values (passwords, tokens, cookies, and headers that carry a
//! sign-in) never go into downloads.json: they are moved to the encrypted
//! vault (vault.rs) when saved, and put back only for the request.
//!
//! None of these go to another site: when a server sends the download to a
//! different host, the request there carries only the browser's name and
//! the referring page (see `Inner::prepare_request`).

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::proxy::PasswordChange;

/// How a download signs in to the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    #[default]
    None,
    /// User name and password, sent with every request.
    Basic,
    /// User name and password, answered to the server's challenge.
    Digest,
    /// A token: "Authorization: Bearer <token>".
    Bearer,
}

/// What the user sets for a download's requests (or a profile's).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestOptions {
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub referer: Option<String>,
    /// Headers of the user's own, in order. A private one's value is kept
    /// in the vault and is empty here (its name is in `private_headers`).
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub auth: AuthKind,
    #[serde(default)]
    pub username: String,
    /// Seconds a connection may go without data before it is tried again;
    /// none = the app's own (20).
    #[serde(default)]
    pub read_timeout: Option<u32>,
    /// Seconds to wait for a server to answer at all; none = 15.
    #[serde(default)]
    pub connect_timeout: Option<u32>,
    /// Coming in only: the password (Basic, Digest) or the token (Bearer).
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// Coming in only: cookies, "name=value; other=value".
    #[serde(default, skip_serializing)]
    pub cookies: Option<String>,
    /// A password or token is kept in the vault.
    #[serde(default)]
    pub has_password: bool,
    /// Cookies are kept in the vault.
    #[serde(default)]
    pub has_cookies: bool,
    /// Names of headers whose values are kept in the vault.
    #[serde(default)]
    pub private_headers: Vec<String>,
}

/// The private part, as kept in the vault.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Private {
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

/// A saved set of request options.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestProfile {
    pub id: u64,
    pub name: String,
    pub options: RequestOptions,
}

/// The saved profiles, and the default for downloads that have none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestBook {
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub default: Option<u64>,
    #[serde(default)]
    pub items: Vec<RequestProfile>,
}

impl RequestBook {
    pub(crate) fn get(&self, id: u64) -> Option<&RequestProfile> {
        self.items.iter().find(|p| p.id == id)
    }

    pub(crate) fn normalized(mut self) -> Self {
        if self.default.is_some_and(|id| self.get(id).is_none()) {
            self.default = None;
        }
        let top = self.items.iter().map(|p| p.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(top + 1);
        self
    }
}

/// Headers the engine sets itself, or that would break a download.
const RESERVED: [&str; 10] =
    ["host", "content-length", "range", "if-range", "transfer-encoding", "connection", "te", "upgrade", "proxy-connection", "expect"];

/// Whether a header carries a sign-in or a secret, and so is kept private.
pub(crate) fn is_private(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "authorization" | "proxy-authorization" | "cookie")
        || ["token", "secret", "key", "session", "auth", "password", "signature", "csrf"].iter().any(|w| lower.contains(w))
}

fn is_token(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

fn clean_value(value: &str, what: &str) -> Result<String, String> {
    let value = value.trim();
    if value.chars().any(|c| matches!(c, '\r' | '\n' | '\0')) {
        return Err(format!("{what} cannot run over more than one line"));
    }
    if value.len() > 16 * 1024 {
        return Err(format!("{what} is too long"));
    }
    Ok(value.to_string())
}

/// Where a request goes, by host, to tell when a redirect leaves the site.
pub(crate) fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok()?.host_str().map(|h| h.to_ascii_lowercase())
}

impl RequestOptions {
    /// Whether anything is set at all.
    pub(crate) fn is_set(&self) -> bool {
        self.user_agent.is_some()
            || self.referer.is_some()
            || !self.headers.is_empty()
            || self.auth != AuthKind::None
            || self.read_timeout.is_some()
            || self.connect_timeout.is_some()
            || self.cookies.is_some()
            || self.has_cookies
    }

    /// Checks what the user typed and tidies it. Private values may still
    /// be in it (they are split off by `split_private`).
    pub fn checked(mut self) -> Result<RequestOptions, String> {
        self.user_agent = match self.user_agent.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(ua) => Some(clean_value(ua, "The browser name")?),
            None => None,
        };
        self.referer = match self.referer.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(r) => {
                let url = reqwest::Url::parse(r).map_err(|_| "The referring page must be a web address".to_string())?;
                if !matches!(url.scheme(), "http" | "https") {
                    return Err("The referring page must be a web address (http or https)".into());
                }
                Some(r.to_string())
            }
            None => None,
        };
        if self.headers.len() > 64 {
            return Err("Keep to 64 headers".into());
        }
        let mut headers = Vec::new();
        for (name, value) in std::mem::take(&mut self.headers) {
            let name = name.trim().to_string();
            if name.is_empty() && value.trim().is_empty() {
                continue;
            }
            if !is_token(&name) {
                return Err(format!("\"{name}\" cannot be a header's name"));
            }
            if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(format!("{name} is set by the app itself and cannot be changed"));
            }
            headers.push((name.clone(), clean_value(&value, &format!("The header {name}"))?));
        }
        self.headers = headers;
        if let Some(c) = &self.cookies {
            let c = clean_value(c, "The cookies")?;
            self.cookies = (!c.is_empty()).then_some(c);
        }
        self.username = clean_value(&self.username, "The user name")?;
        if matches!(self.auth, AuthKind::Basic | AuthKind::Digest) && self.username.is_empty() {
            return Err("Enter the user name to sign in with".into());
        }
        if self.auth == AuthKind::Basic && self.username.contains(':') {
            return Err("A user name for Basic sign-in cannot contain \":\"".into());
        }
        if let Some(p) = &self.password {
            if p.chars().any(|c| matches!(c, '\r' | '\n' | '\0')) {
                return Err("The password or token cannot run over more than one line".into());
            }
        }
        for (t, what, max) in [(&mut self.read_timeout, "wait for data", 600u32), (&mut self.connect_timeout, "wait for an answer", 300)] {
            if let Some(v) = *t {
                if v == 0 || v > max {
                    return Err(format!("The time to {what} must be 1 to {max} seconds"));
                }
            }
        }
        Ok(self)
    }

    /// Moves the private values out, to be kept in the vault. What is left
    /// is safe to save in downloads.json and to show.
    pub(crate) fn split_private(&mut self) -> Private {
        let mut private = Private { password: self.password.take().filter(|p| !p.is_empty()), cookies: self.cookies.take(), headers: Vec::new() };
        self.has_password = private.password.is_some();
        self.has_cookies = private.cookies.is_some();
        self.private_headers.clear();
        for (name, value) in self.headers.iter_mut() {
            if is_private(name) {
                private.headers.push((name.clone(), std::mem::take(value)));
                self.private_headers.push(name.clone());
            }
        }
        private
    }

    /// Applies an edit of a saved profile's private values: `password` and
    /// `cookies` as asked; a private header given with no value keeps the
    /// value saved for it.
    pub(crate) fn merge_private(&mut self, old: &Private, password: PasswordChange, cookies: PasswordChange) {
        self.password = match password {
            PasswordChange::Set(p) => Some(p),
            PasswordChange::Clear => None,
            PasswordChange::Keep => old.password.clone(),
        };
        self.cookies = match cookies {
            PasswordChange::Set(c) => Some(c),
            PasswordChange::Clear => None,
            PasswordChange::Keep => old.cookies.clone(),
        };
        for (name, value) in self.headers.iter_mut() {
            if value.is_empty() {
                if let Some((_, saved)) = old.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                    *value = saved.clone();
                }
            }
        }
    }

    /// The headers to send, all private values filled in. A Digest sign-in
    /// is not among them: it answers the server's challenge (see
    /// `digest_header`).
    pub(crate) fn headers(&self, private: &Private) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(ua) = &self.user_agent {
            out.push(("User-Agent".to_string(), ua.clone()));
        }
        if let Some(r) = &self.referer {
            out.push(("Referer".to_string(), r.clone()));
        }
        if let Some(c) = &private.cookies {
            out.push(("Cookie".to_string(), c.clone()));
        }
        for (name, value) in &self.headers {
            let value = if self.private_headers.iter().any(|p| p.eq_ignore_ascii_case(name)) {
                private.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone()).unwrap_or_default()
            } else {
                value.clone()
            };
            out.push((name.clone(), value));
        }
        let secret = private.password.clone().unwrap_or_default();
        match self.auth {
            AuthKind::Basic => {
                let pair = format!("{}:{secret}", self.username);
                out.push(("Authorization".into(), format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(pair))));
            }
            AuthKind::Bearer => out.push(("Authorization".into(), format!("Bearer {secret}"))),
            AuthKind::None | AuthKind::Digest => {}
        }
        out
    }
}

/// Puts `extra` over `base`: a header set in both takes `extra`'s value.
pub(crate) fn overlay(base: &[(String, String)], extra: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> =
        base.iter().filter(|(n, _)| !extra.iter().any(|(e, _)| e.eq_ignore_ascii_case(n))).cloned().collect();
    out.extend(extra);
    out
}

/// What may follow a download to another site: the browser's name and the
/// page it came from. Cookies, sign-ins and the user's own headers stay.
pub(crate) fn safe_for_other_site(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("user-agent") || n.eq_ignore_ascii_case("referer")).cloned().collect()
}

/// The answer to a server's Digest challenge for a GET of `url`.
pub(crate) fn digest_header(challenge: &str, url: &str, username: &str, password: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    let mut uri = parsed.path().to_string();
    if let Some(q) = parsed.query() {
        uri.push('?');
        uri.push_str(q);
    }
    let mut prompt = digest_auth::parse(challenge).map_err(|e| format!("the server's sign-in challenge could not be read: {e}"))?;
    let context = digest_auth::AuthContext::new(username, password, uri.as_str());
    let answer = prompt.respond(&context).map_err(|e| format!("could not answer the server's sign-in challenge: {e}"))?;
    Ok(answer.to_string())
}

impl crate::Inner {
    fn private_of(&self, key: &str, wanted: bool) -> Result<Private, String> {
        if !wanted {
            return Ok(Private::default());
        }
        let text = self.vault.get(key)?.ok_or("Its saved sign-in, cookies or private headers are missing from the vault; set them again")?;
        serde_json::from_str(&text).map_err(|_| "Its saved sign-in could not be read; set it again".to_string())
    }

    /// The request options a download uses, private values filled in: its
    /// own, its profile's, or the default profile's; `None` when nothing is
    /// set.
    pub(crate) fn request_of(&self, rec: &crate::Record) -> Result<Option<(RequestOptions, Private)>, String> {
        let wants_private = |o: &RequestOptions| o.has_password || o.has_cookies || !o.private_headers.is_empty();
        if let Some(options) = &rec.request {
            let private = self.private_of(&format!("request:{}", rec.id), wants_private(options))?;
            return Ok(Some((options.clone(), private)));
        }
        let profile = {
            let st = self.state.lock().unwrap();
            let id = match rec.request_profile {
                Some(id) => Some(id),
                None => st.requests.default,
            };
            match id {
                Some(id) => Some(st.requests.get(id).cloned().ok_or("The request profile this download uses has been deleted")?),
                None => None,
            }
        };
        let Some(profile) = profile else { return Ok(None) };
        let private = self.private_of(&format!("request-profile:{}", profile.id), wants_private(&profile.options))?;
        Ok(Some((profile.options, private)))
    }

    /// The headers a helper program (yt-dlp) is given for a download: its
    /// own and its request options', without a Digest sign-in, which only
    /// the app's own requests can answer.
    pub(crate) fn request_headers(&self, rec: &crate::Record) -> Vec<(String, String)> {
        match self.request_of(rec) {
            Ok(Some((options, private))) => overlay(&rec.headers, options.headers(&private)),
            Ok(None) => rec.headers.clone(),
            Err(e) => {
                tracing::warn!(id = rec.id, "request options could not be used: {e}");
                rec.headers.clone()
            }
        }
    }

    /// The address and headers a download's requests go with. With request
    /// options set, redirects are followed here: on the same site the
    /// headers go along; to another site only the browser's name and the
    /// referring page do. A Digest sign-in answers the server's challenge.
    pub(crate) async fn prepare_request(
        &self,
        rec: &crate::Record,
        request: Option<&(RequestOptions, Private)>,
        client: &reqwest::Client,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<(String, Vec<(String, String)>), crate::task::Fail> {
        use crate::task::Fail;
        let Some((options, private)) = request else {
            return Ok((rec.url.clone(), rec.headers.clone()));
        };
        let mut headers = overlay(&rec.headers, options.headers(private));
        let mut url = rec.url.clone();
        let home = host_of(&rec.url);
        let mut answered = false;
        for _ in 0..12 {
            let asked = crate::probe::request(client, &url, &headers).header(reqwest::header::RANGE, "bytes=0-0").send();
            let resp = tokio::select! {
                _ = cancel.cancelled() => return Err(Fail::Cancelled),
                r = asked => r.map_err(crate::probe::net_err)?,
            };
            let status = resp.status();
            if status.is_redirection() {
                let Some(location) = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()) else {
                    return Ok((url, headers));
                };
                let next = reqwest::Url::parse(&url)
                    .and_then(|base| base.join(location))
                    .map_err(|_| Fail::Fatal("The server sent the download to an address that cannot be used".into()))?
                    .to_string();
                if host_of(&next) != host_of(&url) {
                    tracing::info!(id = rec.id, from = ?host_of(&url), to = ?host_of(&next), "redirect to another site: only harmless headers follow");
                    headers = safe_for_other_site(&headers);
                }
                url = next;
                continue;
            }
            if status == reqwest::StatusCode::UNAUTHORIZED && options.auth == AuthKind::Digest && !answered && host_of(&url) == home {
                let challenge = resp
                    .headers()
                    .get_all(reqwest::header::WWW_AUTHENTICATE)
                    .iter()
                    .filter_map(|v| v.to_str().ok())
                    .find(|v| v.trim_start().to_ascii_lowercase().starts_with("digest"))
                    .ok_or_else(|| Fail::Fatal("The server does not ask for a Digest sign-in; choose Basic instead".into()))?
                    .to_string();
                let answer = digest_header(&challenge, &url, &options.username, private.password.as_deref().unwrap_or_default())
                    .map_err(Fail::Fatal)?;
                headers.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization"));
                headers.push(("Authorization".into(), answer));
                answered = true;
                continue;
            }
            return Ok((url, headers));
        }
        Err(Fail::Fatal("The server sent the download round in circles (too many redirects)".into()))
    }
}

impl crate::Engine {
    /// The saved request profiles and the default.
    pub fn request_profiles(&self) -> RequestBook {
        self.inner.state.lock().unwrap().requests.clone()
    }

    /// Saves a new request profile (`id` 0) or changes one. Private values
    /// go to the vault: `password` (or token) and `cookies` as asked; a
    /// private header given with no value keeps the one saved.
    pub fn save_request_profile(
        &self,
        id: u64,
        name: &str,
        mut options: RequestOptions,
        password: PasswordChange,
        cookies: PasswordChange,
    ) -> Result<u64, String> {
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > 60 {
            return Err("Give the profile a name of up to 60 characters".into());
        }
        let old = if id == 0 {
            Private::default()
        } else {
            let existing = self.inner.state.lock().unwrap().requests.get(id).cloned().ok_or("That profile has been deleted")?;
            let o = &existing.options;
            self.inner.private_of(&format!("request-profile:{id}"), o.has_password || o.has_cookies || !o.private_headers.is_empty())?
        };
        options.merge_private(&old, password, cookies);
        let mut options = options.checked()?;
        if options.auth == AuthKind::Bearer && options.password.as_deref().unwrap_or_default().is_empty() {
            return Err("Enter the token to sign in with".into());
        }
        let private = options.split_private();
        let id = {
            let mut st = self.inner.state.lock().unwrap();
            if id == 0 {
                let id = st.requests.next_id.max(1);
                st.requests.next_id = id + 1;
                id
            } else {
                id
            }
        };
        let key = format!("request-profile:{id}");
        if private == Private::default() {
            self.inner.vault.delete(&key)?;
        } else {
            self.inner.vault.set(&key, &serde_json::to_string(&private).map_err(|e| e.to_string())?)?;
        }
        {
            let mut st = self.inner.state.lock().unwrap();
            let profile = RequestProfile { id, name, options };
            match st.requests.items.iter_mut().find(|p| p.id == id) {
                Some(slot) => *slot = profile,
                None => st.requests.items.push(profile),
            }
        }
        self.inner.changed();
        Ok(id)
    }

    /// Deletes a request profile. Refused while an unfinished download is
    /// set to use it.
    pub fn delete_request_profile(&self, id: u64) -> Result<(), String> {
        {
            let st = self.inner.state.lock().unwrap();
            let users = st
                .entries
                .iter()
                .filter(|e| e.rec.request_profile == Some(id) && e.rec.status != crate::Status::Completed)
                .count();
            if users > 0 {
                return Err(format!("{users} unfinished download(s) use this profile. Remove them or let them finish first"));
            }
        }
        self.inner.vault.delete(&format!("request-profile:{id}"))?;
        {
            let mut st = self.inner.state.lock().unwrap();
            st.requests.items.retain(|p| p.id != id);
            if st.requests.default == Some(id) {
                st.requests.default = None;
            }
        }
        self.inner.changed();
        Ok(())
    }

    /// Makes a profile the default for downloads with no options of their
    /// own, or `None` for none.
    pub fn set_default_request_profile(&self, id: Option<u64>) -> Result<(), String> {
        {
            let mut st = self.inner.state.lock().unwrap();
            if id.is_some_and(|id| st.requests.get(id).is_none()) {
                return Err("That profile has been deleted".into());
            }
            st.requests.default = id;
        }
        self.inner.changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> RequestOptions {
        RequestOptions {
            user_agent: Some(" Mozilla/5.0 Test ".into()),
            referer: Some("https://site.test/page".into()),
            headers: vec![("X-Plain".into(), "one".into()), ("X-Api-Key".into(), "k3y".into()), ("".into(), "".into())],
            auth: AuthKind::Basic,
            username: "me".into(),
            password: Some("pw".into()),
            cookies: Some("a=1; b=2".into()),
            ..Default::default()
        }
    }

    #[test]
    fn what_is_typed_is_checked() {
        let o = opts().checked().unwrap();
        assert_eq!(o.user_agent.as_deref(), Some("Mozilla/5.0 Test"));
        assert_eq!(o.headers.len(), 2, "the empty line is dropped");
        for bad in [
            RequestOptions { headers: vec![("Bad Name".into(), "x".into())], ..Default::default() },
            RequestOptions { headers: vec![("Range".into(), "bytes=0-".into())], ..Default::default() },
            RequestOptions { headers: vec![("X-Ok".into(), "a\r\nInjected: 1".into())], ..Default::default() },
            RequestOptions { referer: Some("javascript:alert(1)".into()), ..Default::default() },
            RequestOptions { auth: AuthKind::Digest, ..Default::default() },
            RequestOptions { auth: AuthKind::Basic, username: "a:b".into(), ..Default::default() },
            RequestOptions { read_timeout: Some(0), ..Default::default() },
            RequestOptions { connect_timeout: Some(301), ..Default::default() },
        ] {
            assert!(bad.clone().checked().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn private_values_leave_what_is_saved() {
        let mut o = opts().checked().unwrap();
        let private = o.split_private();
        let saved = serde_json::to_string(&o).unwrap();
        for secret in ["pw", "a=1", "k3y"] {
            assert!(!saved.contains(secret), "{secret} is not saved: {saved}");
        }
        assert!(o.has_password && o.has_cookies);
        assert_eq!(o.private_headers, vec!["X-Api-Key"]);
        assert_eq!(private.headers, vec![("X-Api-Key".to_string(), "k3y".to_string())]);

        // put back together, everything is sent
        let sent = o.headers(&private);
        let get = |n: &str| sent.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
        assert_eq!(get("User-Agent"), Some("Mozilla/5.0 Test"));
        assert_eq!(get("Cookie"), Some("a=1; b=2"));
        assert_eq!(get("X-Api-Key"), Some("k3y"));
        assert_eq!(get("X-Plain"), Some("one"));
        assert_eq!(get("Authorization"), Some("Basic bWU6cHc="));

        let bearer = RequestOptions { auth: AuthKind::Bearer, ..Default::default() };
        let sent = bearer.headers(&Private { password: Some("t0k".into()), ..Default::default() });
        assert_eq!(sent, vec![("Authorization".to_string(), "Bearer t0k".to_string())]);
    }

    #[test]
    fn editing_a_profile_keeps_what_is_not_retyped() {
        let old = Private { password: Some("old".into()), cookies: Some("c=1".into()), headers: vec![("X-Token".into(), "saved".into())] };
        let mut o = RequestOptions { headers: vec![("X-Token".into(), "".into()), ("X-New".into(), "v".into())], ..Default::default() };
        o.merge_private(&old, PasswordChange::Keep, PasswordChange::Clear);
        assert_eq!(o.password.as_deref(), Some("old"));
        assert_eq!(o.cookies, None);
        assert_eq!(o.headers[0].1, "saved");
        o.merge_private(&old, PasswordChange::Set("new".into()), PasswordChange::Keep);
        assert_eq!(o.password.as_deref(), Some("new"));
    }

    #[test]
    fn only_harmless_headers_follow_to_another_site() {
        let all = vec![
            ("User-Agent".to_string(), "UA".to_string()),
            ("Referer".to_string(), "https://a.test/".to_string()),
            ("Cookie".to_string(), "s=1".to_string()),
            ("Authorization".to_string(), "Basic x".to_string()),
            ("X-Api-Key".to_string(), "k".to_string()),
        ];
        let kept = safe_for_other_site(&all);
        assert_eq!(kept.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["User-Agent", "Referer"]);
        let merged = overlay(&[("User-Agent".into(), "old".into()), ("Referer".into(), "r".into())], vec![("user-agent".into(), "new".into())]);
        assert_eq!(merged, vec![("Referer".to_string(), "r".to_string()), ("user-agent".to_string(), "new".to_string())]);
        assert!(is_private("X-CSRF-Token") && is_private("cookie") && !is_private("Accept-Language"));
    }

    #[test]
    fn answers_a_digest_challenge() {
        let challenge = r#"Digest realm="files", qop="auth", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#;
        let answer = digest_header(challenge, "https://files.test/dir/a.zip?x=1", "me", "pw").unwrap();
        assert!(answer.starts_with("Digest "), "{answer}");
        assert!(answer.contains(r#"username="me""#) && answer.contains(r#"uri="/dir/a.zip?x=1""#) && answer.contains("response="));
        assert!(digest_header("Basic realm=x", "https://a.test/", "me", "pw").is_err());
    }
}
