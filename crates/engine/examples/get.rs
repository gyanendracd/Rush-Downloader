//! Command-line smoke test: `cargo run --release -p rushdownload-engine --example get -- <url> [dir] [connections]`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rushdownload_engine::{Engine, NewDownload, Settings, Status};

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let url = args.next().ok_or("usage: get <url> [dir] [connections]")?;
    let dir = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let connections = args.next().and_then(|c| c.parse().ok());

    let state = std::env::temp_dir().join(format!("rushdownload-cli-{}", std::process::id()));
    let engine = Engine::new(&state, Settings::new(dir))?;
    let id = engine.add(NewDownload {
        url,
        connections,
        ..Default::default()
    })?;

    let started = Instant::now();
    let mut changes = engine.subscribe();
    loop {
        let _ = tokio::time::timeout(Duration::from_secs(1), changes.changed()).await;
        let Some(t) = engine.list().into_iter().find(|t| t.id == id) else {
            return Err("download disappeared".into());
        };
        match t.status {
            Status::Completed => {
                let secs = started.elapsed().as_secs_f64();
                println!(
                    "\ndone: {} ({} bytes in {:.1}s, {:.2} MB/s)",
                    t.path.display(),
                    t.downloaded,
                    secs,
                    t.downloaded as f64 / secs / 1_048_576.0
                );
                let _ = std::fs::remove_dir_all(&state);
                return Ok(());
            }
            Status::Failed => return Err(t.error.unwrap_or_else(|| "failed".into())),
            _ => {
                let pct = t.total.map_or(0.0, |x| t.downloaded as f64 * 100.0 / x.max(1) as f64);
                print!(
                    "\r{:5.1}%  {:7.2} MB/s  {} connections  {} segments   ",
                    pct,
                    t.speed as f64 / 1_048_576.0,
                    t.active_connections,
                    t.segments.len()
                );
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
    }
}
