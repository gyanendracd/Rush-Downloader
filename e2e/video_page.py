"""A page that plays a video, for testing the Browser page's media catcher.

    python e2e/video_page.py <video file> [port]

/watch/demo   a page with a <video> that starts playing. It offers three
              qualities as <source> tags and a fourth only in its code, and,
              like real video sites, its code also carries clips of other
              videos, which the catcher must leave out.
/media/...    the video file, with Range support
"""
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

VIDEO = open(sys.argv[1], "rb").read()
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8766
PAGE = b"""<!doctype html><html><head><meta charset="utf-8"><title>Demo clip: the harbour at dawn</title></head>
<body style="font-family:sans-serif;background:#111;color:#eee">
<h1>Demo clip</h1>
<video width="480" controls autoplay muted loop>
  <source src="/media/harbour-at-dawn-720.mp4" type="video/mp4" label="720p">
  <source src="/media/harbour-at-dawn-480.mp4" type="video/mp4" label="480p">
  <source src="/media/harbour-at-dawn-360.mp4" type="video/mp4" size="360">
</video>
<script>
var player = {"sources": {"hd": "http:\\/\\/127.0.0.1:%d\\/media\\/harbour-at-dawn-1080.mp4?token=abc"}};
var related = [
  {"title": "Another clip", "preview": "http:\\/\\/127.0.0.1:%d\\/media\\/other-clip-240.mp4"},
  {"title": "A third clip", "preview": "http:\\/\\/127.0.0.1:%d\\/previews\\/third-clip_720p.webm"},
  {"title": "A fourth clip", "preview": "http:\\/\\/127.0.0.1:%d\\/previews\\/fourth-clip_1080p.mp4"}
];
</script>
<p>A test page. The video above is what a download manager should catch.</p>
</body></html>""" % (PORT, PORT, PORT, PORT)


# The same video on a page that builds its player in script, the way many
# sites do. Nothing in its markup names the files, so a site downloader that
# only reads the page finds nothing; the video has to be caught as it plays.
PLAY = b"""<!doctype html><html><head><meta charset="utf-8"><title>Demo clip: the harbour at dawn</title></head>
<body style="font-family:sans-serif;background:#111;color:#eee">
<h1>Demo clip</h1>
<div id="stage"></div>
<script>
var base = "/med" + "ia/harbour-at-dawn-";
var video = document.createElement("video");
video.width = 480; video.controls = true; video.autoplay = true; video.muted = true; video.loop = true;
[["720", "720p"], ["480", "480p"], ["360", "360p"], ["1080", "1080p"]].forEach(function (q) {
  var source = document.createElement("source");
  source.src = base + q[0] + ".mp" + "4";
  source.type = "video/mp" + "4";
  source.setAttribute("label", q[1]);
  video.appendChild(source);
});
document.getElementById("stage").appendChild(video);
</script>
<p>A test page whose player is built by script.</p>
</body></html>"""


# A page whose player loads nothing until its big play button is pressed, the
# way many players wait. The app has to press play by itself to catch it.
CLICK = b"""<!doctype html><html><head><meta charset="utf-8"><title>Demo clip: the harbour at dawn</title></head>
<body style="font-family:sans-serif;background:#111;color:#eee">
<h1>Demo clip</h1>
<div id="stage" style="position:relative;width:480px;height:270px;background:#222">
  <button class="vjs-big-play-button" style="position:absolute;left:200px;top:105px;width:80px;height:60px">Play</button>
</div>
<script>
document.querySelector(".vjs-big-play-button").addEventListener("click", function () {
  this.remove();
  var base = "/med" + "ia/harbour-at-dawn-";
  var video = document.createElement("video");
  video.width = 480; video.controls = true; video.muted = true; video.loop = true;
  [["720", "720p"], ["480", "480p"]].forEach(function (q) {
    var source = document.createElement("source");
    source.src = base + q[0] + ".mp" + "4";
    source.type = "video/mp" + "4";
    source.setAttribute("label", q[1]);
    video.appendChild(source);
  });
  document.getElementById("stage").appendChild(video);
  video.play();
});
</script>
<p>A test page whose player waits for its play button.</p>
</body></html>"""


# A page whose video is a stream: a playlist fetched from an address that
# does not look like one (no .m3u8, served as plain text), and small pieces.
# One piece is fetched as a small .mp4, as fragmented streams do; the app
# must offer the stream's qualities and leave the piece out.
STREAM = b"""<!doctype html><html><head><meta charset="utf-8"><title>Demo clip: the harbour at dawn</title>
<meta property="og:image" content="/thumb.png"></head>
<body style="font-family:sans-serif;background:#111;color:#eee">
<h1>Demo clip</h1>
<script>
var xhr = new XMLHttpRequest();
xhr.open("GET", "/hls/key=abc123/play");
xhr.onload = function () {
  fetch("/hls/v720.m3u8").then(function (r) { return r.text(); });
  fetch("/media/seg-1-v1-a1.mp4").then(function (r) { return r.arrayBuffer(); });
};
xhr.send();
</script>
<p>A test page whose video is a stream.</p>
</body></html>"""
HLS_DIR = sys.argv[3] if len(sys.argv) > 3 else None


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_HEAD(self):
        self.respond(body=False)

    def do_GET(self):
        self.respond(body=True)

    def respond(self, body):
        if self.path.startswith("/media/") or self.path.startswith("/previews/"):
            data, start, status = VIDEO, 0, 200
            end = len(data) - 1
            rng = self.headers.get("Range", "")
            if rng.startswith("bytes="):
                a, _, b = rng[6:].partition("-")
                start = int(a or 0)
                end = min(int(b), end) if b else end
                status = 206
            chunk = data[start : end + 1]
            self.send_response(status)
            self.send_header("Content-Type", "video/mp4")
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(len(chunk)))
            if status == 206:
                self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
            self.end_headers()
            if body:
                try:
                    self.wfile.write(chunk)
                except OSError:
                    pass
        elif self.path == "/thumb.png":
            # A one-pixel picture, standing in for a video's thumbnail.
            import base64

            png = base64.b64decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="
            )
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(png)))
            self.end_headers()
            if body:
                self.wfile.write(png)
        elif self.path == "/catch.js":
            # The app's own catcher script, so it can be tried in any browser.
            import os

            here = os.path.dirname(os.path.abspath(__file__))
            script = open(os.path.join(here, "..", "src-tauri", "src", "catch_media.js"), "rb").read()
            self.send_response(200)
            self.send_header("Content-Type", "text/javascript; charset=utf-8")
            self.send_header("Content-Length", str(len(script)))
            self.end_headers()
            if body:
                self.wfile.write(script)
        elif self.path.startswith("/hls/") and HLS_DIR:
            import os

            name = self.path.split("?")[0].rsplit("/", 1)[-1]
            # The list of qualities lives at an address that gives nothing away.
            listing = name == "play"
            target = os.path.join(HLS_DIR, "master.m3u8" if listing else os.path.basename(name))
            if not os.path.isfile(target):
                self.send_response(404)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            data = open(target, "rb").read()
            kind = "text/plain" if listing else "application/vnd.apple.mpegurl" if name.endswith(".m3u8") else "video/mp2t"
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            if body:
                self.wfile.write(data)
        elif self.path.startswith(("/watch/", "/play/", "/click/", "/stream/")):
            page = (
                PAGE
                if self.path.startswith("/watch/")
                else PLAY
                if self.path.startswith("/play/")
                else CLICK
                if self.path.startswith("/click/")
                else STREAM
            )
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(page)))
            self.end_headers()
            if body:
                self.wfile.write(page)
        else:
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
