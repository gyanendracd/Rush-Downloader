//! FTP and SFTP downloads, end to end, against small servers written for
//! the test: a sign-in in the link (kept out of the list), a wrong password,
//! pause and resume, a chosen proxy, and SFTP's server keys: unknown,
//! trusted, and changed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rushdownload_engine::{Engine, MemoryStore, NewDownload, PasswordChange, ProxyChoice, ProxyKind, ProxyProfile, RemoteOptions, Settings, Status, TaskInfo};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

fn body(len: usize) -> Arc<Vec<u8>> {
    Arc::new((0..len).map(|i| (i * 7 % 251) as u8).collect())
}

/* ---------- a small FTP server ---------- */

#[derive(Default)]
struct FtpLog {
    /// Offsets asked for with REST.
    rests: Vec<u64>,
    logins: Vec<(String, String)>,
}

/// Serves `data` as /pub/file.bin to user "me" with password "pw", passive
/// mode only. With `slow`, the data trickles, so a test can pause midway.
async fn ftp_server(data: Arc<Vec<u8>>, slow: Arc<AtomicBool>) -> (u16, Arc<Mutex<FtpLog>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(FtpLog::default()));
    let shared = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { return };
            let (data, slow, log) = (data.clone(), slow.clone(), shared.clone());
            tokio::spawn(async move {
                let (read, mut write) = sock.into_split();
                let mut lines = BufReader::new(read).lines();
                let _ = write.write_all(b"220 test server ready\r\n").await;
                let (mut user, mut rest, mut passive) = (String::new(), 0u64, None::<TcpListener>);
                while let Ok(Some(line)) = lines.next_line().await {
                    let (cmd, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                    let reply = match cmd.to_ascii_uppercase().as_str() {
                        "USER" => {
                            user = arg.to_string();
                            "331 password please".to_string()
                        }
                        "PASS" => {
                            log.lock().unwrap().logins.push((user.clone(), arg.to_string()));
                            if user == "me" && arg == "pw" { "230 welcome".into() } else { "530 Login incorrect".into() }
                        }
                        "TYPE" => "200 type set".into(),
                        "SIZE" if arg == "/pub/file.bin" => format!("213 {}", data.len()),
                        "SIZE" => "550 no such file".into(),
                        "REST" => {
                            rest = arg.parse().unwrap_or(0);
                            log.lock().unwrap().rests.push(rest);
                            format!("350 restarting at {rest}")
                        }
                        "PASV" => {
                            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
                            let p = l.local_addr().unwrap().port();
                            passive = Some(l);
                            format!("227 Entering Passive Mode (127,0,0,1,{},{})", p >> 8, p & 0xff)
                        }
                        "RETR" if arg == "/pub/file.bin" => {
                            let Some(l) = passive.take() else { continue };
                            let _ = write.write_all(b"150 here it comes\r\n").await;
                            let Ok((mut conn, _)) = l.accept().await else { return };
                            let from = (rest as usize).min(data.len());
                            for chunk in data[from..].chunks(16 * 1024) {
                                if conn.write_all(chunk).await.is_err() {
                                    break;
                                }
                                if slow.load(Relaxed) {
                                    tokio::time::sleep(Duration::from_millis(40)).await;
                                }
                            }
                            drop(conn);
                            rest = 0;
                            "226 done".into()
                        }
                        "RETR" => "550 no such file".into(),
                        "QUIT" => {
                            let _ = write.write_all(b"221 bye\r\n").await;
                            return;
                        }
                        _ => "502 not here".into(),
                    };
                    if write.write_all(format!("{reply}\r\n").as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, log)
}

/* ---------- a small SFTP server ---------- */

mod sftp_server {
    use super::*;
    use russh::keys::{Algorithm, PrivateKey};
    use russh::server::{Auth, ChannelOpenHandle, Msg, Session};
    use russh::{Channel, ChannelId};
    use russh_sftp::protocol::{Attrs, Data, FileAttributes, Handle, OpenFlags, SftpHandle, SftpString, Status as FxStatus, StatusCode};

    pub fn new_key() -> PrivateKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
    }

    struct Ssh {
        data: Arc<Vec<u8>>,
        channels: HashMap<ChannelId, Channel<Msg>>,
    }

    impl russh::server::Handler for Ssh {
        type Error = russh::Error;

        async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
            Ok(if user == "me" && password == "pw" { Auth::Accept } else { Auth::reject() })
        }

        async fn channel_open_session(&mut self, channel: Channel<Msg>, reply: ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
            self.channels.insert(channel.id(), channel);
            reply.accept().await;
            Ok(())
        }

        async fn channel_eof(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
            session.close(channel)?;
            Ok(())
        }

        async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
            match (name, self.channels.remove(&id)) {
                ("sftp", Some(channel)) => {
                    session.channel_success(id)?;
                    russh_sftp::server::run(channel.into_stream(), Files { data: self.data.clone() }).await;
                }
                _ => session.channel_failure(id)?,
            }
            Ok(())
        }
    }

    struct Files {
        data: Arc<Vec<u8>>,
    }

    const PATH: &str = "/home/me/file.bin";

    impl russh_sftp::server::Handler for Files {
        type Error = StatusCode;

        fn unimplemented(&self) -> Self::Error {
            StatusCode::OpUnsupported
        }

        async fn stat(&mut self, id: u32, path: SftpString) -> Result<Attrs, Self::Error> {
            if path.to_string() != PATH {
                return Err(StatusCode::NoSuchFile);
            }
            let attrs = FileAttributes { size: Some(self.data.len() as u64), ..Default::default() };
            Ok(Attrs { id, attrs })
        }

        async fn lstat(&mut self, id: u32, path: SftpString) -> Result<Attrs, Self::Error> {
            self.stat(id, path).await
        }

        async fn open(&mut self, id: u32, filename: SftpString, _: OpenFlags, _: FileAttributes) -> Result<Handle, Self::Error> {
            if filename.to_string() != PATH {
                return Err(StatusCode::NoSuchFile);
            }
            Ok(Handle { id, handle: filename.to_string().into() })
        }

        async fn fstat(&mut self, id: u32, _: SftpHandle) -> Result<Attrs, Self::Error> {
            let attrs = FileAttributes { size: Some(self.data.len() as u64), ..Default::default() };
            Ok(Attrs { id, attrs })
        }

        async fn read(&mut self, id: u32, _: SftpHandle, offset: u64, len: u32) -> Result<Data, Self::Error> {
            let start = offset as usize;
            if start >= self.data.len() {
                return Err(StatusCode::Eof);
            }
            let end = (start + len as usize).min(self.data.len());
            Ok(Data { id, data: self.data[start..end].to_vec() })
        }

        async fn close(&mut self, id: u32, _: SftpHandle) -> Result<FxStatus, Self::Error> {
            Ok(FxStatus { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en-US".into() })
        }
    }

    /// Serves `data` at /home/me/file.bin to "me"/"pw"; the server proves
    /// itself with whichever key `key` holds when a client connects.
    pub async fn start(data: Arc<Vec<u8>>, key: Arc<Mutex<PrivateKey>>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else { return };
                let config = Arc::new(russh::server::Config {
                    keys: vec![key.lock().unwrap().clone()],
                    auth_rejection_time: Duration::from_millis(10),
                    auth_rejection_time_initial: Some(Duration::ZERO),
                    ..Default::default()
                });
                let handler = Ssh { data: data.clone(), channels: HashMap::new() };
                tokio::spawn(async move {
                    if let Ok(session) = russh::server::run_stream(config, sock, handler).await {
                        let _ = session.await;
                    }
                });
            }
        });
        port
    }
}

/* ---------- the tests ---------- */

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rushdownload-remote-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn engine(dir: &PathBuf) -> Engine {
    let mut settings = Settings::new(dir.join("out"));
    settings.organize.enabled = false;
    settings.auto_update_tools = false;
    settings.secure_dns = false;
    Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap()
}

async fn wait(engine: &Engine, id: u64, done: impl Fn(&TaskInfo) -> bool) -> TaskInfo {
    for _ in 0..600 {
        if let Some(t) = engine.list().into_iter().find(|t| t.id == id) {
            if done(&t) {
                return t;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out: {:?}", engine.list());
}

fn finished(t: &TaskInfo) -> bool {
    matches!(t.status, Status::Completed | Status::Failed)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ftp_file_downloads_with_the_sign_in_from_its_link_kept_private() {
    let data = body(300_000);
    let (port, log) = ftp_server(data.clone(), Arc::new(AtomicBool::new(false))).await;
    let dir = temp_dir("ftp");
    let engine = engine(&dir);
    let id = engine.add(NewDownload { url: format!("ftp://me:pw@127.0.0.1:{port}/pub/file.bin"), ..Default::default() }).unwrap();
    let t = wait(&engine, id, finished).await;
    assert_eq!(t.status, Status::Completed, "{:?}", t.error);
    assert_eq!(t.filename, "file.bin");
    assert_eq!(std::fs::read(&t.path).unwrap(), *data);
    assert_eq!(t.url, format!("ftp://127.0.0.1:{port}/pub/file.bin"), "the sign-in is out of the link");
    let remote = t.remote.unwrap();
    assert_eq!(remote.username, "me");
    assert!(remote.has_password);
    let saved = std::fs::read_to_string(dir.join("state").join("downloads.json")).unwrap();
    assert!(!saved.contains("me:pw") && !saved.contains("\"pw\""), "the password is not in the list");
    assert_eq!(log.lock().unwrap().logins, vec![("me".to_string(), "pw".to_string())]);

    // given with the download instead of in the link
    let id = engine
        .add(NewDownload {
            url: format!("ftp://127.0.0.1:{port}/pub/file.bin"),
            remote: Some(RemoteOptions { username: "me".into(), password: Some("pw".into()), ..Default::default() }),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(wait(&engine, id, finished).await.status, Status::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_password_or_missing_file_fails_at_once_with_why() {
    let (port, log) = ftp_server(body(1000), Arc::new(AtomicBool::new(false))).await;
    let dir = temp_dir("ftp-bad");
    let engine = engine(&dir);
    let id = engine.add(NewDownload { url: format!("ftp://me:nope@127.0.0.1:{port}/pub/file.bin"), ..Default::default() }).unwrap();
    let t = wait(&engine, id, finished).await;
    assert_eq!(t.status, Status::Failed);
    assert!(t.error.unwrap().contains("did not accept the user name and password"));
    assert_eq!(log.lock().unwrap().logins.len(), 1, "a refused sign-in is not tried again and again");

    let id = engine.add(NewDownload { url: format!("ftp://me:pw@127.0.0.1:{port}/pub/missing.bin"), ..Default::default() }).unwrap();
    let t = wait(&engine, id, finished).await;
    assert!(t.error.unwrap().contains("no such file"));

    // anonymous, when no user name is given
    let id = engine.add(NewDownload { url: format!("ftp://127.0.0.1:{port}/pub/file.bin"), ..Default::default() }).unwrap();
    wait(&engine, id, finished).await;
    assert!(log.lock().unwrap().logins.iter().any(|(u, _)| u == "anonymous"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_ftp_download_carries_on_from_where_it_was() {
    let data = body(1_500_000);
    let slow = Arc::new(AtomicBool::new(true));
    let (port, log) = ftp_server(data.clone(), slow.clone()).await;
    let dir = temp_dir("ftp-resume");
    let engine = engine(&dir);
    let id = engine.add(NewDownload { url: format!("ftp://me:pw@127.0.0.1:{port}/pub/file.bin"), ..Default::default() }).unwrap();
    wait(&engine, id, |t| t.downloaded > 200_000).await;
    engine.pause(id);
    let paused = wait(&engine, id, |t| t.status == Status::Paused).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    slow.store(false, Relaxed);
    engine.resume(id);
    let t = wait(&engine, id, finished).await;
    assert_eq!(t.status, Status::Completed, "{:?}", t.error);
    assert_eq!(std::fs::read(&t.path).unwrap(), *data, "byte for byte");
    let rests = log.lock().unwrap().rests.clone();
    assert!(rests.iter().any(|&r| r > 0 && r <= paused.downloaded.max(1) + 64 * 1024), "it asked to carry on: {rests:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chosen_proxy_stops_an_ftp_download_rather_than_going_direct() {
    let (port, log) = ftp_server(body(1000), Arc::new(AtomicBool::new(false))).await;
    let dir = temp_dir("ftp-proxy");
    let engine = engine(&dir);
    let proxy = ProxyProfile { id: 0, name: "P".into(), kind: ProxyKind::Socks5, host: "127.0.0.1".into(), port: 9, username: String::new(), has_password: false, bypass: vec![] };
    let pid = engine.save_proxy(proxy, PasswordChange::Keep).unwrap();
    let id = engine
        .add(NewDownload { url: format!("ftp://me:pw@127.0.0.1:{port}/pub/file.bin"), proxy: ProxyChoice::Profile(pid), ..Default::default() })
        .unwrap();
    let t = wait(&engine, id, finished).await;
    assert_eq!(t.status, Status::Failed);
    assert!(t.error.unwrap().contains("cannot go through a proxy"));
    assert!(log.lock().unwrap().logins.is_empty(), "the server was never reached");
}

#[tokio::test(flavor = "multi_thread")]
async fn sftp_asks_to_trust_an_unknown_server_and_refuses_a_changed_key() {
    let data = body(400_000);
    let key = Arc::new(Mutex::new(sftp_server::new_key()));
    let port = sftp_server::start(data.clone(), key.clone()).await;
    let dir = temp_dir("sftp");
    let engine = engine(&dir);
    let url = format!("sftp://me:pw@127.0.0.1:{port}/home/me/file.bin");

    // unknown: refused, with its fingerprint to check
    let id = engine.add(NewDownload { url: url.clone(), ..Default::default() }).unwrap();
    let t = wait(&engine, id, finished).await;
    assert_eq!(t.status, Status::Failed);
    let shown = t.server_key.clone().expect("the refused key is shown");
    assert!(!shown.changed);
    assert!(shown.fingerprint.starts_with("SHA256:"));
    assert!(t.error.unwrap().contains(&shown.fingerprint));
    let expected = key.lock().unwrap().public_key().fingerprint(russh::keys::HashAlg::Sha256).to_string();
    assert_eq!(shown.fingerprint, expected, "the fingerprint is the server's");

    // trusted: it downloads, byte for byte
    engine.trust_server_key(id).unwrap();
    let t = wait(&engine, id, finished).await;
    assert_eq!(t.status, Status::Completed, "{:?}", t.error);
    assert_eq!(std::fs::read(&t.path).unwrap(), *data);
    assert!(t.server_key.is_none());

    // a wrong password, against a trusted server
    let id = engine.add(NewDownload { url: format!("sftp://me:nope@127.0.0.1:{port}/home/me/file.bin"), ..Default::default() }).unwrap();
    let t = wait(&engine, id, finished).await;
    assert!(t.error.unwrap().contains("did not accept"));

    // the server's key changes: refused, and said to be a change
    *key.lock().unwrap() = sftp_server::new_key();
    let id = engine.add(NewDownload { url, ..Default::default() }).unwrap();
    let t = wait(&engine, id, finished).await;
    assert_eq!(t.status, Status::Failed);
    assert!(t.server_key.unwrap().changed);
    assert!(t.error.unwrap().contains("has changed"));
}
