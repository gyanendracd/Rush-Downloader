# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

People who download a lot and are tired of juggling a browser downloader, a video-site downloader and a torrent client. Two kinds use the same app:

- Someone who pastes a link and wants the file, with no settings to think about.
- A power user who runs many downloads at once and wants speed, connections, segments, queue and filters available without leaving the window.

The app stays open all day, in a window or in the background, while downloads come and go. The user glances at it between other work.

## Product Purpose

One downloader for everything, so nobody needs a different tool per kind of link. Success is: paste any supported link, get the file as fast as the line allows, and always be able to see what is happening and act on it.

## Positioning

A single app whose own engine downloads direct files over many connections, and which hands web-page links (YouTube, Instagram, Vimeo and other sites) to a site backend, with one list, one queue and one set of controls for both.

## Operating Context

- Desktop app built with Tauri 2 (Rust engine, React UI rendered in the system webview). Windows first; macOS, Linux, iOS and Android are planned on the same engine.
- Links arrive by paste. A browser extension that sends links from the browser is planned.
- Typical session: several downloads running, more queued, a history of finished ones the user opens or shows in their folder.

## Capabilities and Constraints

Built and working:

- Multi-connection HTTP/HTTPS downloads with dynamic segment splitting, pause, resume across restarts, queue, global speed limit.
- Site downloads through yt-dlp and ffmpeg, with a resolution choice and audio-only (MP3).
- Page scan that lists video, audio and stream links found on a page.
- Per download: copy link, open file, show in folder, delete file, remove from list, retry.
- Settings: download folder, connections per download, downloads at once, speed limit.

Planned, not built: torrents, browser extension, mobile builds, a picker listing the real formats of each video.

Out of scope by decision: DRM-protected streaming services.

Terminology: "download" (one item), "connections", "segments", "queue", "site link" (a page handled by the site backend) versus "file link" (a direct file).

## Brand Commitments

- Name: RushDownload.
- User's stated direction for the interface: a left navigation bar with views and filters for downloads; simple to use yet with full functionality; minimal; must not look like a generic AI-generated app.

## Evidence on Hand

- Measured on the user's connection: about 11–13 MB/s with parallel connections against 2.9 MB/s for a single stream from the same server. This is one measurement on one line, not a general benchmark, and must not be presented as one.
- No logo, illustrations, testimonials or marketing copy exist. Do not invent any.

## Product Principles

1. Paste and go: the common case needs no decisions.
2. Nothing hidden that a power user needs; nothing shown that a casual user must understand.
3. Always truthful about state: what is running, what failed and why, what the server allows.
4. One list for every kind of download.
