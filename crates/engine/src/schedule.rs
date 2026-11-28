//! Download schedules: start a group of downloads at a set local time, once
//! or on chosen days of the week, and pause them again at an optional stop
//! time.
//!
//! Times are kept as wall-clock times ("09:00 on Mondays"), not as instants,
//! and turned into instants with the computer's time zone each time they are
//! checked. A change of time zone, or of summer time, therefore moves a
//! schedule with the clock on the wall, the way an alarm clock does.
//!
//! Each start a schedule meets is written down by its local date and time
//! before anything is done about it, so a start is acted on at most once:
//! not again after a restart, nor when the clock is set back.
//!
//! Everything here is plain computation on a given "now"; the engine (lib.rs)
//! runs it every few seconds, saves the result and starts or pauses the
//! downloads it names.

use chrono::{Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};

/// Minutes in a day; a time of day is kept as minutes after midnight.
const DAY: u16 = 24 * 60;

/// A start found this many minutes after its time is late: the app was
/// closed, or the computer asleep, when it came.
const GRACE_MINUTES: i64 = 2;

/// When a schedule starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Repeat {
    /// Once, on this day ("2026-11-28").
    Once { date: String },
    /// Every week on these days: bit 0 is Monday, bit 6 Sunday.
    Weekly { days: u8 },
}

/// What the user sets for a schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleSpec {
    pub name: String,
    pub repeat: Repeat,
    /// Start time, in minutes after midnight.
    pub start: u16,
    /// Optional stop time, in minutes after midnight. One at or before the
    /// start time is on the next day (a night from 23:00 to 06:00).
    pub stop: Option<u16>,
    /// The downloads it starts, in the order they were picked.
    pub downloads: Vec<u64>,
    /// Where its downloads' files go once finished, in place of the folder
    /// their kind of file is sorted into; none keeps the usual place.
    #[serde(default)]
    pub folder: Option<String>,
}

/// What last happened to a schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Started,
    Stopped,
    /// Its start came while the app could not act on it, and was let go.
    Missed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub kind: EventKind,
    /// When, in seconds since 1970.
    pub at: i64,
}

/// A schedule as it is saved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Schedule {
    pub id: u64,
    #[serde(flatten)]
    pub spec: ScheduleSpec,
    /// False while the user has paused the schedule.
    pub enabled: bool,
    pub created_at: i64,
    /// The last start dealt with (started or missed), as a local date and
    /// time in minutes (see `key`). Starts up to this one are never acted on again.
    #[serde(default)]
    seen: Option<i64>,
    /// The last start that did start its downloads.
    #[serde(default)]
    started: Option<i64>,
    /// The last stop that paused them.
    #[serde(default)]
    stopped: Option<i64>,
    #[serde(default)]
    pub last: Option<Event>,
    /// A one-time schedule that has nothing left to do.
    #[serde(default)]
    pub done: bool,
}

/// Every schedule, as saved in `schedules.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Book {
    pub next_id: u64,
    pub items: Vec<Schedule>,
}

/// Where a schedule stands, for the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    /// Waiting for its next start.
    Waiting,
    /// Started, and its downloads are running or its time is not up.
    Active,
    /// Paused by the user.
    Paused,
    /// A one-time schedule that has run.
    Done,
    /// A one-time schedule whose start was missed.
    Missed,
}

/// A schedule as the interface shows it.
#[derive(Clone, Debug, Serialize)]
pub struct ScheduleInfo {
    pub id: u64,
    #[serde(flatten)]
    pub spec: ScheduleSpec,
    pub enabled: bool,
    pub created_at: i64,
    pub status: ScheduleStatus,
    /// The next start, in seconds since 1970.
    pub next_start: Option<i64>,
    /// When the running window ends, or the next one would, in seconds since 1970.
    pub next_stop: Option<i64>,
    pub last: Option<Event>,
}

/// What a check asks the engine to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    Start(Vec<u64>),
    Stop(Vec<u64>),
}

/// A local date and time as one number (minutes since 1970, read as if the
/// clock were UTC), so starts can be compared and saved simply.
fn key(t: NaiveDateTime) -> i64 {
    t.and_utc().timestamp().div_euclid(60)
}

fn from_key(k: i64) -> NaiveDateTime {
    chrono::DateTime::from_timestamp(k * 60, 0).map(|t| t.naive_utc()).unwrap_or_default()
}

fn time_of(minutes: u16) -> NaiveTime {
    NaiveTime::from_hms_opt(u32::from(minutes / 60), u32::from(minutes % 60), 0).unwrap_or(NaiveTime::MIN)
}

fn date_of(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

fn has_day(days: u8, date: NaiveDate) -> bool {
    days & (1 << date.weekday().num_days_from_monday()) != 0
}

/// The latest start at or before `now`.
fn latest_start(spec: &ScheduleSpec, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let at = time_of(spec.start);
    match &spec.repeat {
        Repeat::Once { date } => date_of(date).map(|d| d.and_time(at)).filter(|t| *t <= now),
        Repeat::Weekly { days } => (0..8)
            .map(|back| (now.date() - Duration::days(back)).and_time(at))
            .find(|t| *t <= now && has_day(*days, t.date())),
    }
}

/// The first start after `now`.
fn next_start(spec: &ScheduleSpec, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let at = time_of(spec.start);
    match &spec.repeat {
        Repeat::Once { date } => date_of(date).map(|d| d.and_time(at)).filter(|t| *t > now),
        Repeat::Weekly { days } => (0..8)
            .map(|ahead| (now.date() + Duration::days(ahead)).and_time(at))
            .find(|t| *t > now && has_day(*days, t.date())),
    }
}

/// When the window opened by `start` ends, if the schedule has a stop time.
fn stop_after(spec: &ScheduleSpec, start: NaiveDateTime) -> Option<NaiveDateTime> {
    let stop = spec.stop?;
    let same_day = start.date().and_time(time_of(stop));
    Some(if stop > spec.start { same_day } else { same_day + Duration::days(1) })
}

/// The instant a local date and time stands for, in this computer's time
/// zone now. A time skipped when the clocks go forward is read an hour on;
/// a time that happens twice when they go back is its first.
pub fn instant(t: NaiveDateTime) -> i64 {
    match Local.from_local_datetime(&t) {
        LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => at.timestamp(),
        LocalResult::None => Local
            .from_local_datetime(&(t + Duration::hours(1)))
            .earliest()
            .map_or_else(|| t.and_utc().timestamp(), |at| at.timestamp()),
    }
}

/// The local date and time now.
pub fn local_now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// Checks what a schedule is asked for and why not, before it is saved.
pub fn check(spec: &ScheduleSpec) -> Result<(), &'static str> {
    if spec.start >= DAY || spec.stop.is_some_and(|s| s >= DAY) {
        return Err("time of day out of range");
    }
    if spec.downloads.is_empty() {
        return Err("no downloads picked");
    }
    match &spec.repeat {
        Repeat::Once { date } if date_of(date).is_none() => Err("not a date"),
        Repeat::Weekly { days } if days & 0x7f == 0 => Err("no days picked"),
        _ => Ok(()),
    }
}

impl Schedule {
    pub fn new(id: u64, spec: ScheduleSpec, now: NaiveDateTime, created_at: i64) -> Schedule {
        let mut s = Schedule {
            id,
            spec,
            enabled: true,
            created_at,
            seen: None,
            started: None,
            stopped: None,
            last: None,
            done: false,
        };
        s.forget_past(now);
        s
    }

    /// After the schedule is made or changed: a start already gone by is
    /// not caught up on, unless its window is still open, in which case it
    /// starts at the next check.
    pub fn forget_past(&mut self, now: NaiveDateTime) {
        self.done = false;
        self.seen = None;
        self.started = None;
        self.stopped = None;
        if let Some(start) = latest_start(&self.spec, now) {
            if !self.window_open(start, now) {
                self.seen = Some(key(start));
            }
        }
        self.finish_once(now);
    }

    /// The window a start opens is open at `now` only for a schedule with a stop time.
    fn window_open(&self, start: NaiveDateTime, now: NaiveDateTime) -> bool {
        stop_after(&self.spec, start).is_some_and(|stop| now < stop)
    }

    /// Whether `now` is inside a window the schedule should be running in:
    /// its downloads are then let run rather than held back.
    pub fn in_window(&self, now: NaiveDateTime) -> bool {
        self.enabled && !self.done && latest_start(&self.spec, now).is_some_and(|s| self.window_open(s, now))
    }

    /// A one-time schedule with no start left and nothing left to stop is done.
    fn finish_once(&mut self, now: NaiveDateTime) {
        if !matches!(self.spec.repeat, Repeat::Once { .. }) || next_start(&self.spec, now).is_some() {
            return;
        }
        let stop_due = self
            .started
            .and_then(|k| stop_after(&self.spec, from_key(k)))
            .is_some_and(|stop| self.stopped.is_none_or(|k| k < key(stop)));
        if self.seen.is_some() && !stop_due {
            self.done = true;
        }
    }

    /// One check at local time `now` (`at`: the same instant in seconds since
    /// 1970). `launch` is true for the first check after the app opens;
    /// `start_on_launch` is the setting that lets a start missed while it
    /// was closed run then. Returns what to do, and whether the schedule
    /// changed and must be saved.
    pub fn step(&mut self, now: NaiveDateTime, at: i64, launch: bool, start_on_launch: bool) -> (Vec<Act>, bool) {
        let mut acts = Vec::new();
        let mut changed = false;
        if !self.enabled || self.done {
            return (acts, changed);
        }

        // The end of the window last started, first, so that a window that
        // ended while the app was closed is closed before anything new opens.
        if let Some(started) = self.started {
            if let Some(stop) = stop_after(&self.spec, from_key(started)) {
                if now >= stop && self.stopped.is_none_or(|k| k < key(stop)) {
                    self.stopped = Some(key(stop));
                    self.last = Some(Event { kind: EventKind::Stopped, at });
                    acts.push(Act::Stop(self.spec.downloads.clone()));
                    changed = true;
                }
            }
        }

        if let Some(start) = latest_start(&self.spec, now) {
            if self.seen.is_none_or(|k| k < key(start)) {
                let late = (now - start).num_minutes() >= GRACE_MINUTES;
                let window_left = match stop_after(&self.spec, start) {
                    Some(stop) => now < stop,
                    // with no stop time, the downloads run until they finish
                    None => true,
                };
                let run = if !late {
                    true
                } else if !window_left {
                    false
                } else if launch {
                    start_on_launch
                } else {
                    // the computer was asleep, or the clock moved: still in time
                    true
                };
                self.seen = Some(key(start));
                if run {
                    self.started = Some(key(start));
                    self.last = Some(Event { kind: EventKind::Started, at });
                    acts.push(Act::Start(self.spec.downloads.clone()));
                } else {
                    self.last = Some(Event { kind: EventKind::Missed, at });
                }
                changed = true;
            }
        }

        let was_done = self.done;
        self.finish_once(now);
        (acts, changed || self.done != was_done)
    }

    /// The schedule as the interface shows it. `running` tells whether any
    /// of its downloads is queued or running.
    pub fn info(&self, now: NaiveDateTime, running: bool) -> ScheduleInfo {
        let open = self
            .started
            .map(from_key)
            .filter(|s| latest_start(&self.spec, now) == Some(*s))
            .map(|s| match stop_after(&self.spec, s) {
                Some(stop) => now < stop,
                None => running,
            })
            .unwrap_or(false);
        let status = if !self.enabled {
            ScheduleStatus::Paused
        } else if self.done {
            if self.started.is_some() {
                ScheduleStatus::Done
            } else {
                ScheduleStatus::Missed
            }
        } else if open {
            ScheduleStatus::Active
        } else {
            ScheduleStatus::Waiting
        };
        let next = if self.done { None } else { next_start(&self.spec, now) };
        let window_stop = if status == ScheduleStatus::Active {
            self.started.and_then(|s| stop_after(&self.spec, from_key(s)))
        } else {
            next.and_then(|n| stop_after(&self.spec, n))
        };
        ScheduleInfo {
            id: self.id,
            spec: self.spec.clone(),
            enabled: self.enabled,
            created_at: self.created_at,
            status,
            next_start: next.map(instant),
            next_stop: window_stop.map(instant),
            last: self.last,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    fn weekly(days: u8, start: u16, stop: Option<u16>) -> ScheduleSpec {
        ScheduleSpec { name: "night".into(), repeat: Repeat::Weekly { days }, start, stop, downloads: vec![7, 9], folder: None }
    }

    fn once(date: &str, start: u16, stop: Option<u16>) -> ScheduleSpec {
        ScheduleSpec { name: "once".into(), repeat: Repeat::Once { date: date.into() }, start, stop, downloads: vec![7], folder: None }
    }

    const MON: u8 = 1;
    const EVERY_DAY: u8 = 0x7f;

    // 2026-11-30 is a Monday.

    #[test]
    fn starts_once_on_time_and_never_twice() {
        let mut s = Schedule::new(1, weekly(MON, 9 * 60, None), at("2026-11-29 12:00"), 0);
        assert_eq!(s.step(at("2026-11-30 08:59"), 0, false, true).0, vec![]);
        assert_eq!(s.step(at("2026-11-30 09:00"), 0, false, true).0, vec![Act::Start(vec![7, 9])]);
        assert_eq!(s.step(at("2026-11-30 09:00"), 0, false, true).0, vec![]);
        // the clock set back an hour, then forward again: still once
        assert_eq!(s.step(at("2026-11-30 08:30"), 0, false, true).0, vec![]);
        assert_eq!(s.step(at("2026-11-30 09:30"), 0, false, true).0, vec![]);
        // next Monday it starts again
        assert_eq!(s.step(at("2026-12-07 09:00"), 0, false, true).0, vec![Act::Start(vec![7, 9])]);
    }

    #[test]
    fn a_start_gone_by_when_made_is_not_caught_up_on() {
        let mut s = Schedule::new(1, weekly(EVERY_DAY, 9 * 60, None), at("2026-11-30 15:00"), 0);
        assert_eq!(s.step(at("2026-11-30 15:00"), 0, false, true).0, vec![]);
        assert_eq!(s.info(at("2026-11-30 15:00"), false).status, ScheduleStatus::Waiting);
    }

    #[test]
    fn made_inside_its_window_it_starts_at_once() {
        let mut s = Schedule::new(1, weekly(EVERY_DAY, 9 * 60, Some(17 * 60)), at("2026-11-30 15:00"), 0);
        assert!(s.in_window(at("2026-11-30 15:00")));
        assert_eq!(s.step(at("2026-11-30 15:00"), 0, false, true).0, vec![Act::Start(vec![7, 9])]);
        assert_eq!(s.info(at("2026-11-30 15:01"), false).status, ScheduleStatus::Active);
        assert_eq!(s.step(at("2026-11-30 17:00"), 0, false, true).0, vec![Act::Stop(vec![7, 9])]);
        assert_eq!(s.info(at("2026-11-30 17:01"), false).status, ScheduleStatus::Waiting);
    }

    #[test]
    fn a_night_window_stops_the_next_morning() {
        let mut s = Schedule::new(1, weekly(MON, 23 * 60, Some(6 * 60)), at("2026-11-30 12:00"), 0);
        assert_eq!(s.step(at("2026-11-30 23:00"), 0, false, true).0, vec![Act::Start(vec![7, 9])]);
        assert_eq!(s.step(at("2026-12-01 05:59"), 0, false, true).0, vec![]);
        assert_eq!(s.step(at("2026-12-01 06:00"), 0, false, true).0, vec![Act::Stop(vec![7, 9])]);
    }

    #[test]
    fn missed_while_closed_with_its_window_open() {
        let spec = weekly(MON, 9 * 60, Some(12 * 60));
        // the setting on: it starts when the app opens
        let mut s = Schedule::new(1, spec.clone(), at("2026-11-29 12:00"), 0);
        assert_eq!(s.step(at("2026-11-30 10:00"), 0, true, true).0, vec![Act::Start(vec![7, 9])]);
        // the setting off: it waits for next week
        let mut s = Schedule::new(1, spec, at("2026-11-29 12:00"), 0);
        assert_eq!(s.step(at("2026-11-30 10:00"), 0, true, false).0, vec![]);
        assert_eq!(s.last.unwrap().kind, EventKind::Missed);
        assert_eq!(s.step(at("2026-11-30 10:01"), 0, false, false).0, vec![]);
    }

    #[test]
    fn missed_after_its_window_closed() {
        let mut s = Schedule::new(1, once("2026-11-30", 9 * 60, Some(10 * 60)), at("2026-11-29 12:00"), 0);
        assert_eq!(s.step(at("2026-11-30 11:00"), 0, true, true).0, vec![]);
        assert!(s.done);
        assert_eq!(s.info(at("2026-11-30 11:00"), false).status, ScheduleStatus::Missed);
    }

    #[test]
    fn a_window_that_ended_while_closed_is_closed_at_launch() {
        let mut s = Schedule::new(1, weekly(MON, 9 * 60, Some(10 * 60)), at("2026-11-29 12:00"), 0);
        assert_eq!(s.step(at("2026-11-30 09:00"), 0, false, true).0, vec![Act::Start(vec![7, 9])]);
        // the app closes at 09:30 and opens at 14:00: its downloads were put
        // back in the queue, so they are paused now
        assert_eq!(s.step(at("2026-11-30 14:00"), 0, true, true).0, vec![Act::Stop(vec![7, 9])]);
    }

    #[test]
    fn woken_from_sleep_without_a_stop_time_still_starts() {
        let mut s = Schedule::new(1, weekly(MON, 9 * 60, None), at("2026-11-29 12:00"), 0);
        assert_eq!(s.step(at("2026-11-30 11:00"), 0, false, false).0, vec![Act::Start(vec![7, 9])]);
    }

    #[test]
    fn once_runs_then_is_done() {
        let mut s = Schedule::new(1, once("2026-11-30", 9 * 60, None), at("2026-11-29 12:00"), 0);
        assert_eq!(s.info(at("2026-11-29 12:00"), false).status, ScheduleStatus::Waiting);
        assert_eq!(s.step(at("2026-11-30 09:00"), 0, false, true).0, vec![Act::Start(vec![7])]);
        assert!(s.done);
        assert_eq!(s.info(at("2026-11-30 09:01"), true).status, ScheduleStatus::Done);
        assert_eq!(s.step(at("2026-11-30 09:05"), 0, false, true).0, vec![]);
    }

    #[test]
    fn once_with_a_stop_is_done_after_it_stops() {
        let mut s = Schedule::new(1, once("2026-11-30", 9 * 60, Some(9 * 60 + 30)), at("2026-11-29 12:00"), 0);
        s.step(at("2026-11-30 09:00"), 0, false, true);
        assert!(!s.done);
        assert_eq!(s.step(at("2026-11-30 09:30"), 0, false, true).0, vec![Act::Stop(vec![7])]);
        assert!(s.done);
    }

    #[test]
    fn a_paused_schedule_does_nothing() {
        let mut s = Schedule::new(1, weekly(MON, 9 * 60, None), at("2026-11-29 12:00"), 0);
        s.enabled = false;
        assert_eq!(s.step(at("2026-11-30 09:00"), 0, false, true).0, vec![]);
        assert_eq!(s.info(at("2026-11-30 09:00"), false).status, ScheduleStatus::Paused);
    }

    #[test]
    fn next_start_skips_days_not_picked() {
        let s = Schedule::new(1, weekly(MON | 1 << 4, 9 * 60, None), at("2026-11-30 10:00"), 0);
        let next = next_start(&s.spec, at("2026-11-30 10:00")).unwrap();
        assert_eq!(next, at("2026-12-04 09:00")); // Friday
    }

    #[test]
    fn keys_round_trip() {
        let t = at("2026-11-30 23:59");
        assert_eq!(from_key(key(t)), t);
    }

    #[test]
    fn checks_what_is_asked() {
        assert!(check(&weekly(0, 60, None)).is_err());
        assert!(check(&weekly(MON, DAY, None)).is_err());
        assert!(check(&once("tomorrow", 60, None)).is_err());
        let mut none = weekly(MON, 60, None);
        none.downloads.clear();
        assert!(check(&none).is_err());
        assert!(check(&weekly(MON, 60, Some(30))).is_ok());
    }
}
