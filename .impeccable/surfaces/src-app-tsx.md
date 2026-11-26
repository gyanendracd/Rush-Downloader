---
version: 1
slug: "src-app-tsx"
primary_target: "src/App.tsx"
related_targets: ["src/styles.css"]
---

# RushDownload app shell

Scope: the whole desktop app window (src/App.tsx and everything under src/). Mode: Operate.

Audience and job: someone with the app open all day who pastes links, watches several downloads, and acts on them; casual by default, every engine detail one click away.

Constraints: only controls backed by engine features that exist. Offline desktop app, no network fonts. The user supplied a written specification and a reference mockup; both are binding.

## Direction contract

THESIS: A download manager built like a serious desktop tool, where the list of downloads is the product and everything else stays out of its way. Refuses the card-and-chart dashboard.

OWN-WORLD: Cool neutral grounds (dark #101114 / light #F7F8FA), a second neutral layer for the sidebar, one electric blue (#4388F5 dark, #276FE5 light) reserved for primary action, selection, focus and running progress; green, amber and red only for done, paused and failed. Inter at 13px, tabular figures, 6px radius, hairline separators, flat rows, minimal shadow only on overlays. Lightning-bolt mark.

STORY: The user sees what is running and how fast, trusts the state shown, and can act on any download without hunting.

FIRST VIEWPORT: 232px sidebar (Home, Downloads, Queue, History, Failed, then file-type categories, Settings and theme at the bottom); main workspace with page title, filter tabs with counts, search and sort, then the dense list; a collapsible 340px inspector on the right for the selected download. Primary action "Add download" top right of the workspace.

FORM: Direction pinned by the user's specification and mockup, which overrides the roll. seed:b16399fd3f543cfc.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

Signature interaction: the progress bar of a segmented download shows each connection's own range filling independently. Motion: 120–180ms ease-out on state changes only.

Unresolved: Queue reordering, scheduling, tray, browser integration, torrents and other protocols are not built in the engine and are absent from the interface.
