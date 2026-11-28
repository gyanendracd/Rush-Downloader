# Real-app tests

These scripts drive the real RushDownload window (not the browser preview) and check that the interface and engine work together. Everything stays on this machine: downloads come from a local test server and go to a separate folder, so your own list and Downloads folder are not touched.

## Run

From the project root. Pick any empty folder for `<test-folder>`.

1. Local test site (leave it running):

   ```powershell
   python e2e/server.py 8765
   ```

2. Interface server, unless `npm run tauri dev` is already running:

   ```powershell
   npm run dev
   ```

3. A separate test copy of the app. Build first with `cargo build -p rushdownload`.

   ```powershell
   powershell -ExecutionPolicy Bypass -File e2e\launch.ps1 -TestFolder <test-folder>
   ```

   It prints the test copy's process id and parks its window off-screen. The test copy has its own list, download folder and browser data, so it can run beside your own open copy of the app.

4. The tests, in order:

   ```powershell
   node e2e/cdp.mjs e2e/t1_download.js
   node e2e/cdp.mjs e2e/t2_list.js
   node e2e/cdp.mjs e2e/t3_settings.js
   node e2e/cdp.mjs e2e/t4_confirm.js
   node e2e/cdp.mjs e2e/t5_toggle.js
   node e2e/cdp.mjs e2e/t6_one_box.js
   node e2e/cdp.mjs e2e/t7_path.js
   node e2e/cdp.mjs e2e/t7_torrent_file.js
   ```

Each prints what it observed as JSON, or `FAILED:` with the step that timed out.

`win2.ps1` reads or changes one window's state and always needs `-ProcessId`. Never point it at a copy of the app you did not start for testing.

## What they cover

- `t1_download.js`: paste a link on Home, completion, a multi-connection download, pause (and that it really stops), resume (and that it continues).
- `t2_list.js`: status chips, the detail dialog and its tabs, copy link, the row menu, remove from list, Type and Size filters, search, the empty state, sort, Ctrl and Shift selection, the command palette, navigation shortcuts, the add dialog.
- `t3_settings.js`: settings reaching the engine, theme and density, About, scanning a page for media, duplicate file naming, importing a list of links.
- `t4_confirm.js`: the in-app confirmation before cancelling or deleting, that "keep" and Escape change nothing, and that confirming removes the data from disk.
- `t5_toggle.js`: the details popup's single button switching between Pause and Resume, and showing Retry after a failure.
- `t6_one_box.js`: the single link box accepting file links, video pages, magnet links and .torrent links, and refusing anything else. It submits nothing.
- `t7_torrent_file.js`: a .torrent file's path in the link box opening the torrent dialog, its file list and save folder, the selection gating Download, the download starting, and cancelling removing its folder. It needs the internet and a .torrent file: put its path in `t7_path.js` and run that first. Set `$env:RUSHDOWNLOAD_NO_DHT = "1"` before `launch.ps1` to keep Windows Firewall from asking about the test copy.
- `t8_signin_open.js` then `t8_signin_check.js`: a site's page opening inside the Sign in page, saving and closing it, and forgetting sign-ins removing the kept cookies and the site's browser data. It opens a public page and signs in to nothing; it needs the internet.
- `t9_signin_page.js`: the Sign in page through its own buttons: a site opens, "Save sign-in and close" returns to the list, a visit without signing in is not reported as signed in, and leaving the page closes the site.
- `t12_browser_catch.js`: the Browser page opening a page that plays a video, catching the video file, downloading it and naming it after the page. Start `python e2e/video_page.py <any .mp4> 8766` first.
- `t13_failed_to_browser.js`: a page link that cannot be downloaded from the link alone, and its row menu opening that page on the Browser page. Needs `e2e/video_page.py` on port 8766.
- `t14_open_direct.js` with `read_site.mjs`: the embedded browser reaching Cloudflare's check page through the app's secure-DNS proxy. Launch the test copy by hand with `RUSHDOWNLOAD_DNS_PROXY_PORT=38975`, `secure_dns` on in its settings and `--proxy-server=http://127.0.0.1:38975` added to `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`; then `node e2e/read_site.mjs "dns over"` should report DoH as Yes.
- `t15_page_video.js`: the Browser page on a page the site downloader understands: the "Full video" row with the page's title and resolutions, small caught files marked as preview clips, and the full video downloading. Needs `e2e/video_page.py` on port 8766.
- `t16_clipboard.js`: a link copied anywhere on the computer landing in the Home box. Copy a link first (`Set-Clipboard -Value "https://example.com/files/a.zip"`), wait two seconds, then run it; it reports the page showing and what the box holds. Plain text must leave the box unchanged. The test changes the clipboard, so save and restore your own (`Get-Clipboard -Raw`).
- `t19_stream_qualities.js`: a page whose video is a stream with its playlist at an address that does not look like one. One row, the stream's qualities in the list and nothing else, and the chosen quality downloading as a whole video. Start `python e2e/video_page.py <video> 8766 <folder with master.m3u8 and its pieces>`.
- `t18_home_opens_page_and_waits.js`: a link pasted on Home that cannot be downloaded from the link alone. The app opens the page on the Browser page and starts the player so the qualities show up, then waits: nothing may download until a quality is chosen and Download is pressed. Set `window.__link` to `/click/demo` (use a test video of more than 4 MB) or `/stream/demo`.
- `t20_home_card_for_caught_video.js`: a link typed on Home whose page the site downloader cannot read. Home shows the same card as for other videos (picture, title, qualities) by opening the page out of sight; the app stays on Home and nothing downloads until a quality is picked and Download is pressed. Needs `e2e/video_page.py` with a stream folder.
- `t21_playlist_card.js`: a playlist link typed on Home (set `window.__link`): the card shows the playlist's picture, its name and how many videos it holds. Needs the internet; nothing is downloaded.
- `t22_playlist_and_profile_details.js`: a playlist link (`window.__link`) and a profile link (`window.__profile`) typed on Home: the card counts the videos and photos, gives the videos' total length, and offers a resolution for a playlist; Settings has the switch for asking where to save each one. Needs the internet; nothing is downloaded. The folder window itself is not driven by the test.
- `w_max.js`, `w_min.js`, `w_close.js` with `win2.ps1`: the window's own buttons, checked against the real window state.

## Not covered

The Windows folder and file pickers, dragging the window, and site downloads and torrents, which need the internet. Torrents have a live engine test instead: `cargo test --release -p rushdownload-engine -- --ignored torrent` (it uses Ubuntu's official torrent).

## Browser extension pairing

Start the test copy with a port for the extension, then run the check (it pairs as a made-up extension, presses Allow in the test window, and tries the key):

```powershell
$env:RUSHDOWNLOAD_EXT_PORT = "38999"
powershell -ExecutionPolicy Bypass -File e2e\launch.ps1 -TestFolder <test-folder>
node e2e/t23_extension_pair.mjs
node e2e/cdp.mjs e2e/t23_revoke.js
```
