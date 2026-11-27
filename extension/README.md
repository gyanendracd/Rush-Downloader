# RushDownload browser extension

Finds the videos on the page you are looking at and sends them to the RushDownload app on the same computer. For Chrome, Edge, Brave and other Chromium browsers.

## Install

1. Open `chrome://extensions` (or `edge://extensions`).
2. Turn on **Developer mode**.
3. Choose **Load unpacked** and pick this `extension` folder.

The app has to be open for links to reach it. The popup says whether it is.

## Use

- **The toolbar button** lists every video, audio file and stream found on the page. Tick the ones you want and press **Send to app**: they start downloading straight away.
- **Send page** hands the page's own link to the app, which looks it up and lets you pick a quality on Home. Use this for sites whose player shows no file of its own (YouTube and the like).
- **Right-click → Send to RushDownload** on a video, a link or the page does the same without opening the popup. The badge shows OK or ! for a moment.
- The number on the button is how many files the tab has loaded so far.

## What it reads

- `scan.js` runs in the page, and each frame inside it, only when the popup is opened. It collects addresses from video and audio elements, links to media files, embedded players of other sites, and media addresses written in the page's code.
- `background.js` notes the address of each video, audio file or stream playlist a tab loads, until the tab moves to another page. Pieces of a stream are left out.

Only addresses, the page's link and its title leave the browser, and only to the app on this computer (`127.0.0.1:38951`). Cookies are not sent: a file that needs your sign-in is better sent with **Send page**, which uses the sign-ins kept in the app.

## How it reaches the app

The app listens on `127.0.0.1:38951` (`src-tauri/src/extension.rs`; the port is also in `shared.js`, and the two must match). It adds links only for requests a browser marks as coming from an extension, so a web page cannot start downloads, and it takes nothing but plain `http` and `https` addresses.

`RUSHDOWNLOAD_EXT_PORT` gives the app another port. A test copy of the app (one started with `RUSHDOWNLOAD_STATE_DIR`) does not listen unless that is set.

## Trying it out

`popup.html?tab=<tab id>` opened in a tab of its own reads that tab instead of the one in front, which lets the popup be driven by a test. `e2e/video_page.py` serves pages to try it on.
