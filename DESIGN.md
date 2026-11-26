# RushDownload design system

Written from the built interface (`src/styles.css`, `src/ui.tsx`). The look follows the user's product mockup, which they asked to be reproduced as it is. Change it only with them.

## Character

Soft, light and friendly: a pale lavender-white ground with two faint glows, navy text, one saturated blue, rounded shapes, coloured file tiles. Dark mode is the same design on a navy-black ground.

## Layout

- Frameless window. The app draws its own minimise, maximise and close buttons at the top right; the brand row and the strip beside the buttons drag the window.
- Two columns: sidebar 176px, workspace fluid. Below 760px the sidebar collapses to 60px of icons.
- Sidebar holds only Home, Downloads and Settings, with Dark Mode and About pinned to the bottom.
- A download's detail is a centred dialog opened by clicking its row. There is no side panel, no table header and no status bar.
- Page padding 28px.

## Colour

Tokens live on `:root` (light) and `:root[data-theme="dark"]`.

| Role | Light | Dark |
|---|---|---|
| Ground `--bg` (plus `--glow-1`, `--glow-2`) | `#f3f5fb` | `#0b0f1d` |
| Raised surface `--surface` | `#ffffff` | `#141a2e` |
| Soft fill, chips, hover `--hover` | `#eef2fb` | `#1b2238` |
| Hairline `--border` | `#e2e7f2` | `#232b45` |
| Text `--text` / `--text-2` / `--text-3` | `#0f1b3d` / `#4f5a75` / `#6a748c` | `#eef1fb` / `#aab3ca` / `#8d97b2` |
| Accent `--accent` | `#1f62f0` | `#7aa2ff` |
| Filled button `--accent-solid` | `#1f62f0` | `#2b63e6` |
| Active nav, active chip, selection `--accent-soft` | `#e4ecff` | `#1a2850` |
| Done `--ok` / `--ok-solid` | `#15803d` / `#16a34a` | `#4ad38a` / `#1f9d57` |
| Failed `--err` / `--err-solid` | `#d0262d` / `#e5343b` | `#ff7b82` / `#d93a42` |
| Speed shade, slowest on screen `--speed-min` | `#cfe0ff` | `#1a2f63` |
| Speed shade, fastest on screen `--speed-max` | `#0a36a8` | `#a9d0ff` |

The speed graph uses one colour only, the logo's blue, mixed between `--speed-min` and `--speed-max`. The shade is relative to what the graph shows: the slowest moment on screen is the palest and the fastest is the strongest (deeper on light, brighter on dark), so any rise or dip is visible even on a steady connection. A fixed scale in MB/s was tried and left steady downloads looking like one flat colour. The graph carries no legend; the speed label and the hover tooltip give the numbers. Numbers on the graph stay in `--accent` so they read at every speed.

Logo: a solid bolt filled with a diagonal gradient from `#2F7DFC` (top left) to `#0B60FB` (bottom right), sampled from the user's logo image. The same bolt is the app icon (`src-tauri/icons`).

Background art: four large soft discs behind the workspace, in those two logo blues at 7 to 11 percent opacity (`--art-1` to `--art-3`).

File tiles are coloured by type: video blue, audio green, image orange, document red, archive amber, program dark blue, other slate.

## Type

Inter Variable, bundled with the app. 13px base. Welcome heading 24px/750. Page titles 15px/700. File names 13.5px/650. Secondary text 12px. Changing numbers use tabular figures.

## Shape

- Pills (999px) for status chips and the two Home buttons.
- 10px for buttons, fields, nav items and icon buttons; 12px for rows and wide dialog buttons; 16–18px for the paste field and dialogs; 11px for file tiles.
- Round 32px buttons for pause and resume; a 22px filled green tick for done and red exclamation for failed.
- Soft shadow only on overlays and the two Home pill buttons.

## Components

- **Nav item**: icon and label; the active one gets a pale blue fill, blue text and a 3px blue bar on its left edge.
- **Paste field**: a large rounded field with the blue Download button inside it and a soft blue ring.
- **Status chips**: All, Downloading, Completed, Paused, Failed, each with its count; Queued appears only while something is waiting.
- **Row**: file tile, bold name, one line of status and facts (the status word coloured for done and failed), a 3px bar while unfinished, time left at the right, one round action and a "more" menu. Right-click opens the same menu. Separators are inset hairlines.
- **Filter bar**: Type, Date and Size dropdowns and a search field, shown by the search and filter buttons beside "Sort by".
- **Detail dialog**: large tile, name, blue summary line, bar with time left, tabs Progress, Details and Options, a speed graph drawn as one bold wave through up to nine averaged points (no markers along it; only the pointed-at moment gets one), with a dense row of 96 bars beneath whose softly eased tops (1.5px corners, not half-circles) trace the same curve a little below the line, every bar the same dark grey (`--bar`: `#7d8491` on light, `#4d5567` on dark) so the blue wave is the only colour in the graph, with 3px between bars, and a fill that fades to nothing; each stretch of the wave and its fill take the shade of the speed at that point, and pointing at the graph shows that moment's speed in a small tooltip with a guide line, and three equal buttons (Pause, Resume, Cancel in red).
- **Settings**: a second column of sections (General, Downloads, Network, Scheduler, Appearance, Advanced) beside the content.

## Motion

120–180ms ease-out on state changes only. `prefers-reduced-motion` removes it.

## Honesty rules

- Options from the mockup that the app cannot do yet (start with Windows, tray, notifications, hardware acceleration, languages, scheduler) are shown switched off with a "Soon" tag, never as working controls.
- "Add from Browser" opens the page scan and "Browse Files" imports a text file of links; both have tooltips that say so.
- Sample data lives only in `src/mock.ts`, loaded in development outside Tauri and absent from production builds.
