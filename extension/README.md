# RushDownload browser extension

Sends the downloads your browser starts, and the videos on the page you are looking at, to the RushDownload app on the same computer. One source tree for Chrome, Edge, Brave (Chromium, Manifest V3) and Firefox (121 and later).

## Install for development

- **Chrome, Edge, Brave:** open `chrome://extensions` (or `edge://extensions`), turn on **Developer mode**, choose **Load unpacked** and pick this `extension` folder. It installs as "RushDownload (development)".
- **Firefox:** run `npm run extension:build:dev`, open `about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on** and pick `dist-extension/firefox-dev/manifest.json`. Firefox asks for host permissions separately: allow "Access your data for all websites" under the add-on's Permissions, or nothing can be caught.

## Connect it to the app

The app must be open. Open the extension and press **Connect**: the app asks "Connect a browser extension?" and names the browser. Allow it, and the extension keeps the key the app gives it. Until then the app refuses everything the extension sends.

Connected browsers are listed in the app under **Settings, Advanced, Browser extension**, where **Disconnect** removes one; that browser must connect again.

## Use

- **Downloads the browser starts** go to the app instead, when the extension is connected and the app takes them. If the app is closed, not connected, or refuses a link, the browser keeps downloading as usual. The browser's own download is stopped only after the app has taken it.
- **Options** (the extension's options page, or the link in its popup): turn catching off, leave chosen sites, file types, or files under a size to the browser, and choose whether the site's cookies go along.
- **Right-click a link, Download with the browser** keeps that one file in the browser.
- **The toolbar button** lists every video, audio file and stream found on the page. Tick the ones you want and press **Send to app**.
- **Send page** hands the page's link to the app, which looks it up and lets you pick a quality on Home (YouTube and the like).
- **Right-click, Send to RushDownload** on a video, a link or the page does the same without the popup.

## What it reads, and why it asks for each permission

| Permission | Why |
|---|---|
| `downloads` | To see downloads the browser starts, and to stop the browser's copy once the app has taken it. |
| `cookies` | To send a file's site cookies along, which a download behind a sign-in needs. Off in Options if you prefer. |
| `webRequest`, host access to all sites | To note the address of each video, audio file or stream a tab loads (pieces of a stream are left out), so the popup can list files the page itself does not show. |
| `scripting`, `activeTab` | To read the page you are on for video and audio addresses, only when you open the popup. |
| `contextMenus` | The right-click items. |
| `storage` | The key from the app, the Options, and each tab's list of found files. |

Only addresses, the page's link and title, and (if allowed) the cookies of a caught download leave the browser, and only to the app on this computer (`127.0.0.1:38951`). Nothing is collected or sent anywhere else. In the app, cookies are kept encrypted with the download and never sent on to another site.

## How it reaches the app

The app listens on `127.0.0.1:38951` (`src-tauri/src/extension.rs`; the port is also in `shared.js`, and the two must match). Pairing (`POST /pair`, then `GET /pair/<id>`) needs the user's yes in the app; afterwards every request carries `Authorization: Bearer <key>`, which must belong to the same extension (`Origin`). The app keeps only a hash of each key. `/ping` says only that the app is open and whether the asking extension is connected.

`RUSHDOWNLOAD_EXT_PORT` gives the app another port. A test copy of the app (one started with `RUSHDOWNLOAD_STATE_DIR`) does not listen unless that is set.

## Build for the stores

```
npm run extension:build
```

writes `dist-extension/chromium/` and `dist-extension/firefox/`, and a `.zip` of each ready to upload to the Chrome Web Store, Microsoft Edge Add-ons and Firefox Add-ons. `npm run extension:build:dev` writes development builds ("RushDownload (Dev)", their own Firefox id) beside them. The version comes from `package.json`, so the extension and the app carry the same number; the build warns when `src-tauri/tauri.conf.json` says otherwise. `extension/manifest.json` itself is the development manifest.

## Known limits

- A download the browser could only make with a form (POST) or a one-time link may not download again in the app; leave such sites to the browser in Options.
- A sign-in made with the browser's own password box (HTTP authentication) cannot be read by extensions; give the app the user name and password in the download's Request options instead.
- A file whose size the site does not say is always sent, whatever the size limit.

## Trying it out

`popup.html?tab=<tab id>` opened in a tab of its own reads that tab instead of the one in front, which lets the popup be driven by a test. `e2e/video_page.py` serves pages to try it on, and `e2e/t23_extension_pair.mjs` checks pairing against a test copy of the app.
