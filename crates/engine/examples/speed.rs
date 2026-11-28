//! Measures how fast the engine downloads a link, for a set time, with a
//! chosen number of connections and secure DNS on or off; nothing is kept.
//!
//! `cargo run --release -p rushdownload-engine --example speed -- <url> [seconds] [connections] [dns:on|off]`

use std::time::{Duration, Instant};

use rushdownload_engine::{Engine, MemoryStore, NewDownload, Settings, Status};

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let url = args.next().ok_or("usage: speed <url> [seconds] [connections] [dns:on|off]")?;
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(15);
    let connections: usize = args.next().and_then(|c| c.parse().ok()).unwrap_or(16);
    let secure_dns = args.next().map_or(true, |d| d != "dns:off");

    let root = std::env::temp_dir().join(format!("rushdownload-speed-{}", std::process::id()));
    let mut settings = Settings::new(root.join("out"));
    settings.secure_dns = secure_dns;
    settings.auto_update_tools = false;
    settings.organize.enabled = false;
    let engine = Engine::with_secrets(root.join("state"), settings, std::sync::Arc::new(MemoryStore::default()))?;
    let id = engine.add(NewDownload { url, connections: Some(connections), ..Default::default() })?;

    let started = Instant::now();
    let mut first: Option<(Instant, u64)> = None;
    let mut peak_conns = 0;
    let mut last_line = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Some(t) = engine.list().into_iter().find(|t| t.id == id) else { break };
        peak_conns = peak_conns.max(t.active_connections);
        if t.status == Status::Failed {
            return Err(t.error.unwrap_or_default());
        }
        if first.is_none() && t.downloaded > 0 {
            first = Some((Instant::now(), t.downloaded));
        }
        if last_line.elapsed() >= Duration::from_secs(1) {
            last_line = Instant::now();
            println!("{:5.1}s  {:7.2} MB/s now  {:3} connections", started.elapsed().as_secs_f64(), t.speed as f64 / 1_048_576.0, t.active_connections);
        }
        let done = t.status == Status::Completed;
        if done || started.elapsed() >= Duration::from_secs(seconds) {
            if let Some((at, bytes)) = first {
                let secs = at.elapsed().as_secs_f64().max(0.001);
                println!(
                    "average {:.2} MB/s over {:.1}s (dns {}, {} connections asked, {} at most)",
                    (t.downloaded - bytes) as f64 / secs / 1_048_576.0,
                    secs,
                    if secure_dns { "secure" } else { "system" },
                    connections,
                    peak_conns
                );
            }
            break;
        }
    }
    engine.remove(id, true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
