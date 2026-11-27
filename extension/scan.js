// Run in the page (and each frame inside it) when the popup opens. It reads
// the whole page for video and audio addresses and hands the list back.
// Nothing else is read from the page, and nothing is changed on it.
(() => {
  const MEDIA = /\.(mp4|m4v|webm|mov|mkv|flv|m3u8|mpd|mp3|m4a|aac|ogg|opus|wav|flac)(?:[?#]|$)/i;
  const AUDIO = ["mp3", "m4a", "aac", "ogg", "opus", "wav", "flac"];
  // A media address written anywhere in the page's own code, slashes escaped or not.
  const IN_SOURCE =
    /https?:(?:\\?\/){2}[^"'\s<>()\\]+?(?:\\?\/[^"'\s<>()\\]+?)*\.(?:mp4|m4v|webm|m3u8|mpd)(?:\?[^"'\s<>()\\]*)?/gi;
  // Players of other sites placed in the page; the app fetches these by their own address.
  const EMBED =
    /^https?:\/\/(?:[a-z0-9-]+\.)*(?:youtube(?:-nocookie)?\.com\/embed\/|player\.vimeo\.com\/video\/|dailymotion\.com\/(?:embed\/|player)|player\.twitch\.tv\/|streamable\.com\/[eo]\/|rumble\.com\/embed\/|ok\.ru\/videoembed\/|facebook\.com\/plugins\/video)/i;
  const LIMIT = 150;
  const found = new Map();
  // A player that builds its video in pieces shows no address of its own.
  let hidden = false;

  function quality(text) {
    const hit = /(?:^|[^0-9])(2160|1440|1080|720|540|480|360|240|144)(?:p|[^0-9]|$)/i.exec(String(text || ""));
    if (hit) return hit[1] + "p";
    return /(?:^|[^a-z0-9])4k(?:[^a-z0-9]|$)/i.test(String(text || "")) ? "2160p" : "";
  }

  // `from`: "player" (set on a video or audio element), "loaded" (the page
  // fetched it), "link" (a link on the page), "embed", or "code" (only
  // written in the page's code). The strongest one seen is kept.
  const RANK = { player: 0, loaded: 1, embed: 2, link: 3, code: 4 };
  function add(address, from, hint, label) {
    let url;
    try {
      url = new URL(address, location.href).href;
    } catch (_) {
      return;
    }
    if (!/^https?:/i.test(url) || url.length > 4000) return;
    const ext = ((MEDIA.exec(url) || [])[1] || "").toLowerCase();
    let kind;
    if (from === "embed") kind = "embed";
    else if (ext === "m3u8" || ext === "mpd") kind = "stream";
    else if (hint === "audio" || AUDIO.includes(ext)) kind = "audio";
    else if (ext || hint === "video") kind = "video";
    else return;
    const known = found.get(url);
    if (known) {
      if (RANK[from] < RANK[known.from]) known.from = from;
      if (!known.label) known.label = quality(label);
      return;
    }
    if (found.size >= LIMIT) return;
    found.set(url, { url, kind, from, label: quality(label) || quality(url), referer: location.href });
  }

  // Players that set their file on the element, and every quality listed as a <source>.
  for (const el of document.querySelectorAll("video, audio, video source, audio source")) {
    const hint = el.closest("audio") ? "audio" : "video";
    const label = ["label", "res", "size", "data-quality", "data-res", "title"].map((n) => el.getAttribute(n)).find(Boolean);
    for (const address of [el.currentSrc, el.src, el.getAttribute("src")]) {
      if (!address) continue;
      if (/^blob:/i.test(address)) hidden = true;
      else if (!/^data:/i.test(address)) add(address, "player", hint, label);
    }
  }

  // What the page has loaded so far, as the browser lists it.
  try {
    for (const entry of performance.getEntriesByType("resource")) {
      const by = entry.initiatorType;
      // Video a player fetches by script comes in pieces, which are no use
      // alone; only a stream's playlist is taken from those.
      const piece = (by === "fetch" || by === "xmlhttprequest") && !/\.(m3u8|mpd)(?:[?#]|$)/i.test(entry.name);
      if (!piece) add(entry.name, "loaded", by === "video" || by === "audio" ? by : "", "");
    }
  } catch (_) {}

  // The video a page names as its own for shared links.
  for (const name of ["og:video", "og:video:url", "og:video:secure_url", "og:audio", "twitter:player:stream"]) {
    for (const meta of document.querySelectorAll(`meta[property="${name}"], meta[name="${name}"]`)) {
      const address = meta.getAttribute("content");
      if (address && MEDIA.test(address)) add(address, "link", "", "");
    }
  }

  // Links to media files anywhere on the page.
  for (const link of document.querySelectorAll("a[href]")) {
    const address = link.getAttribute("href");
    if (address && MEDIA.test(address)) add(address, "link", "", link.textContent.slice(0, 60));
  }

  // Players of other sites.
  for (const frame of document.querySelectorAll("iframe[src], embed[src]")) {
    const address = frame.src || frame.getAttribute("src");
    if (address && EMBED.test(address)) add(address, "embed", "", "");
  }

  // Addresses a player keeps in the page's code and loads only when asked.
  const html = document.documentElement ? document.documentElement.innerHTML : "";
  if (html.length < 4000000) {
    let count = 0;
    for (const match of html.matchAll(IN_SOURCE)) {
      add(match[0].replace(/\\\//g, "/").replace(/&amp;/g, "&"), "code", "", "");
      if (++count > 300) break;
    }
  }

  return { items: [...found.values()], hidden };
})();
