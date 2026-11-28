/* English text of the Scheduler page and its dialog (src/Scheduler.tsx).
 * Dates, times, day names and "in 3 hours" are written by the browser in
 * the interface's language, so they are not here. */
import { plural } from "../format";

export const scheduler = {
  title: "Scheduler",
  intro: "Start downloads at a set time, once or every week, and pause them again at a stop time. Schedules run while RushDownload is open, also in the tray.",
  newSchedule: "New schedule",
  sections: {
    active: "Running now",
    upcoming: "Upcoming",
    paused: "Paused",
    finished: "Finished",
  },
  empty: {
    title: "No schedules yet",
    text: "Make one to download at night, or while you are away.",
  },
  status: {
    waiting: "Waiting",
    active: "Running",
    paused: "Paused",
    done: "Done",
    missed: "Missed",
  },
  /** `date`: the day, already written out. */
  once: (date: string) => `Once, ${date}`,
  everyDay: "Every day",
  weekdays: "Monday to Friday",
  weekends: "Saturday and Sunday",
  /** `days`: the days, already listed ("Mon, Wed and Fri"). */
  onDays: (days: string) => `Every ${days}`,
  /** A window: "23:00 to 07:00". */
  window: (start: string, stop: string) => `${start} to ${stop}`,
  /** `when`: a date and time, and how far off it is. */
  nextStart: (when: string) => `Next start: ${when}`,
  stopsAt: (when: string) => `Pauses again: ${when}`,
  noNext: "No start left",
  last: {
    started: (when: string) => `Started ${when}`,
    stopped: (when: string) => `Paused again ${when}`,
    missed: (when: string) => `Missed ${when}, as RushDownload was closed`,
  },
  downloads: (n: number) => plural(n, { one: "# download", other: "# downloads" }),
  /** The rest of the downloads, after the first few are named. */
  more: (n: number) => plural(n, { other: "+#" }),
  noDownloads: "Its downloads have been removed from the list.",
  actions: {
    pause: "Pause schedule",
    resume: "Resume schedule",
    edit: "Edit",
    delete: "Delete",
  },
  deleted: "Schedule deleted. Its downloads stay as they are.",
  saved: "Schedule saved",
  dialog: {
    newTitle: "New schedule",
    editTitle: "Edit schedule",
    name: "Name",
    namePlaceholder: "Night downloads",
    repeat: "Repeat",
    once: "Once",
    weekly: "Every week",
    date: "Date",
    days: "Days",
    start: "Start at",
    stop: "Pause again at",
    stopHint: "A time at or before the start is on the next day, for a night from 23:00 to 07:00.",
    downloads: "Downloads",
    downloadsHint: "They wait, paused, until the schedule starts them, and start in this order.",
    noneToPick: "Nothing to schedule: every download in the list has finished.",
    save: "Save schedule",
    errors: {
      downloads: "Pick at least one download.",
      days: "Pick at least one day.",
      past: "That time has already passed.",
    },
  },
  settings: {
    onLaunch: "Start missed schedules when RushDownload opens",
    onLaunchHint:
      "A schedule whose start came while the app was closed starts when it opens, if its pause time has not come yet. Off, it waits for its next start.",
    open: "Open Scheduler",
    count: (n: number) => plural(n, { one: "# schedule", other: "# schedules" }),
  },
  /** In a download's right-click menu. */
  menu: "Schedule…",
};
