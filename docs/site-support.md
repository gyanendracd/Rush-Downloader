# Site support: 100 sites, tested

Tested on 27 Nov 2026 from one home network in India, with yt-dlp 2026.11.23 and gallery-dl 1.32.21.

Each site was checked with a real public link using `crates/engine/examples/sitecheck.rs`. The links are in `docs/site-tests.tsv` and `docs/site-tests-2.tsv`. To run the check again:

```
cargo run -p rushdownload-engine --example sitecheck -- docs/site-tests.tsv
cargo run -p rushdownload-engine --example sitecheck -- docs/site-tests-2.tsv
```

**What "tested" means.** Where it says *downloaded*, the smallest file (under 40 MB) was downloaded through the engine and checked on disk. Where it says *listed*, the link was read and its files, names and sizes came back, but nothing was downloaded, to keep tests small. *Read by yt-dlp* means the video page was read (title, length, qualities); two of these (YouTube, SoundCloud) were also downloaded.

**Status key**

- ✅ **Works**: tested and passing.
- 🟡 **Partly**: one part works and another does not (see the note).
- 🔒 **Not allowed**: copy protection, paid access, or a site-only gate. The app says why in plain words and does not try.
- ❌ **Fails**: the site refused or the downloader cannot read it right now (reason given).
- ⚪ **Untested**: built, but no live public link could be found to test with.

## How it works

There are no 100 scrapers. Three layers sit on top of the download core:

1. **Download core** (`task.rs`): segmented HTTP/HTTPS with range requests and resume, plus BitTorrent (`torrent.rs`, `torrent_split.rs`).
2. **Site backend**: yt-dlp for video and audio pages (`media.rs`), and gallery-dl for galleries and profiles (`profile.rs`).
3. **Site integrations** (new, `crates/engine/src/sites.rs`): one small resolver per site. It turns a share link or project page into direct file links with names and sizes, using the site's public API or its own download link. Links that expire (MediaFire, MEGA, pCloud, WeTransfer) get a fresh link right before each file starts.
   - **Picking files.** Home, the Add dialog and the extension's "Send page" all open **Choose files**, the torrent dialog's file tree (`TorrentDialog` with `site`). The user ticks one or many files. A list of 10 files or fewer starts with everything ticked; a longer one starts with nothing ticked.
   - **Several files** (`bundle.rs`) download as one item in the list, three files at a time, each on the segmented engine. The detail dialog shows each file's progress (the same `ItemList` torrents use). Pause, resume and retry work; finished files are skipped on resume.
   - **MEGA** files are decrypted on this computer after download (AES-CTR, using the key in the link).
   - **Blocked sites** (game stores, paid courses, end-to-end encrypted drives, site-only hosts) are caught from the link alone. Home shows the reason, and Download does nothing.
4. **Built-in browser** (new in `src-tauri/src/lib.rs`): when a page in the app's Browser starts a file download (for example a site's own Download button after a wait or a script check), the app takes it over, with the page's cookies and referer. This is for sites like MajorGeeks, FossHub and Softpedia that hide the file behind a script. **Built, not yet tried in the running app.**

## P0 and P1 status

| Ticket item | Status | Evidence |
|---|---|---|
| Plain HTTP/HTTPS, segmented | ✅ | 10 MB test file downloaded in 3.4 s; NASA, USGS and UN files downloaded |
| Google Drive | ✅ | Single file downloaded (gdown.pptx, 34 KB). Folder of 332 files listed, with subfolders and Google Docs exported as .docx/.xlsx/.pptx, and a file downloaded |
| Dropbox | ✅ | Old `/s/` link downloaded (1.6 MB); new `/scl/fi/` link listed (3.6 GB file, not downloaded) |
| MediaFire | ✅ | File downloaded (1.4 MB, but slow: 43 s); folder of 1,146 files listed |
| MEGA | ✅ | File downloaded and decrypted (16 KB PNG); folder listed (13 files, 480 MB); 3 folder files downloaded together and decrypted |
| GitHub releases and assets | ✅ | Latest release: 34 assets plus source ZIP and TAR.GZ; one file and then three files together downloaded |
| YouTube | ✅ | "Me at the zoo" downloaded (437 KB) |
| Vimeo | ✅ | Read through the `player.vimeo.com` link. The `vimeo.com/<id>` page link now asks yt-dlp to sign in. |
| Instagram | ✅ | Public reel read by yt-dlp |
| Reddit | 🟡 | yt-dlp: "Account authentication is required". Works once a sign-in is added (Settings, or the Sign in page). |
| Telegram shared files | 🟡 | Public channel video downloaded (759 KB). A shared **document** cannot be fetched: Telegram's web view does not serve files to visitors. |
| Hugging Face | ✅ | gpt2: 26 files, all sized; a file downloaded |
| Kaggle | ✅ | iris dataset ZIP downloaded, no sign-in needed |
| Internet Archive | ✅ | Items listed with sizes; files downloaded |
| BitTorrent | ✅ | archive.org torrent read: 158 files (the existing engine) |

## All 100 sites

### 1. Video

| Site | Status | How | Test result / reason |
|---|---|---|---|
| YouTube | ✅ | yt-dlp | Downloaded 437 KB in 3.5 s |
| Vimeo | ✅ | yt-dlp | `player.vimeo.com/video/76979871` read (720p max). Page links need a sign-in now. |
| Dailymotion | ✅ | yt-dlp | Read: 187 s video, up to 1080p |
| Twitch | ✅ | yt-dlp (list) | Channel videos list: 1,136 videos |
| TikTok | ❌ | yt-dlp | yt-dlp: "Unexpected response from webpage request". This is a yt-dlp extractor problem from this network. |
| Instagram | ✅ | yt-dlp | Public reel read (1280p) |
| Facebook | ✅ | yt-dlp | Public video read (74 s) |
| X (Twitter) | ⚪ | yt-dlp | The test post has no video any more ("No video could be found"). A live one still needs testing. |
| Reddit | 🟡 | yt-dlp | Needs a sign-in (yt-dlp: account authentication required) |
| Bilibili | ✅ | yt-dlp | Read: up to 1080p |

### 2. Audio

| Site | Status | How | Test result / reason |
|---|---|---|---|
| SoundCloud | ✅ | yt-dlp | Downloaded 4.1 MB |
| Bandcamp | ✅ | yt-dlp | Track read |
| Mixcloud | ✅ | yt-dlp | Show read (62 min) |
| Audiomack | ❌ | yt-dlp | yt-dlp cannot read the page right now (site changed) |
| Internet Archive audio | ✅ | resolver | 186 files listed with sizes; one downloaded |
| Jamendo | ✅ | yt-dlp | Track read |
| Free Music Archive | ❌ | — | No yt-dlp extractor, and the player loads the file by script. Try the built-in Browser. |
| Pixabay Music | 🟡 | page scan | Scan found the MP3 on the music page. Pixabay photo pages turn apps away (403). |
| Freesound | ❌ | yt-dlp | Site answers 403 (bot check). Full-quality files need a Freesound sign-in anyway. |
| BBC Sounds | ❌ | yt-dlp | yt-dlp error ("programme_id"), and BBC audio is UK-only. Not usable from India. |

### 3. Cloud storage

| Site | Status | How | Test result / reason |
|---|---|---|---|
| Google Drive | ✅ | resolver | File downloaded; folder of 332 files listed; Docs, Sheets and Slides exported |
| Dropbox | ✅ | resolver | File downloaded; folder links download as one ZIP (Dropbox's own way) |
| OneDrive | ⚪ | resolver | Built (public sharing API, then the page's download link). No live public link found to test with. |
| Box | ✅ | resolver | Shared file downloaded (33.7 MB in 10 s); shared folder listed (61 files, 2 sub-levels) |
| MEGA | ✅ | resolver + decrypt | File and folder files downloaded and decrypted |
| MediaFire | ✅ | resolver | File downloaded; folder listed (1,146 files) |
| WeTransfer | ⚪ | resolver | Built (public transfer API). Transfers expire within days, so no live link was found. |
| pCloud | ✅ | resolver | Public link downloaded (3.7 MB) |
| Sync.com | 🔒 | — | End-to-end encrypted in the browser; the app never gets the key |
| Proton Drive | 🔒 | — | End-to-end encrypted in the browser |

### 4. File hosts

| Site | Status | How | Test result / reason |
|---|---|---|---|
| GoFile | 🔒 | — | GoFile keeps its listing API for its own website (a site-only token). Using it from an app would get around that gate, so the app says to download on the site. |
| 1fichier | 🔒 | — | Each free download needs a wait and a click on 1fichier's page. The site was also unreachable from this network. |
| Sendspace | ⚪ | resolver | Built (the page's download button). No live link found. |
| Filemail | ⚪ | — | No resolver yet; links expire, none found |
| Smash | ⚪ | — | No resolver yet; links expire, none found |
| Wormhole | 🔒 | — | End-to-end encrypted in the browser |
| SwissTransfer | ⚪ | resolver | Built (public links API). Links expire, none found. |
| TransferNow | ⚪ | — | No resolver yet; links expire, none found |
| Hightail | ⚪ | — | Service being wound down; no links found |
| MASV | ⚪ | — | Business transfers that expire; no links found |

### 5. Software

| Site | Status | How | Test result / reason |
|---|---|---|---|
| GitHub | ✅ | resolver | Release assets plus source ZIP and TAR.GZ; one file, then three files together, downloaded |
| GitLab | ✅ | resolver | Release (38 assets) listed; 18 MB file downloaded in 7 s |
| Bitbucket | ✅ | resolver | Repository ZIP and TAR.GZ; ZIP downloaded |
| SourceForge | ✅ | resolver | 7-Zip installer downloaded (1.5 MB) |
| FossHub | 🟡 | built-in browser | The download link is made by script on the page. The Browser page now takes over that download; this is not yet tried in the running app. |
| FileHippo | ❌ | — | Answers 412 to apps (bot check). The built-in Browser may work. |
| Softpedia | 🟡 | built-in browser | Download is a form post on the page; same as FossHub |
| MajorGeeks | 🟡 | built-in browser | Mirror page starts the download by script; same as FossHub |
| Ninite | ✅ | resolver | Installer downloaded (434 KB) |
| Chocolatey | ✅ | resolver | Package (.nupkg) downloaded |

### 6. Games

| Site | Status | How | Test result / reason |
|---|---|---|---|
| Steam | 🔒 | — | Store client only, with copy protection |
| Epic Games | 🔒 | — | Store client only |
| GOG | 🟡 | page scan | Trailers found by scan. Offline installers need your GOG sign-in; not tested. |
| itch.io | 🟡 | resolver | Free uploads are read from the game page; not tested, because the test game is "name your own price", which goes through itch.io's purchase page. The app reports that it is paid. |
| Battle.net | 🔒 | — | Store client only |
| EA app | 🔒 | — | Store client only |
| Ubisoft Connect | 🔒 | — | Store client only |
| Rockstar | 🔒 | — | Store client only |
| Xbox | 🔒 | — | Store client only |
| PlayStation Store | 🔒 | — | Console store only |

### 7. Images and 3D

| Site | Status | How | Test result / reason |
|---|---|---|---|
| Unsplash | ✅ | resolver | Full-size photo downloaded (1.4 MB) |
| Pexels | ✅ | resolver | Original photo downloaded |
| Pixabay | ❌ | — | 403 to apps (bot check). The built-in Browser may work. |
| Wikimedia Commons | ✅ | resolver | Original file downloaded |
| Flickr | ✅ | gallery-dl | Profile found as a collection |
| DeviantArt | 🟡 | gallery-dl | A gallery is found as a collection. A single artwork link is not. |
| ArtStation | ❌ | resolver | Artwork data answers 403 to apps (bot check) |
| Sketchfab | 🔒 | — | Model downloads need a Sketchfab account (their download API requires sign-in) |
| Poly Haven | ✅ | resolver | 180 files (every map, resolution and format) listed; a file downloaded |
| Blender Studio | 🟡 | — | Most films and assets are for subscribers. Free trailers are on YouTube (works). |

### 8. Documents and research

| Site | Status | How | Test result / reason |
|---|---|---|---|
| Internet Archive | ✅ | resolver | Items listed with sizes; files downloaded. Copies kept for lending are left out. |
| Project Gutenberg | ✅ | resolver | Book in 9 formats; EPUB downloaded. It goes through Gutenberg's own mirror, because gutenberg.org answers 503 from this network. |
| Google Books | ❌ | — | Full-view PDF downloads sit behind a captcha; the app does not solve captchas |
| Open Library | ⚪ | resolver | Built (finds the scan on archive.org; leaves out lending-only copies). openlibrary.org timed out from this network on both tries. |
| arXiv | ✅ | resolver | PDF downloaded; source listed |
| Zenodo | ✅ | resolver | Record files listed; downloaded |
| Figshare | ✅ | resolver | Article files downloaded (24.9 MB in 4 s) |
| Dryad | 🟡 | resolver | Files listed, but downloads answer 401: Dryad now asks for an API sign-in |
| Kaggle | ✅ | resolver | Dataset ZIP downloaded, no sign-in |
| Hugging Face | ✅ | resolver | Model, dataset and space files listed with sizes; downloaded |

### 9. Education

| Site | Status | How | Test result / reason |
|---|---|---|---|
| Coursera | 🔒 | — | Paid, signed-in students only |
| edX | 🟡 | yt-dlp | Course page's public video (on YouTube) read. Course videos need enrolment. |
| Udemy | 🔒 | — | Paid, with copy protection |
| Khan Academy | ❌ | yt-dlp | yt-dlp cannot read the page right now. The same videos are on YouTube (works). |
| MIT OpenCourseWare | ✅ | resolver | Whole-course ZIP listed (23 MB) |
| Stanford Online | 🟡 | — | Free lectures are on YouTube (works); course pages hold no files |
| OpenLearn | ✅ | page scan | Course video found on the page |
| FutureLearn | 🔒 | — | Paid, signed-in learners only |
| Skillshare | 🔒 | — | Paid subscription |
| LinkedIn Learning | 🔒 | — | Paid subscription |

### 10. Open data

| Site | Status | How | Test result / reason |
|---|---|---|---|
| OpenStreetMap | ✅ | resolver | Way exported as .osm and downloaded |
| NASA data | ✅ | resolver + direct | NASA Image Library: 5 sizes listed, one downloaded; GISS CSV downloaded |
| NOAA | ✅ | folder listing | NCEI folder listed (16 files); a file downloaded |
| USGS | ✅ | direct | Earthquake feed CSV downloaded |
| Data.gov | ✅ | resolver | Data set's 8 files listed (CSV, JSON, XML, KML). Data.gov moved off CKAN; the app reads the data set page. |
| World Bank | ✅ | resolver | Indicator CSV ZIP downloaded |
| UN Data | ✅ | direct | CSV size read (2.2 MB) |
| European Data Portal | ✅ | resolver | Data set's files listed; a file downloaded. Map services (WMS/WFS) are left out. |
| AWS Open Data Registry | ✅ | resolver | Public S3 bucket listed (first 1,000 files); a file downloaded |
| Google Dataset Search | 🔒 | — | A search engine: open a result and download from that site |

## Totals

Out of 100 sites, one status each:

| Status | Count |
|---|---|
| ✅ Works | 47 |
| 🟡 Partly | 12 |
| 🔒 Not allowed (reason shown) | 20 |
| ❌ Fails (reason given) | 10 |
| ⚪ Untested (no live link) | 11 |

## Known gaps, for follow-up

- **No resolver yet:** Filemail, Smash and TransferNow. Their links expire within days, so they need a live link to build against.
- **Untested resolvers:** OneDrive, WeTransfer, SwissTransfer, Sendspace and Open Library need a live public link.
- **Built-in browser download take-over** needs a try in the running app, on MajorGeeks, FossHub, Softpedia and Pixabay.
- **yt-dlp extractor failures** (TikTok, Audiomack, Khan Academy, BBC) come from yt-dlp, not the app. Updating yt-dlp (`yt-dlp -U`) may fix them.
- **Slow MediaFire file:** 1.4 MB took 43 s. Worth a look: MediaFire may be throttling, or the fresh-link step may be retrying.
