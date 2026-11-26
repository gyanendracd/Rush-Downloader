# RushDownload design system

Written from the built interface (`src/styles.css`, `src/ui.tsx`, `src/views.tsx`). The layout began as the user's product mockup; the user then chose one taste for the whole theme: polished liquid glass. Change it only with them.

## Character

Liquid glass: translucent panes floating over a soft field of colour, lit along their top edge, with near-black text and one saturated blue. Nothing behind a pane has a hard edge, so text never crosses a shape. Dark mode is the same design on a near-black ground with deeper colour in the field.

Five materials, never the same one twice in a stack (tokens `--glass-*`, section "liquid glass" in the stylesheet):

| Material | Used for | Blur |
|---|---|---|
| Chrome | the sidebar | yes |
| Surface | cards, lists, panes of settings, the Home "downloading now" pane | no |
| Control | buttons, chips, segmented controls | no |
| Float | menus, toasts, tooltips, the selection toolbar | yes |
| Sheet | dialogs, the command palette | yes |

## Layout

- Frameless window. The app draws its own minimise, maximise and close buttons at the top right; the brand row and the strip beside the buttons drag the window.
- Two columns: sidebar 176px, workspace fluid. Below 760px the sidebar collapses to 60px of icons.
- Sidebar holds Home, Downloads, History, Browser, Sign in and Settings, then, only while something is downloading, waiting or paused, the transfer summary (speed now, and how far along what is in hand has got; finished downloads are not counted), with Dark Mode and About pinned to the bottom.
- Home is one centred column, 700px at most so a long link can be read in its box: heading, link box, the two secondary buttons, then a pane listing up to three running downloads with their progress; a slim capsule of four marks rests at the foot.
- Settings has its sections as tabs across the top, the way a browser has them: the tab in front is one piece with the panel of settings under it. In a narrow window the tabs show their marks only. Each setting is one line: its name, and a sentence about it where one is needed, on the left; its control on the right. Lines that belong together share a pane.
- A download's detail is a centred dialog opened by clicking its row. There is no side panel, no table header and no status bar.
- Page padding 28px.

## Colour

Tokens live on `:root` (light) and `:root[data-theme="dark"]`.

| Role | Light | Dark |
|---|---|---|
| Ground `--bg` | `#eef1f7` | `#0b0c11` |
| Colour field `--wall-1` to `--wall-4` | `#c6dbff` `#e2d3ff` `#c4ecf3` `#ffdbe6` | `#17306b` `#3a2168` `#0d3d4b` `#45193a` |
| Content pane `--glass-regular` | white at 32% | white at 6.5% |
| Hairline `--border` | `rgba(60, 60, 67, 0.12)` | `rgba(255, 255, 255, 0.1)` |
| Text `--text` / `--text-2` / `--text-3` | `#1d1d1f` / `#55555c` / `#68686f` | `#f5f5f7` / `#b4b4bb` / `#8e8e96` |
| Accent `--accent` | `#0b63f6` | `#6ea8ff` |
| Filled button `--accent-solid` | `#0b63f6` | `#2a6cf0` |
| Active nav, active chip, selection `--accent-soft` | accent at 12% | `rgba(80, 140, 255, 0.2)` |
| Done `--ok` / `--ok-solid` | `#15803d` / `#16a34a` | `#4ad38a` / `#1f9d57` |
| Failed `--err` / `--err-solid` | `#d0262d` / `#e5343b` | `#ff7b82` / `#d93a42` |
| Speed shade, slowest on screen `--speed-min` | `#cfe0ff` | `#1a2f63` |
| Speed shade, fastest on screen `--speed-max` | `#0a36a8` | `#a9d0ff` |

The speed graph uses one colour only, the logo's blue, mixed between `--speed-min` and `--speed-max`. The shade is relative to what the graph shows: the slowest moment on screen is the palest and the fastest is the strongest (deeper on light, brighter on dark), so any rise or dip is visible even on a steady connection. A fixed scale in MB/s was tried and left steady downloads looking like one flat colour. The graph carries no legend; the speed label and the hover tooltip give the numbers. Numbers on the graph stay in `--accent` so they read at every speed.

Logo: a solid bolt filled with a diagonal gradient from `#2F7DFC` (top left) to `#0B60FB` (bottom right), sampled from the user's logo image. The same bolt is the app icon (`src-tauri/icons`).

Background art: four large glows behind the workspace (`--art-1` to `--art-3`), each fading to nothing with no edge. They were hard-edged discs once; an edge crossing a line of text or a pane read as a flaw, so they are glows.

File tiles are coloured by type: video blue, audio green, image orange, document red, archive amber, program dark blue, other slate.

## Type

Inter Variable, bundled with the app. 13px base. Welcome heading 30px/700. Page titles 20px/700. Section names in Settings 17px/650. File names 13.5px/600. Secondary text 12px. Large type is tracked tighter (-0.035em at 30px, -0.025em at 20px); body text is left near zero. Changing numbers use tabular figures.

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
- **Settings**: a strip of tabs (General, Downloads, Network, Performance, Scheduler, Appearance, Advanced) standing on one panel. The front tab and the panel share one fill (`--tab-fill`) with nothing drawn between them, and the tab's feet curve out into the panel's top edge; tabs behind are plain, with a hairline between neighbours. The front tab never animates away from the panel. Picking another tab is one move (a view transition of the panel and its content only, so the rest of the window stays live): the panel glides to its new height in 320ms, what was in it slides off the way the tabs run and fades in 150ms, and the new content follows it in from the other side. Without view transitions, or with reduced motion, the tab simply changes. The panel holds panes of settings, drawn flat so there is one sheet of glass, not two (`.setting-card`, `.setting-row`). An on-off setting is a whole line with its switch at the end; settings that are not built yet sit in their own pane under "Not built yet".
- **Downloading now** (Home): a pane with the count and total speed as its heading, which opens the Downloads page, and up to three running downloads under it, each a small file tile, name, percent and a 3px bar.
- **A saved profile**: its row in the list and its dialog show the profile's own picture, round, where a file's tile would be; a video from a site shows the site's picture of it, cut to the tile's shape, and a playlist its first video's; any other file keeps its coloured tile. In the dialog's list each finished file is its own picture: a photo as it is, a video by its first frame, loaded only once its row comes into view.
- **Profile card** (Home): the profile's round picture, the name it shows, then `@handle`, counts and site, up to two lines of what the profile says about itself, and its numbers where the site tells a visitor who is not signed in (posts, how many carry photos or videos, followers, following, when it joined). A profile with a banner shows it across the top of the card, with the picture standing half over its lower edge. Who it is arrives first and on its own; the counts join the line when they are in, and are simply absent where the site will not list the posts.

## Motion

120–180ms ease-out on state changes only. `prefers-reduced-motion` removes it.

## Downloads and History

Two pages over the same list, for two questions.

- **Downloads** answers "what can I use, and what is on its way": everything running, waiting, failed or still on disk, and what was deleted in the last 10 days. A finished download whose file is gone is greyed, its name struck through, and its line reads "Deleted" with how long ago and how big it was; the word carries the meaning, the grey only supports it. Its round button fetches it again. Older deleted downloads leave this page.
- **History** answers "what have I ever downloaded": every link, newest first, as a table (Name, Link, Size, Added, Status) whose column heads stay put. A profile or a playlist is one small card in the table, and under it every file it fetched, a line each in the same columns, scrolling on their own past eight. What a profile or playlist fetched is written down when it finishes, so its lines are still there, as deleted, after the files are gone.
- Removing a download removes it from both pages. Nothing here deletes a file by itself: "deleted" only ever reports what was found on disk.

## When glass is not wanted

`prefers-reduced-transparency: reduce` and `prefers-contrast: more` swap every material for a solid one: no blur, opaque panes, no colour field. More contrast also darkens secondary text and firms up the hairlines. The layout does not change.

## Honesty rules

- Options from the mockup that the app cannot do yet (start with Windows, tray, notifications, hardware acceleration, languages, scheduler) are shown switched off with a "Soon" tag, never as working controls.
- "Add from Browser" opens the page scan and "Browse Files" imports a text file of links; both have tooltips that say so.
- Sample data lives only in `src/mock.ts`, loaded in development outside Tauri and absent from production builds.
