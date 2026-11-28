//! Files on FTP, FTPS and SFTP servers.
//!
//! - `ftp://` plain FTP (port 21);
//! - `ftpes://` FTP made secure with TLS after connecting ("explicit", 21);
//! - `ftps://` FTP over TLS from the start ("implicit", 990);
//! - `sftp://` the SSH file transfer protocol (22), which is not FTP at all.
//!
//! Each download is one connection reading the file in order; a paused one
//! carries on from what is on disk (FTP's REST, a seek in SFTP). A sign-in
//! written into the link is taken out of it when the download is added: the
//! user name stays with the download, the password goes to the vault.
//!
//! An SFTP server proves who it is with its key. A key the user trusted in
//! the app, or that their own OpenSSH `known_hosts` lists, is accepted. Any
//! other is refused, and the download shows its fingerprint so the user can
//! choose to trust it; a key that changed is refused with a warning.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::task::{Fail, Live};
use crate::{Inner, Record, Status};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Proto {
    Ftp,
    FtpsExplicit,
    FtpsImplicit,
    Sftp,
}

impl Proto {
    fn default_port(self) -> u16 {
        match self {
            Proto::Ftp | Proto::FtpsExplicit => 21,
            Proto::FtpsImplicit => 990,
            Proto::Sftp => 22,
        }
    }
}

/// The protocol of a link, when it is one of these.
pub(crate) fn proto_of(url: &str) -> Option<Proto> {
    let scheme = url.split_once("://")?.0.to_ascii_lowercase();
    Some(match scheme.as_str() {
        "ftp" => Proto::Ftp,
        "ftpes" => Proto::FtpsExplicit,
        "ftps" => Proto::FtpsImplicit,
        "sftp" => Proto::Sftp,
        _ => return None,
    })
}

/// How a download signs in to an FTP or SFTP server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteOptions {
    /// None for FTP: anonymous.
    #[serde(default)]
    pub username: String,
    /// Coming in only: the password, or a private key's passphrase.
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// A password (or passphrase) is kept in the vault.
    #[serde(default)]
    pub has_password: bool,
    /// SFTP: sign in with this private key file instead of a password.
    #[serde(default)]
    pub key_file: Option<PathBuf>,
    /// FTP: the server connects back for data (active mode) instead of
    /// the app connecting to it (passive, which works behind most routers).
    #[serde(default)]
    pub active_mode: bool,
    /// Seconds to wait for the server to answer; none = 20.
    #[serde(default)]
    pub connect_timeout: Option<u32>,
}

impl RemoteOptions {
    pub fn checked(mut self) -> Result<RemoteOptions, String> {
        self.username = self.username.trim().to_string();
        if self.username.chars().any(|c| c.is_control()) || self.username.len() > 255 {
            return Err("That user name cannot be used".into());
        }
        if self.password.as_deref().is_some_and(|p| p.contains(['\r', '\n', '\0'])) {
            return Err("The password cannot run over more than one line".into());
        }
        if let Some(key) = &self.key_file {
            if !key.is_file() {
                return Err(format!("The key file {} was not found", key.display()));
            }
        }
        if let Some(t) = self.connect_timeout {
            if t == 0 || t > 300 {
                return Err("The time to wait for the server must be 1 to 300 seconds".into());
            }
        }
        Ok(self)
    }
}

/// A server key that was refused, for the user to look at and trust.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerKey {
    pub host: String,
    pub port: u16,
    /// "SHA256:…", as OpenSSH shows it.
    pub fingerprint: String,
    /// The key itself, in OpenSSH's one-line form.
    pub key: String,
    /// It differs from the key trusted before for this server.
    pub changed: bool,
}

/// Takes a sign-in out of a link: the link without it, the user name and
/// the password.
pub(crate) fn split_credentials(url: &str) -> Result<(String, Option<String>, Option<String>), String> {
    let mut parsed = reqwest::Url::parse(url).map_err(|_| "That is not a valid link".to_string())?;
    if parsed.host_str().is_none_or(|h| h.is_empty()) {
        return Err("The link has no server in it".into());
    }
    if parsed.path().trim_matches('/').is_empty() {
        return Err("The link names a server but no file on it".into());
    }
    let decode = |s: &str| percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned();
    let user = Some(decode(parsed.username())).filter(|u| !u.is_empty());
    let password = parsed.password().map(decode).filter(|p| !p.is_empty());
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    Ok((parsed.to_string(), user, password))
}

/// The server keys the user trusted in the app, by "host:port".
fn trusted_file(state_dir: &Path) -> PathBuf {
    state_dir.join("sftp-trusted-keys.json")
}

fn trusted(state_dir: &Path) -> std::collections::HashMap<String, String> {
    std::fs::read(trusted_file(state_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub(crate) fn trust(state_dir: &Path, key: &ServerKey) -> Result<(), String> {
    let mut all = trusted(state_dir);
    all.insert(format!("{}:{}", key.host, key.port), key.key.clone());
    let bytes = serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::write(trusted_file(state_dir), bytes).map_err(|e| format!("could not save the server's key: {e}"))?;
    tracing::info!(host = %key.host, port = key.port, fingerprint = %key.fingerprint, "SFTP server key trusted");
    Ok(())
}

/// Copies what a server sends into the file from `offset` on, through the
/// download's speed limit; returns where it ended.
async fn copy<R: AsyncRead + Unpin>(reader: &mut R, file: &mut tokio::fs::File, mut offset: u64, live: &Arc<Live>) -> Result<u64, Fail> {
    let mut buf = vec![0u8; 64 * 1024];
    let _conn = crate::task::ConnGuard::new(&live.conns);
    loop {
        let n = tokio::select! {
            _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
            r = tokio::time::timeout(Duration::from_secs(60), reader.read(&mut buf)) => match r {
                Err(_) => return Err(Fail::Retry("the server stopped sending".into())),
                Ok(Err(e)) => return Err(Fail::Retry(format!("the connection broke off: {e}"))),
                Ok(Ok(n)) => n,
            },
        };
        if n == 0 {
            return Ok(offset);
        }
        live.limiter.acquire(n).await;
        file.write_all(&buf[..n]).await.map_err(|e| Fail::Fatal(format!("could not write to disk: {e}")))?;
        offset += n as u64;
        live.downloaded.store(offset, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Runs a download from an FTP, FTPS or SFTP server.
pub(crate) async fn run(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record) -> Result<(), Fail> {
    let proto = proto_of(&rec.url).ok_or_else(|| Fail::Fatal("not an FTP or SFTP link".into()))?;
    // The app's proxies carry web requests only: rather than connect some
    // other way than chosen, the download stops and says so.
    if inner.upstream(&rec.proxy).map_err(Fail::Fatal)?.is_some() {
        return Err(Fail::Fatal(
            "FTP and SFTP downloads cannot go through a proxy yet. Choose a direct connection for this download".into(),
        ));
    }
    let options = rec.remote.clone().unwrap_or_default();
    let password = if options.has_password {
        inner
            .vault
            .get(&format!("remote:{id}"))
            .map_err(Fail::Fatal)?
            .ok_or_else(|| Fail::Fatal("The saved password is missing from the vault; add the download again".into()))?
    } else {
        String::new()
    };
    let mut attempt = 0u32;
    loop {
        match once(inner, id, live, rec, proto, &options, &password).await {
            Err(Fail::Retry(why)) if attempt < 3 => {
                attempt += 1;
                tracing::info!(id, attempt, "remote download will try again: {why}");
                live.connect_failures.store(attempt as u64, std::sync::atomic::Ordering::Relaxed);
                tokio::select! {
                    _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
                    _ = tokio::time::sleep(Duration::from_secs(1 << attempt)) => {}
                }
            }
            other => return other,
        }
    }
}

async fn once(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record, proto: Proto, options: &RemoteOptions, password: &str) -> Result<(), Fail> {
    let rec = inner.record(id).unwrap_or_else(|| rec.clone());
    let url = reqwest::Url::parse(&rec.url).map_err(|_| Fail::Fatal("That is not a valid link".into()))?;
    let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']).to_string();
    let port = url.port().unwrap_or(proto.default_port());
    let path = percent_encoding::percent_decode_str(url.path()).decode_utf8_lossy().into_owned();
    let timeout = Duration::from_secs(options.connect_timeout.unwrap_or(20).into());

    std::fs::create_dir_all(&rec.dir).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    let part = crate::probe::part_path(&rec.dir, &rec.filename);
    let (filename, offset) = if rec.part_started && part.is_file() {
        (rec.filename.clone(), std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0))
    } else {
        let (name, _) = crate::task::create_part(&rec.dir, &rec.filename)?;
        (name, 0)
    };
    inner.update(id, |r| {
        r.filename = filename.clone();
        r.part_started = true;
        r.resumable = true;
    });
    let part = crate::probe::part_path(&rec.dir, &filename);
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&part)
        .await
        .map_err(|e| Fail::Fatal(format!("could not open the partial file: {e}")))?;

    let ended = match proto {
        Proto::Sftp => sftp(inner, id, live, &host, port, &path, options, password, timeout, &mut file, offset).await?,
        _ => ftp(inner, id, live, proto, &host, port, &path, options, password, timeout, &mut file, offset).await?,
    };
    file.flush().await.map_err(|e| Fail::Fatal(format!("could not write to disk: {e}")))?;
    file.sync_all().await.map_err(|e| Fail::Fatal(format!("could not write to disk: {e}")))?;
    drop(file);
    inner.update(id, |r| r.total = Some(ended));
    let final_name = crate::task::finalize(&rec.dir, &filename)?;
    inner.update(id, |r| r.filename = final_name.clone());
    Ok(())
}

/// Puts the file back to `offset` bytes long, for a start (or a start over) there.
async fn start_at(file: &mut tokio::fs::File, offset: u64) -> Result<(), Fail> {
    file.set_len(offset).await.map_err(|e| Fail::Fatal(format!("could not prepare the file: {e}")))?;
    file.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| Fail::Fatal(format!("could not prepare the file: {e}")))?;
    Ok(())
}

fn ftp_fail(e: suppaftp::FtpError) -> Fail {
    use suppaftp::{FtpError, Status};
    match e {
        FtpError::ConnectionError(io) => Fail::Retry(format!("could not reach the server: {io}")),
        FtpError::UnexpectedResponse(resp) => {
            let said = String::from_utf8_lossy(&resp.body).trim().to_string();
            match resp.status {
                Status::NotLoggedIn => Fail::Fatal(format!("The server did not accept the user name and password ({said})")),
                Status::FileUnavailable => Fail::Fatal(format!("The server has no such file, or does not allow reading it ({said})")),
                Status::NotAvailable | Status::ActionAborted | Status::RequestFileActionIgnored => Fail::Retry(format!("the server is busy: {said}")),
                _ => Fail::Fatal(format!("The server answered: {said}")),
            }
        }
        FtpError::SecureError(why) => Fail::Fatal(format!("A secure connection could not be made: {why}")),
        other => Fail::Retry(other.to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn ftp(
    inner: &Arc<Inner>,
    id: u64,
    live: &Arc<Live>,
    proto: Proto,
    host: &str,
    port: u16,
    path: &str,
    options: &RemoteOptions,
    password: &str,
    timeout: Duration,
    file: &mut tokio::fs::File,
    mut offset: u64,
) -> Result<u64, Fail> {
    use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
    use suppaftp::types::FileType;
    let connector = || AsyncRustlsConnector::from(tokio_rustls::TlsConnector::from(crate::gateway::tls_config()));
    let addr = (host.to_string(), port);
    let connecting = async {
        let ftp = match proto {
            Proto::FtpsImplicit => AsyncRustlsFtpStream::connect_secure_implicit(addr, connector(), host).await?,
            _ => {
                let plain = AsyncRustlsFtpStream::connect(addr).await?;
                if proto == Proto::FtpsExplicit {
                    plain.into_secure(connector(), host).await?
                } else {
                    plain
                }
            }
        };
        Ok::<_, suppaftp::FtpError>(ftp)
    };
    let mut ftp = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        r = tokio::time::timeout(timeout, connecting) => r.map_err(|_| Fail::Retry("the server did not answer in time".into()))?.map_err(ftp_fail)?,
    };
    let (user, pass) = if options.username.is_empty() { ("anonymous", "anonymous@") } else { (options.username.as_str(), password) };
    ftp.login(user, pass).await.map_err(ftp_fail)?;
    ftp.transfer_type(FileType::Binary).await.map_err(ftp_fail)?;
    if options.active_mode {
        ftp = ftp.active_mode(timeout);
    }
    let total = ftp.size(path).await.ok().map(|s| s as u64);
    if let Some(total) = total {
        inner.update(id, |r| {
            r.total = Some(total);
            r.status = Status::Downloading;
        });
    }
    if offset > 0 && total.is_some_and(|t| offset > t) {
        offset = 0;
    }
    if offset > 0 && ftp.resume_transfer(offset as usize).await.is_err() {
        tracing::info!(id, "the FTP server cannot carry on from where it was; starting over");
        offset = 0;
    }
    start_at(file, offset).await?;
    live.downloaded.store(offset, std::sync::atomic::Ordering::Relaxed);
    inner.update(id, |r| r.status = Status::Downloading);
    let mut stream = ftp.retr_as_stream(path).await.map_err(ftp_fail)?;
    let ended = copy(&mut stream, file, offset, live).await?;
    stream.finish().await.map_err(ftp_fail)?;
    let _ = ftp.quit().await;
    if let Some(total) = total {
        if ended != total {
            return Err(Fail::Retry(format!("the server sent {ended} of {total} bytes")));
        }
    }
    Ok(ended)
}

/// Checks an SFTP server's key; a refused one is kept to show the user.
struct KeyCheck {
    host: String,
    port: u16,
    state_dir: PathBuf,
    refused: Arc<Mutex<Option<ServerKey>>>,
}

impl russh::client::Handler for KeyCheck {
    type Error = russh::Error;

    async fn check_server_key(&mut self, offered: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = offered else {
            // Certificates are not checked here, so not taken.
            return Ok(false);
        };
        let line = key.to_openssh().unwrap_or_default();
        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
        let refuse = |changed: bool| ServerKey { host: self.host.clone(), port: self.port, fingerprint: fingerprint.clone(), key: line.clone(), changed };
        // Trusted in the app: that key, and only that key.
        if let Some(saved) = trusted(&self.state_dir).get(&format!("{}:{}", self.host, self.port)) {
            let same = russh::keys::PublicKey::from_openssh(saved).is_ok_and(|k| k == *key);
            if !same {
                *self.refused.lock().unwrap() = Some(refuse(true));
            }
            return Ok(same);
        }
        // Listed in the user's own OpenSSH known_hosts.
        match russh::keys::check_known_hosts(&self.host, self.port, key) {
            Ok(true) => Ok(true),
            Err(russh::keys::Error::KeyChanged { .. }) => {
                *self.refused.lock().unwrap() = Some(refuse(true));
                Ok(false)
            }
            _ => {
                *self.refused.lock().unwrap() = Some(refuse(false));
                Ok(false)
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn sftp(
    inner: &Arc<Inner>,
    id: u64,
    live: &Arc<Live>,
    host: &str,
    port: u16,
    path: &str,
    options: &RemoteOptions,
    password: &str,
    timeout: Duration,
    file: &mut tokio::fs::File,
    mut offset: u64,
) -> Result<u64, Fail> {
    if options.username.is_empty() {
        return Err(Fail::Fatal("Enter the user name to sign in to the SFTP server with".into()));
    }
    let refused = Arc::new(Mutex::new(None));
    let state_dir = inner.state_file.parent().map(Path::to_path_buf).unwrap_or_default();
    let check = KeyCheck { host: host.to_string(), port, state_dir, refused: refused.clone() };
    let config = Arc::new(russh::client::Config { inactivity_timeout: Some(Duration::from_secs(120)), ..Default::default() });
    let connecting = russh::client::connect(config, (host.to_string(), port), check);
    let connected = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        r = tokio::time::timeout(timeout, connecting) => r.map_err(|_| Fail::Retry("the server did not answer in time".into()))?,
    };
    let refused_key = refused.lock().unwrap().clone();
    if let Some(key) = refused_key {
        let why = if key.changed {
            format!(
                "The server's key has changed since it was trusted ({}). This can mean someone is listening in. Trust the new key only if you know why it changed",
                key.fingerprint
            )
        } else {
            format!("This server's key is not known yet ({}). Check it with the server's owner, then trust it in the download's details", key.fingerprint)
        };
        inner.update(id, |r| r.server_key = Some(key));
        return Err(Fail::Fatal(why));
    }
    let mut handle = connected.map_err(|e| Fail::Retry(format!("could not reach the server: {e}")))?;
    inner.update(id, |r| r.server_key = None);

    let signed_in = match &options.key_file {
        Some(key_file) => {
            let passphrase = (!password.is_empty()).then_some(password);
            let key = russh::keys::load_secret_key(key_file, passphrase)
                .map_err(|e| Fail::Fatal(format!("The key file could not be read (a passphrase may be needed): {e}")))?;
            let hash = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
            handle
                .authenticate_publickey(options.username.clone(), russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
        }
        None => handle.authenticate_password(options.username.clone(), password.to_string()).await,
    }
    .map_err(|e| Fail::Retry(format!("the sign-in broke off: {e}")))?;
    if !signed_in.success() {
        return Err(Fail::Fatal("The server did not accept the user name and password (or key)".into()));
    }
    let sftp_fail = |e: russh_sftp::client::error::Error| {
        let text = e.to_string();
        if text.to_ascii_lowercase().contains("no such file") || text.contains("NoSuchFile") {
            Fail::Fatal(format!("The server has no such file: {path}"))
        } else if text.contains("PermissionDenied") || text.to_ascii_lowercase().contains("permission") {
            Fail::Fatal(format!("The server does not allow reading {path}"))
        } else {
            Fail::Retry(format!("the server's file system answered: {text}"))
        }
    };
    let channel = handle.channel_open_session().await.map_err(|e| Fail::Retry(format!("could not open a session: {e}")))?;
    channel.request_subsystem(true, "sftp").await.map_err(|e| Fail::Fatal(format!("The server offers no SFTP: {e}")))?;
    let session = russh_sftp::client::SftpSession::new(channel.into_stream()).await.map_err(sftp_fail)?;
    let total = session.metadata(path).await.map_err(sftp_fail)?.size;
    if let Some(total) = total {
        inner.update(id, |r| r.total = Some(total));
    }
    if total.is_some_and(|t| offset > t) {
        offset = 0;
    }
    let mut remote = session.open(path).await.map_err(sftp_fail)?;
    if offset > 0 {
        remote.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| Fail::Retry(format!("could not carry on where it was: {e}")))?;
    }
    start_at(file, offset).await?;
    live.downloaded.store(offset, std::sync::atomic::Ordering::Relaxed);
    inner.update(id, |r| r.status = Status::Downloading);
    let ended = copy(&mut remote, file, offset, live).await?;
    let _ = session.close().await;
    if let Some(total) = total {
        if ended != total {
            return Err(Fail::Retry(format!("the server sent {ended} of {total} bytes")));
        }
    }
    Ok(ended)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_say_their_protocol() {
        assert_eq!(proto_of("ftp://h/f"), Some(Proto::Ftp));
        assert_eq!(proto_of("FTPES://h/f"), Some(Proto::FtpsExplicit));
        assert_eq!(proto_of("ftps://h/f"), Some(Proto::FtpsImplicit));
        assert_eq!(proto_of("sftp://h/f"), Some(Proto::Sftp));
        assert_eq!(proto_of("https://h/f"), None);
        assert_eq!(Proto::FtpsImplicit.default_port(), 990);
        assert_eq!(Proto::Sftp.default_port(), 22);
    }

    #[test]
    fn a_sign_in_is_taken_out_of_the_link() {
        let (url, user, pass) = split_credentials("ftp://me%40work:p%3Ass@files.test:2121/pub/a%20b.zip").unwrap();
        assert_eq!(url, "ftp://files.test:2121/pub/a%20b.zip");
        assert_eq!((user.as_deref(), pass.as_deref()), (Some("me@work"), Some("p:ss")));
        let (url, user, pass) = split_credentials("sftp://files.test/home/me/x.iso").unwrap();
        assert_eq!(url, "sftp://files.test/home/me/x.iso");
        assert_eq!((user, pass), (None, None));
        assert!(split_credentials("ftp://files.test/").unwrap_err().contains("no file"));
        assert!(split_credentials("ftp:///x").is_err());
    }

    #[test]
    fn options_are_checked() {
        assert!(RemoteOptions { username: "a\nb".into(), ..Default::default() }.checked().is_err());
        assert!(RemoteOptions { key_file: Some("Z:/no/such/key".into()), ..Default::default() }.checked().is_err());
        assert!(RemoteOptions { connect_timeout: Some(0), ..Default::default() }.checked().is_err());
        assert_eq!(RemoteOptions { username: " me ".into(), ..Default::default() }.checked().unwrap().username, "me");
    }
}
