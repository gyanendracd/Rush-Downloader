//! The app's log: one file a day in the `logs` folder beside its other
//! state, the last two weeks kept. Passwords and proxy keys are never
//! written to it (the engine prints proxies without them).
//!
//! `RUSHDOWNLOAD_LOG` sets what is written, in the usual filter form
//! ("debug", "rushdownload_engine=trace"); `RUSHDOWNLOAD_DEBUG` turns on the
//! engine's notes on site downloads.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Starts writing the log. Keep the guard for as long as the app runs: the
/// last lines are written out when it is dropped.
pub fn init(state_dir: &Path) -> Option<WorkerGuard> {
    let dir = state_dir.join("logs");
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("rushdownload")
        .filename_suffix("log")
        .max_log_files(14)
        .build(&dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let default = if std::env::var_os("RUSHDOWNLOAD_DEBUG").is_some() {
        "info,rushdownload_engine=debug,rushdownload_lib=debug"
    } else {
        "info,librqbit=warn,librqbit_dht=warn,librqbit_upnp=warn,librqbit_tracker_comms=warn"
    };
    let filter = EnvFilter::try_from_env("RUSHDOWNLOAD_LOG").unwrap_or_else(|_| EnvFilter::new(default));
    let file = fmt::layer().with_writer(writer).with_ansi(false).with_target(true);
    // While developing, the same lines on the terminal too.
    let terminal = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr).with_target(true));
    tracing_subscriber::registry().with(filter).with(file).with(terminal).try_init().ok()?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "RushDownload started");
    Some(guard)
}
