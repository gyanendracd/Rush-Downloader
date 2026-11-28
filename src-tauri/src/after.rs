//! What happens once every download has finished: nothing, a notification,
//! closing or quitting the app, or putting the computer to sleep, into
//! hibernation or shutting it down (Settings → Downloads).
//!
//! A batch is every download seen running since the list was last idle. It
//! is done when none of them is running any more, none is paused (a paused
//! download is still wanted), and no schedule is in its running window with
//! downloads of its own left. It counts only if at least one finished; a
//! batch with a failure or a cancelled download counts only if the setting
//! for that is on.
//!
//! Everything but a notification waits through a countdown, shown in a
//! dialog with Cancel; any download that starts during it cancels it. The
//! actions run fixed programs or system calls only, never text from anyone.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::time::Duration;

use rushdownload_engine::{AfterAll, Engine, ScheduleStatus, Status};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/* ---------- the batch ---------- */

#[derive(Debug, Default)]
pub(crate) struct Batch {
    members: HashSet<u64>,
}

/// What a look at the list found.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    /// Something is queued or running.
    Busy,
    /// Nothing runs, but the batch is not done (or there is none).
    Waiting,
    /// The batch is done; it is forgotten, so this is said once.
    Done { completed: bool, troubled: bool },
}

impl Batch {
    /// `list`: every download's id and status. `scheduled`: a schedule in
    /// its running window still has downloads to finish.
    pub(crate) fn observe(&mut self, list: &[(u64, Status)], scheduled: bool) -> Seen {
        let running = |s: Status| matches!(s, Status::Queued | Status::Connecting | Status::Downloading);
        let mut busy = false;
        for (id, status) in list {
            if running(*status) {
                busy = true;
                self.members.insert(*id);
            }
        }
        if busy {
            return Seen::Busy;
        }
        if self.members.is_empty() || scheduled {
            return Seen::Waiting;
        }
        let (mut completed, mut troubled) = (false, false);
        for id in &self.members {
            match list.iter().find(|(d, _)| d == id).map(|(_, s)| *s) {
                Some(Status::Paused) => return Seen::Waiting,
                Some(Status::Completed) => completed = true,
                // failed, or taken off the list (cancelled) before it finished
                Some(_) | None => troubled = true,
            }
        }
        self.members.clear();
        Seen::Done { completed, troubled }
    }
}

/* ---------- the countdown ---------- */

/// What the window is told, as the event "after-all".
#[derive(Clone, Serialize)]
struct Notice {
    action: AfterAll,
    /// Seconds left; null once the countdown is over or stopped.
    left: Option<u32>,
    /// "done" (it ran, or for a notification: send it now), "cancelled"
    /// (by the user), or "busy" (a download started).
    reason: Option<&'static str>,
}

/// The countdown in progress, if any. A new number stops the one before.
#[derive(Default)]
pub(crate) struct Countdown {
    round: AtomicU64,
    running: AtomicBool,
    now: AtomicBool,
}

impl Countdown {
    fn stop(&self, app: &AppHandle, action: AfterAll, reason: &'static str) {
        if self.running.swap(false, SeqCst) {
            self.round.fetch_add(1, SeqCst);
            let _ = app.emit("after-all", Notice { action, left: None, reason: Some(reason) });
        }
    }
}

/// Watches the list and, when a batch is done, does what the settings say.
pub(crate) fn watch(app: AppHandle, engine: Engine) {
    app.manage(Countdown::default());
    let mut changes = engine.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut batch = Batch::default();
        loop {
            let list: Vec<(u64, Status)> = engine.list().iter().map(|t| (t.id, t.status)).collect();
            let scheduled = engine.schedules().iter().any(|s| {
                s.status == ScheduleStatus::Active
                    && s.spec.downloads.iter().any(|id| list.iter().any(|(d, st)| d == id && *st != Status::Completed))
            });
            let settings = engine.settings();
            let countdown = app.state::<Countdown>();
            match batch.observe(&list, scheduled) {
                Seen::Busy => countdown.stop(&app, settings.after_all, "busy"),
                Seen::Waiting => {}
                Seen::Done { completed, troubled } => {
                    if completed && (!troubled || settings.after_failures) {
                        begin(&app, settings.after_all, settings.after_countdown);
                    }
                }
            }
            if changes.changed().await.is_err() {
                return;
            }
        }
    });
}

fn begin(app: &AppHandle, action: AfterAll, seconds: u32) {
    match action {
        AfterAll::Nothing => return,
        AfterAll::Notify => {
            // sent by the window, in the interface's language
            let _ = app.emit("after-all", Notice { action, left: None, reason: Some("done") });
            return;
        }
        _ => {}
    }
    let countdown = app.state::<Countdown>();
    let round = countdown.round.fetch_add(1, SeqCst) + 1;
    countdown.running.store(true, SeqCst);
    countdown.now.store(false, SeqCst);
    // The computer or the app is about to go: the window comes to the front,
    // so the countdown can be seen and stopped.
    if action != AfterAll::Close {
        crate::shell::bring_back(app);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let countdown = app.state::<Countdown>();
        let mut left = seconds.max(1);
        while left > 0 {
            if countdown.round.load(SeqCst) != round {
                return;
            }
            let _ = app.emit("after-all", Notice { action, left: Some(left), reason: None });
            for _ in 0..10 {
                if countdown.now.load(SeqCst) || countdown.round.load(SeqCst) != round {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if countdown.now.load(SeqCst) {
                break;
            }
            left -= 1;
        }
        if countdown.round.load(SeqCst) != round || !countdown.running.swap(false, SeqCst) {
            return;
        }
        let _ = app.emit("after-all", Notice { action, left: None, reason: Some("done") });
        // a moment for the window to say it, then the action
        std::thread::sleep(Duration::from_millis(300));
        if let Err(e) = perform(&app, action) {
            let _ = app.emit("after-all-failed", e);
        }
    });
}

/// Stops the countdown (the dialog's Cancel).
#[tauri::command]
pub(crate) fn after_all_cancel(app: AppHandle, state: tauri::State<crate::AppState>) {
    let action = state.0.settings().after_all;
    app.state::<Countdown>().stop(&app, action, "cancelled");
}

/// Does it now, without waiting for the countdown to end.
#[tauri::command]
pub(crate) fn after_all_now(app: AppHandle) {
    let countdown = app.state::<Countdown>();
    if countdown.running.load(SeqCst) {
        countdown.now.store(true, SeqCst);
    }
}

/* ---------- the actions ---------- */

fn perform(app: &AppHandle, action: AfterAll) -> Result<(), String> {
    match action {
        AfterAll::Nothing | AfterAll::Notify => Ok(()),
        // as the window's close button: to the tray if that is on, else the app ends
        AfterAll::Close => match app.get_webview_window("main") {
            Some(window) => window.close().map_err(|e| e.to_string()),
            None => Ok(()),
        },
        AfterAll::Exit => {
            app.exit(0);
            Ok(())
        }
        AfterAll::Sleep => power::sleep(),
        AfterAll::Hibernate => power::hibernate(),
        AfterAll::Shutdown => power::shut_down(),
    }
}

#[cfg(windows)]
mod power {
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;

    /// A program of Windows' own, by its full path, so no other program of
    /// the same name is run in its place.
    fn system32(exe: &str) -> PathBuf {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        root.join("System32").join(exe)
    }

    fn run(exe: &str, args: &[&str]) -> Result<(), String> {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let status = Command::new(system32(exe))
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{exe} ended with {status}"))
        }
    }

    pub(super) fn shut_down() -> Result<(), String> {
        run("shutdown.exe", &["/s", "/t", "0"])
    }

    pub(super) fn hibernate() -> Result<(), String> {
        run("shutdown.exe", &["/h"])
    }

    pub(super) fn sleep() -> Result<(), String> {
        // SAFETY: a plain call into powrprof.dll with three flags.
        let done = unsafe { windows_sys::Win32::System::Power::SetSuspendState(false, false, false) };
        if done {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    }
}

#[cfg(target_os = "macos")]
mod power {
    use std::process::Command;

    fn run(program: &str, args: &[&str]) -> Result<(), String> {
        let status = Command::new(program).args(args).status().map_err(|e| e.to_string())?;
        if status.success() { Ok(()) } else { Err(format!("{program} ended with {status}")) }
    }

    pub(super) fn shut_down() -> Result<(), String> {
        run("/usr/bin/osascript", &["-e", "tell application \"System Events\" to shut down"])
    }

    pub(super) fn hibernate() -> Result<(), String> {
        Err("hibernation is not offered on macOS".into())
    }

    pub(super) fn sleep() -> Result<(), String> {
        run("/usr/bin/pmset", &["sleepnow"])
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod power {
    use std::process::Command;

    fn systemctl(verb: &str) -> Result<(), String> {
        let status = Command::new("systemctl").arg(verb).status().map_err(|e| e.to_string())?;
        if status.success() { Ok(()) } else { Err(format!("systemctl {verb} ended with {status}")) }
    }

    pub(super) fn shut_down() -> Result<(), String> {
        systemctl("poweroff")
    }

    pub(super) fn hibernate() -> Result<(), String> {
        systemctl("hibernate")
    }

    pub(super) fn sleep() -> Result<(), String> {
        systemctl("suspend")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Status::*;

    #[test]
    fn done_once_every_download_of_the_batch_has_finished() {
        let mut b = Batch::default();
        assert_eq!(b.observe(&[(1, Completed)], false), Seen::Waiting, "nothing ran: no batch");
        assert_eq!(b.observe(&[(1, Downloading), (2, Queued)], false), Seen::Busy);
        assert_eq!(b.observe(&[(1, Completed), (2, Downloading)], false), Seen::Busy);
        assert_eq!(b.observe(&[(1, Completed), (2, Completed)], false), Seen::Done { completed: true, troubled: false });
        assert_eq!(b.observe(&[(1, Completed), (2, Completed)], false), Seen::Waiting, "said once");
    }

    #[test]
    fn a_paused_download_holds_it_back() {
        let mut b = Batch::default();
        b.observe(&[(1, Downloading), (2, Downloading)], false);
        assert_eq!(b.observe(&[(1, Completed), (2, Paused)], false), Seen::Waiting);
        // resumed, and then it finishes
        assert_eq!(b.observe(&[(1, Completed), (2, Queued)], false), Seen::Busy);
        assert_eq!(b.observe(&[(1, Completed), (2, Completed)], false), Seen::Done { completed: true, troubled: false });
    }

    #[test]
    fn a_schedule_still_running_holds_it_back() {
        let mut b = Batch::default();
        b.observe(&[(1, Downloading)], false);
        assert_eq!(b.observe(&[(1, Completed), (5, Paused)], true), Seen::Waiting);
        assert_eq!(b.observe(&[(1, Completed), (5, Paused)], false), Seen::Done { completed: true, troubled: false });
    }

    #[test]
    fn failures_and_cancellations_are_told() {
        let mut b = Batch::default();
        b.observe(&[(1, Downloading), (2, Downloading), (3, Downloading)], false);
        // 2 failed, 3 was taken off the list
        assert_eq!(b.observe(&[(1, Completed), (2, Failed)], false), Seen::Done { completed: true, troubled: true });

        b.observe(&[(4, Downloading)], false);
        assert_eq!(b.observe(&[], false), Seen::Done { completed: false, troubled: true }, "all cancelled: nothing finished");
    }

    #[test]
    fn a_download_paused_before_the_batch_is_not_waited_for() {
        let mut b = Batch::default();
        b.observe(&[(1, Paused), (2, Downloading)], false);
        assert_eq!(b.observe(&[(1, Paused), (2, Completed)], false), Seen::Done { completed: true, troubled: false });
    }
}
