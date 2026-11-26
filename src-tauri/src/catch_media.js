// Runs in every page shown on the Browser page. It notices the video and audio
// files a page loads or offers, the way a download manager's browser add-on
// does, and tells the app their addresses. It reads nothing else from the page.
(() => {
  if (window.__rushCatch) return;
  window.__rushCatch = true;
  const MARK = "RUSHDL::MEDIA::";
  const PAGE_MARK = "RUSHDL::PAGE::";
  const EXT = "mp4|m4v|webm|mov|mkv|flv|m3u8|mpd|mp3|m4a|aac|ogg|opus|wav|flac";
  const MEDIA = new RegExp("\\.(" + EXT + ")(?:[?#]|$)", "i");
  // A media address written anywhere in the page's own code, slashes escaped or not.
  const IN_SOURCE = new RegExp("https?:(?:\\\\?/){2}[^\"'\\s<>()\\\\]+?(?:\\\\?/[^\"'\\s<>()\\\\]+?)*\\.(?:mp4|m4v|webm|m3u8|mpd)(?:\\?[^\"'\\s<>()\\\\]*)?", "gi");
  const QUALITY = /(2160|1440|1080|720|540|480|360|240|144)p?/g;
  const seen = new Set();
  // What the page really plays or loads, by its address with the quality
  // taken out, and what was only found written in the page's code.
  const playing = new Set();
  const written = new Map();
  // Streams already told to the app, and how much was known about each.
  const streams = new Map();
  const waiting = [];
  let busy = false;

  // The top page passes each find to the app through the page title, the one
  // channel a page has; frames inside it hand their finds to the top page.
  function pump() {
    if (busy || waiting.length === 0) return;
    busy = true;
    const real = document.title;
    const next = waiting.shift();
    document.title = (next.__page ? PAGE_MARK : MARK) + JSON.stringify(next);
    setTimeout(() => {
      document.title = real;
      setTimeout(() => {
        busy = false;
        pump();
      }, 40);
    }, 60);
  }

  function send(item) {
    if (window.top === window) {
      waiting.push(item);
      pump();
    } else {
      try {
        window.top.postMessage({ __rushMedia: item }, "*");
      } catch (_) {}
    }
  }

  // "1080p" from a label the page gives, or from the address itself.
  function quality(text) {
    const found = /(?:^|[^0-9])(2160|1440|1080|720|540|480|360|240|144)(?:p|[^0-9]|$)/i.exec(String(text || ""));
    if (found) return found[1] + "p";
    return /(?:^|[^a-z0-9])4k(?:[^a-z0-9]|$)/i.test(String(text || "")) ? "2160p" : "";
  }

  // The same video in another quality has the same address once the quality
  // and the file type are taken out of it.
  function family(url) {
    try {
      return new URL(url).pathname.toLowerCase().replace(QUALITY, "#").replace(/\.[a-z0-9]+$/, "");
    } catch (_) {
      return "";
    }
  }

  // `fromCode`: only written in the page's code, not played or loaded. Pages
  // carry clips of many other videos there, so such a find counts only when
  // it is another quality of something the page really plays.
  function report(address, hint, label, fromCode) {
    let url;
    try {
      url = new URL(address, location.href).href;
    } catch (_) {
      return;
    }
    if (!/^https?:/i.test(url) || seen.has(url) || seen.size > 200) return;
    const ext = (MEDIA.exec(url) || [])[1];
    if (!ext && hint !== "video" && hint !== "audio") return;
    const lower = (ext || "").toLowerCase();
    if (lower === "m3u8" || lower === "mpd") {
      stream(url, false, []);
      return;
    }
    const kin = family(url);
    if (fromCode && !playing.has(kin)) {
      if (written.size < 300) written.set(url, label);
      return;
    }
    seen.add(url);
    written.delete(url);
    const kind =
      hint === "audio" || ["mp3", "m4a", "aac", "ogg", "opus", "wav", "flac"].includes(lower) ? "audio" : "video";
    send({ url, kind, referer: location.href, label: quality(label) || quality(url), heights: [], master: false });
    if (!fromCode && !playing.has(kin)) {
      playing.add(kin);
      // Other qualities of this one, seen earlier in the page's code.
      for (const [other, otherLabel] of [...written]) {
        if (family(other) === kin) report(other, "", otherLabel, true);
      }
    }
  }

  // A stream: a playlist that lists the video's pieces. `master` is the
  // playlist that lists every quality; `heights` are the picture sizes it
  // names. A stream is told again when more becomes known about it.
  function stream(address, master, heights) {
    let url;
    try {
      url = new URL(address, location.href).href;
    } catch (_) {
      return;
    }
    if (!/^https?:/i.test(url)) return;
    const known = streams.get(url);
    const worth = (master ? 2 : 0) + (heights.length > 0 ? 1 : 0);
    if ((known !== undefined && known >= worth) || streams.size > 60) return;
    streams.set(url, worth);
    seen.add(url);
    send({ url, kind: "stream", referer: location.href, label: "", heights: heights.slice(0, 12), master });
  }

  // Many players fetch their playlist from an address that does not look
  // like one, so a small text answer is read to see whether it is a
  // playlist (HLS starts with #EXTM3U, DASH with an <MPD> element).
  function playlist(address, text) {
    if (!text || text.length > 2000000) return;
    const start = text.slice(0, 3000);
    const sizes = [];
    const add = (value) => {
      const h = Number(value);
      if (h >= 100 && h <= 4320 && !sizes.includes(h)) sizes.push(h);
    };
    if (start.trimStart().startsWith("#EXTM3U")) {
      for (const m of text.matchAll(/RESOLUTION=\d+x(\d+)/gi)) add(m[1]);
      sizes.sort((a, b) => b - a);
      stream(address, /#EXT-X-STREAM-INF/i.test(text), sizes);
    } else if (/<MPD[\s>]/.test(start)) {
      for (const m of text.matchAll(/\bheight="(\d+)"/g)) add(m[1]);
      sizes.sort((a, b) => b - a);
      stream(address, true, sizes);
    }
  }

  // Whether an answer is worth reading: one that says it is a playlist, one
  // whose address says so, or a small text-like answer that might be one.
  function mayBePlaylist(address, type, length) {
    const kind = String(type || "").toLowerCase();
    if (kind.includes("mpegurl") || kind.includes("dash+xml")) return true;
    if (/\.(m3u8|mpd)(?:[?#]|$)/i.test(String(address || ""))) return true;
    const vague = kind === "" || kind.startsWith("text/") || kind.includes("octet-stream") || kind.includes("binary");
    return vague && length > 0 && length < 400000;
  }

  try {
    const nativeFetch = window.fetch;
    if (nativeFetch) {
      window.fetch = function () {
        const call = nativeFetch.apply(this, arguments);
        call.then(
          (answer) => {
            try {
              const length = Number(answer.headers.get("content-length") || 0);
              if (mayBePlaylist(answer.url, answer.headers.get("content-type"), length)) {
                answer
                  .clone()
                  .text()
                  .then((text) => playlist(answer.url, text))
                  .catch(() => {});
              }
            } catch (_) {}
          },
          () => {},
        );
        return call;
      };
    }
    const open = XMLHttpRequest.prototype.open;
    const sendRequest = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method, address) {
      this.__rushAddress = address;
      return open.apply(this, arguments);
    };
    XMLHttpRequest.prototype.send = function () {
      this.addEventListener("load", () => {
        try {
          const address = this.responseURL || this.__rushAddress;
          if (this.responseType === "" || this.responseType === "text") {
            const text = this.responseText || "";
            if (mayBePlaylist(address, this.getResponseHeader("content-type"), text.length)) playlist(address, text);
          } else if (this.responseType === "arraybuffer" && this.response) {
            const size = this.response.byteLength;
            if (mayBePlaylist(address, this.getResponseHeader("content-type"), size) && size < 2000000) {
              playlist(address, new TextDecoder().decode(this.response));
            }
          }
        } catch (_) {}
      });
      return sendRequest.apply(this, arguments);
    };
  } catch (_) {}

  // When the app is asked to fetch a page's video by itself, it starts the
  // player the way a visitor would: videos are played (muted), and a big
  // play button, where a player shows one first, is pressed.
  const PLAY_BUTTONS =
    ".vjs-big-play-button, .jw-icon-display, .jw-display-icon-container, .plyr__control--overlaid, " +
    "button[aria-label*='play' i], [class*='play-button' i], [class*='playbutton' i], " +
    "[class*='big-play' i], [class*='play_button' i], [class*='btn-play' i]";
  function startPlayer() {
    for (const video of document.querySelectorAll("video")) {
      try {
        video.muted = true;
        const started = video.play();
        if (started && started.catch) started.catch(() => {});
      } catch (_) {}
    }
    for (const button of document.querySelectorAll(PLAY_BUTTONS)) {
      const box = button.getBoundingClientRect();
      if (box.width >= 20 && box.height >= 20) {
        try {
          button.click();
        } catch (_) {}
        break;
      }
    }
  }

  window.addEventListener("message", (event) => {
    const data = event.data;
    if (!data) return;
    if (data.__rushPlay) {
      startPlayer();
      // Players often sit in a frame of their own.
      for (const frame of document.querySelectorAll("iframe")) {
        try {
          frame.contentWindow.postMessage({ __rushPlay: true }, "*");
        } catch (_) {}
      }
      return;
    }
    const item = data.__rushMedia;
    if (window.top === window && item && typeof item.url === "string" && typeof item.referer === "string") {
      waiting.push({
        url: item.url,
        kind: String(item.kind),
        referer: item.referer,
        label: String(item.label || ""),
        heights: Array.isArray(item.heights) ? item.heights.slice(0, 12).map(Number) : [],
        master: item.master === true,
      });
      pump();
    }
  });

  // Everything the page loads is listed by the browser itself.
  try {
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        report(entry.name, entry.initiatorType);
      }
    }).observe({ type: "resource", buffered: true });
  } catch (_) {}

  // The picture a page offers for itself (the one shared links show), so the
  // app can show it beside the video's name.
  let pictureTold = "";
  function picture() {
    if (window.top !== window) return;
    const from = (selector, attribute) => {
      const el = document.querySelector(selector);
      return el ? el.getAttribute(attribute) : "";
    };
    const found =
      from('meta[property="og:image"]', "content") ||
      from('meta[name="twitter:image"]', "content") ||
      from("video[poster]", "poster") ||
      from('link[rel="image_src"]', "href");
    if (!found) return;
    let url;
    try {
      url = new URL(found, location.href).href;
    } catch (_) {
      return;
    }
    if (url === pictureTold || !/^https?:/i.test(url)) return;
    pictureTold = url;
    waiting.push({ __page: true, thumbnail: url });
    pump();
  }

  function look() {
    picture();
    // Players that set their file directly on the video element, and every
    // quality a player lists as a <source>, loaded or not.
    for (const el of document.querySelectorAll("video, audio, video source, audio source")) {
      const hint = el.closest("audio") ? "audio" : "video";
      const label = ["label", "res", "size", "data-quality", "data-res", "title"]
        .map((name) => el.getAttribute(name))
        .find(Boolean);
      for (const address of [el.currentSrc, el.src, el.getAttribute("src")]) {
        if (address && !/^(blob|data):/i.test(address)) report(address, hint, label);
      }
    }
    // Qualities a player keeps in the page's own code and loads only when chosen.
    const html = document.documentElement ? document.documentElement.innerHTML : "";
    if (html.length < 3000000) {
      let count = 0;
      for (const match of html.matchAll(IN_SOURCE)) {
        report(match[0].replace(/\\\//g, "/").replace(/&amp;/g, "&"), "", "", true);
        if (++count > 200) break;
      }
    }
  }
  setInterval(look, 1500);
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", look);
  } else {
    look();
  }
})();
