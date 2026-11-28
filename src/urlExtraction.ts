/* UrlExtractionService: finds every http(s) link in any text, whatever its
 * layout: one link per line or hundreds on one line, links run together,
 * links inside JSON, CSV, XML, HTML, Markdown, playlists or plain prose.
 *
 * It looks for the start of a link ("http://" or "https://") anywhere in the
 * text and reads on while the characters can belong to a link. Nothing
 * depends on spaces, commas, line breaks or the file's format. A link is
 * given back exactly as it was written (only the escapes of the format it
 * sat in are undone, as in "\/" in JSON or "&amp;" in HTML): it is never
 * re-encoded or "normalised", so it always leads where it led.
 *
 * Pure functions, no DOM: used from a worker (urlExtraction.worker.ts) for
 * large inputs, from the page for small ones, and from the tests. */

/** One link found, once, with where it was found. */
export interface ExtractedUrl {
  url: string;
  /** The first file it was found in; null for pasted text. */
  source: string | null;
  /** Every file it was found in, in order. */
  sources: string[];
  /** How many times it was found in all. */
  count: number;
}

export interface ExtractionStats {
  /** Valid links found, repeats included. */
  discovered: number;
  /** Links found more than once, counted for each extra time. */
  duplicates: number;
  /** Things that started like a link but are not one (no real host...). */
  invalid: number;
}

export interface ExtractionResult extends ExtractionStats {
  urls: ExtractedUrl[];
}

/** A piece of text to look through, and what to call where it came from. */
export interface ExtractionSource {
  text: string;
  /** A file's name; null for text typed or pasted in. */
  name: string | null;
}

/** Characters that may be part of a link: RFC 3986's unreserved and
 *  reserved characters and "%", plus any letter beyond ASCII (links are
 *  often written with their real letters, not %-escapes). */
function isLinkChar(code: number): boolean {
  if (code >= 0x61 && code <= 0x7a) return true; // a-z
  if (code >= 0x41 && code <= 0x5a) return true; // A-Z
  if (code >= 0x30 && code <= 0x39) return true; // 0-9
  switch (code) {
    case 0x2d: // -
    case 0x2e: // .
    case 0x5f: // _
    case 0x7e: // ~
    case 0x3a: // :
    case 0x2f: // /
    case 0x3f: // ?
    case 0x23: // #
    case 0x5b: // [
    case 0x5d: // ]
    case 0x40: // @
    case 0x21: // !
    case 0x24: // $
    case 0x26: // &
    case 0x27: // '
    case 0x28: // (
    case 0x29: // )
    case 0x2a: // *
    case 0x2b: // +
    case 0x2c: // ,
    case 0x3b: // ;
    case 0x3d: // =
    case 0x25: // %
      return true;
  }
  if (code <= 0x7f) return false;
  // Beyond ASCII: letters and marks belong to a link; spaces, quotes and
  // punctuation of other scripts end it.
  if (code === 0xa0 || code === 0x3000 || (code >= 0x2000 && code <= 0x206f)) return false; // spaces, “ ” ‘ ’ … and the like
  if (code === 0xab || code === 0xbb) return false; // « »
  if (code >= 0x3001 && code <= 0x3011) return false; // 、。〈〉《》「」『』【】
  if (code >= 0xff01 && code <= 0xff0f) return false; // ！＂＃…／ (full-width punctuation)
  if (code >= 0xff1a && code <= 0xff20) return false; // ：；＜＝＞？＠
  if (code === 0xfeff || code === 0xfffd) return false;
  return true;
}

/** Characters that end a sentence or wrap a link, and are dropped from its
 *  end; brackets only when the link does not open them itself. */
const TRAILING = new Set([".", ",", ";", ":", "!", "?", "'", "*", '"']);

/** What closes the bracket or quote a link was written inside. */
const CLOSER: Record<string, string> = {
  "(": ")",
  "[": "]",
  "{": "}",
  "<": ">",
  '"': '"',
  "'": "'",
  "`": "`",
  // a field of a list written with commas or semicolons ("1,https://…,title")
  ",": ",",
  ";": ";",
};

/** Openers whose closer ends the link at once (quotes and field separators). */
const FLAT = new Set(['"', "'", "`", ",", ";"]);

/** Characters after which "http" starts a second link run into the first
 *  ("…file1.ziphttps://…", "a.zip,https://…"). After "/", "=", "?", "&",
 *  ":" and the like it is the address a link carries inside itself
 *  ("…/web/2020/https://…", "?u=https://…"), and is left whole. */
function startsNewLink(prev: string): boolean {
  return /[A-Za-z0-9,;|)\]'"]/.test(prev);
}

const SCHEME = /https?:\/\//gi;

/** Undoes the escapes of the formats links are found in, without touching
 *  anything else: JSON's "\/" and "&", HTML and XML entities. */
export function unescapeFormats(text: string): string {
  let out = text;
  if (out.includes("\\")) {
    out = out
      .replace(/\\u([0-9a-fA-F]{4})/g, (whole, hex: string) => {
        const code = parseInt(hex, 16);
        // only what can be part of a link or end one; never a control character
        return code >= 0x20 ? String.fromCharCode(code) : whole;
      })
      .replace(/\\+\//g, "/");
  }
  if (out.includes("&")) {
    out = out.replace(/&(amp|quot|apos|lt|gt|#(\d{1,7})|#x([0-9a-fA-F]{1,6}));/gi, (whole, name: string, dec?: string, hex?: string) => {
      if (dec || hex) {
        const code = dec ? parseInt(dec, 10) : parseInt(hex!, 16);
        return code >= 0x20 && code <= 0x10ffff ? String.fromCodePoint(code) : whole;
      }
      switch (name.toLowerCase()) {
        case "amp":
          return "&";
        case "quot":
          return '"';
        case "apos":
          return "'";
        case "lt":
          return "<";
        case "gt":
          return ">";
      }
      return whole;
    });
  }
  return out;
}

/** Drops what ends a sentence or closes a bracket from the end of a link;
 *  a closing bracket stays when the link opened it ("…/Foo_(bar)"). */
export function trimLinkEnd(link: string): string {
  let end = link.length;
  for (;;) {
    if (end === 0) break;
    const last = link[end - 1];
    if (TRAILING.has(last)) {
      end--;
      continue;
    }
    if (last === ")" || last === "]") {
      const open = last === ")" ? "(" : "[";
      const body = link.slice(0, end);
      const opens = body.split(open).length - 1;
      const closes = body.split(last).length - 1;
      if (closes > opens) {
        end--;
        continue;
      }
    }
    break;
  }
  return link.slice(0, end);
}

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/;

/** Whether a found link is a real web address: it parses, is http(s), and
 *  names a host that can exist (a domain with a dot, localhost, or an IP). */
export function isValidLink(link: string): boolean {
  if (link.length > 32_768) return false;
  let parsed: URL;
  try {
    parsed = new URL(link);
  } catch {
    return false;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return false;
  const host = parsed.hostname;
  if (!host) return false;
  if (host === "localhost" || host.startsWith("[")) return true;
  // The host as written: the parser reads "123.456" as an IP address, but
  // nobody writes one that way.
  const written = /^https?:\/\/(?:[^@/?#]*@)?([^:/?#]*)/i.exec(link)?.[1] ?? "";
  if (/^[\d.]+$/.test(written)) {
    const ip = IPV4.exec(written);
    return !!ip && ip.slice(1).every((part) => Number(part) <= 255);
  }
  const labels = host.split(".");
  if (labels.length < 2 || labels.some((label) => label === "" || label.length > 63)) return false;
  const top = labels[labels.length - 1];
  // A top-level domain is letters (or an IDN's "xn--" form), never all digits.
  return /^(?:[a-z]{2,63}|xn--[a-z0-9-]{2,59})$/i.test(top);
}

/** The links in one text, in the order they are written, repeats included;
 *  `invalid` counts what started like a link but is not one. */
export function findLinks(raw: string, options: { commaEnds?: boolean } = {}): { links: string[]; invalid: number } {
  const text = unescapeFormats(raw);
  const links: string[] = [];
  let invalid = 0;
  // Each place a link starts, so one run into another can be cut there.
  const starts: number[] = [];
  SCHEME.lastIndex = 0;
  for (let m = SCHEME.exec(text); m; m = SCHEME.exec(text)) starts.push(m.index);

  let next = 0; // index into `starts`
  while (next < starts.length) {
    const start = starts[next++];
    // Written inside quotes or brackets: the closing one ends it (a bracket
    // only when the link has not opened one of its own).
    const opener = start > 0 ? text[start - 1] : "";
    const closer = CLOSER[opener];
    const quoted = !!closer && FLAT.has(opener);
    let depth = 0;
    const schemeEnd = text.indexOf("//", start) + 2;
    let end = schemeEnd;
    while (end < text.length) {
      const ch = text[end];
      if (!isLinkChar(text.charCodeAt(end))) break;
      // In a table (CSV), an unquoted comma ends the field.
      if (options.commaEnds && ch === "," && !quoted) break;
      if (closer && ch === closer) {
        if (quoted || depth === 0) break;
        depth--;
      } else if (closer && !quoted && ch === opener) {
        depth++;
      }
      // Another link starting here: run into this one, or carried inside it.
      if (next < starts.length && starts[next] === end) {
        if (startsNewLink(text[end - 1])) break;
        next++;
      }
      end++;
    }
    // Skip the starts of addresses this link carried inside itself.
    while (next < starts.length && starts[next] < end) next++;
    const whole = text.slice(start, end);
    // Inside quotes everything up to the closing quote is the link (an
    // attribute's value); elsewhere what ends a sentence is not part of it.
    const link = quoted && text[end] === closer && closer !== "," && closer !== ";" ? whole : trimLinkEnd(whole);
    if (link.length > schemeEnd - start && isValidLink(link)) links.push(link);
    else invalid++;
  }
  return { links, invalid };
}

/** Every link in the given texts, each once, in the order first found. */
export function extractUrls(sources: ExtractionSource[], onProgress?: (done: number) => void): ExtractionResult {
  const byUrl = new Map<string, ExtractedUrl>();
  const urls: ExtractedUrl[] = [];
  let discovered = 0;
  let invalid = 0;
  sources.forEach((source, i) => {
    // A table's fields are separated by commas, which never belong to its links.
    const found = findLinks(source.text, { commaEnds: /\.(csv|tsv)$/i.test(source.name ?? "") });
    invalid += found.invalid;
    for (const url of found.links) {
      discovered++;
      const known = byUrl.get(url);
      if (known) {
        known.count++;
        if (source.name && !known.sources.includes(source.name)) known.sources.push(source.name);
        continue;
      }
      const entry: ExtractedUrl = { url, source: source.name, sources: source.name ? [source.name] : [], count: 1 };
      byUrl.set(url, entry);
      urls.push(entry);
    }
    onProgress?.(i + 1);
  });
  return { urls, discovered, duplicates: discovered - urls.length, invalid };
}

/** The links as a text file: one per line, in order. */
export function toTxt(urls: string[]): string {
  return urls.length ? `${urls.join("\n")}\n` : "";
}

/** A CSV field, quoted where it has to be. */
function csvField(value: string): string {
  return /[",\r\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
}

/** The links as CSV: Index, URL, Source File. */
export function toCsv(rows: { url: string; source: string | null }[]): string {
  const lines = ["Index,URL,Source File", ...rows.map((r, i) => [String(i + 1), csvField(r.url), csvField(r.source ?? "")].join(","))];
  return `${lines.join("\r\n")}\r\n`;
}
