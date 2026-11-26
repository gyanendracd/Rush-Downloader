"""Local test site for RushDownload: range-capable, throttled, nothing leaves this machine."""
import re
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MB = 1024 * 1024


def blob(size, seed):
    out = bytearray(size)
    x = seed
    for i in range(0, size, 4096):
        x = (x * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        chunk = x.to_bytes(8, "little") * 512
        out[i:i + 4096] = chunk[: min(4096, size - i)]
    return bytes(out)


FILES = {
    "/files/big-video.mkv": (blob(48 * MB, 1), "video/x-matroska", 600_000),   # bytes/s per connection
    "/files/small-archive.zip": (blob(2 * MB, 2), "application/zip", 0),
    "/files/report.pdf": (blob(300_000, 3), "application/pdf", 0),
    "/media/clip.mp4": (blob(1 * MB, 4), "video/mp4", 0),
    "/media/song.mp3": (blob(400_000, 5), "audio/mpeg", 0),
    "/batch/one.bin": (blob(200_000, 6), "application/octet-stream", 0),
    "/batch/two.bin": (blob(250_000, 7), "application/octet-stream", 0),
}

PAGE = b"""<!doctype html><html><head><title>Test page</title></head><body>
<video src="/media/clip.mp4"></video>
<a href="/media/song.mp3">song</a>
<a href="/files/report.pdf">report</a>
<a href="/other.html">another page</a>
</body></html>"""


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/page.html":
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(PAGE)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(PAGE)
            return
        if path not in FILES:
            body = b"not found"
            self.send_response(404)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)
            return

        data, ctype, rate = FILES[path]
        start, end = 0, len(data) - 1
        m = re.match(r"bytes=(\d+)-(\d*)", self.headers.get("Range", ""))
        if m:
            start = int(m.group(1))
            end = min(int(m.group(2)) if m.group(2) else end, end)
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        else:
            self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", '"v1"')
        self.send_header("Connection", "close")
        self.end_headers()
        try:
            step = 32 * 1024
            for i in range(start, end + 1, step):
                self.wfile.write(data[i:min(i + step, end + 1)])
                if rate:
                    time.sleep(step / rate)
        except (BrokenPipeError, ConnectionError, OSError):
            pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
