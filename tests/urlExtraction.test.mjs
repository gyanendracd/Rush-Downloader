// Tests of the link finder used by Batch Download (src/urlExtraction.ts).
// Run with `npm test` (Node's own test runner; Node reads the TypeScript
// file directly, so nothing has to be built first).
import { test } from "node:test";
import assert from "node:assert/strict";
import { extractUrls, findLinks, toCsv, toTxt, trimLinkEnd, isValidLink } from "../src/urlExtraction.ts";

const links = (text, options) => findLinks(text, options).links;

test("the example from the request: exactly three links, random text ignored, queries kept", () => {
  const text =
    "https://www.youtube.com/live/wFNzXwueV1w?si=CH2UvxTnnZ6O0cO8 dfsdf sdf fs rerwe r https://youtu.be/g9K-sdtqeR4?si=GAQF3coOyfqytOLn dfsd frf 2wer https://youtu.be/tLFUd_aQrGM?si=6IcrfFr7Z-xte6hg das dsa d";
  const result = extractUrls([{ text, name: null }]);
  assert.deepEqual(
    result.urls.map((u) => u.url),
    [
      "https://www.youtube.com/live/wFNzXwueV1w?si=CH2UvxTnnZ6O0cO8",
      "https://youtu.be/g9K-sdtqeR4?si=GAQF3coOyfqytOLn",
      "https://youtu.be/tLFUd_aQrGM?si=6IcrfFr7Z-xte6hg",
    ],
  );
  assert.equal(result.discovered, 3);
  assert.equal(result.duplicates, 0);
  assert.equal(result.invalid, 0);
});

test("random text between links", () => {
  assert.deepEqual(
    links("abc https://example.com/file1.zip random text https://example.com/file2.mp4 another random string https://example.com/file3.pdf"),
    ["https://example.com/file1.zip", "https://example.com/file2.mp4", "https://example.com/file3.pdf"],
  );
});

test("links run together with no space are cut where the second one starts", () => {
  assert.deepEqual(links("https://example.com/file1.ziphttps://example.com/file2.zip"), [
    "https://example.com/file1.zip",
    "https://example.com/file2.zip",
  ]);
  assert.deepEqual(links("https://a.example.com/x,https://b.example.com/y;https://c.example.com/z"), [
    "https://a.example.com/x",
    "https://b.example.com/y",
    "https://c.example.com/z",
  ]);
});

test("an address carried inside a link is not cut out of it", () => {
  assert.deepEqual(links("https://web.archive.org/web/2020/https://example.com/page"), [
    "https://web.archive.org/web/2020/https://example.com/page",
  ]);
  assert.deepEqual(links("go https://example.com/redirect?to=https://other.example.org/x&y=1 now"), [
    "https://example.com/redirect?to=https://other.example.org/x&y=1",
  ]);
  assert.deepEqual(links("https://example.com/r?u=https%3A%2F%2Fother.example.org%2Fx"), [
    "https://example.com/r?u=https%3A%2F%2Fother.example.org%2Fx",
  ]);
});

test("JSON, escaped JSON and JSON Lines", () => {
  const json = `{
"title": "My downloads",
"url": "https://example.com/video.mp4",
"items": [
"https://example.com/file1.zip",
"https://example.com/file2.zip"
]
}`;
  assert.deepEqual(links(json), ["https://example.com/video.mp4", "https://example.com/file1.zip", "https://example.com/file2.zip"]);
  assert.deepEqual(links(String.raw`{"u":"https:\/\/example.com\/a.mp4?x=1&y=2"}`), ["https://example.com/a.mp4?x=1&y=2"]);
  assert.deepEqual(links('{"a":"https://e.example.com/1"}\n{"a":"https://e.example.com/2"}'), [
    "https://e.example.com/1",
    "https://e.example.com/2",
  ]);
});

test("HTML and XML, entities undone", () => {
  const html = `<a href="https://example.com/file1.zip">Download 1</a>
<a href="https://example.com/file2.zip">Download 2</a>
<a href='https://example.com/q?a=1&amp;b=2'>x</a><img src=https://example.com/i.png>`;
  assert.deepEqual(links(html), [
    "https://example.com/file1.zip",
    "https://example.com/file2.zip",
    "https://example.com/q?a=1&b=2",
    "https://example.com/i.png",
  ]);
  const xspf = `<?xml version="1.0"?><playlist><trackList><track><location>https://cdn.example.com/a%20b.mp3</location></track></trackList></playlist>`;
  assert.deepEqual(links(xspf), ["https://cdn.example.com/a%20b.mp3"]);
});

test("mixed prose, numbered lists, trailing punctuation", () => {
  const text = `Download these files:

1. https://example.com/file1.zip
Some additional information here.
https://example.com/file2.zip?token=123
Another random description.
https://example.com/file3.zip.`;
  assert.deepEqual(links(text), [
    "https://example.com/file1.zip",
    "https://example.com/file2.zip?token=123",
    "https://example.com/file3.zip",
  ]);
  assert.deepEqual(links("Did you see https://example.com/a?, or (https://example.com/b)!"), [
    "https://example.com/a",
    "https://example.com/b",
  ]);
});

test("brackets: Markdown, Wikipedia-style parentheses, angle brackets", () => {
  assert.deepEqual(links("[https://example.com/a](https://example.com/b) and <https://example.com/c>"), [
    "https://example.com/a",
    "https://example.com/b",
    "https://example.com/c",
  ]);
  assert.deepEqual(links("see https://en.wikipedia.org/wiki/Foo_(bar) here"), ["https://en.wikipedia.org/wiki/Foo_(bar)"]);
  assert.deepEqual(links("[text](https://en.wikipedia.org/wiki/Foo_(bar))"), ["https://en.wikipedia.org/wiki/Foo_(bar)"]);
  assert.deepEqual(links("**https://example.com/bold**"), ["https://example.com/bold"]);
});

test("fragments, ports, IP addresses, IPv6 and non-ASCII are kept as written", () => {
  assert.deepEqual(links("https://example.com/page#section-2 http://localhost:8080/x http://192.168.1.10/f.iso http://[::1]:3000/a"), [
    "https://example.com/page#section-2",
    "http://localhost:8080/x",
    "http://192.168.1.10/f.iso",
    "http://[::1]:3000/a",
  ]);
  assert.deepEqual(links("«https://ru.wikipedia.org/wiki/Москва»"), ["https://ru.wikipedia.org/wiki/Москва"]);
  // the address is given back exactly: not lowercased, not re-encoded
  assert.deepEqual(links("HTTPS://Example.COM/A%2FB?Q=1"), ["HTTPS://Example.COM/A%2FB?Q=1"]);
});

test("CSV: fields with and without quotes", () => {
  const csv = 'id,url,title\n1,https://example.com/a.zip,First\n2,"https://example.com/b.zip?x=1,2",Second\nhttps://example.com/c.zip,Third\n';
  assert.deepEqual(extractUrls([{ text: csv, name: "list.csv" }]).urls.map((u) => u.url), [
    "https://example.com/a.zip",
    "https://example.com/b.zip?x=1,2",
    "https://example.com/c.zip",
  ]);
});

test("M3U playlists, RTF and INI", () => {
  const m3u = "#EXTM3U\n#EXTINF:123,Song\nhttps://radio.example.com/stream.mp3\n#EXTINF:-1,Live\nhttp://tv.example.com/live/index.m3u8?token=abc\n";
  assert.deepEqual(links(m3u), ["https://radio.example.com/stream.mp3", "http://tv.example.com/live/index.m3u8?token=abc"]);
  assert.deepEqual(links(String.raw`{\rtf1 {\field{\*\fldinst{HYPERLINK "https://example.com/doc.pdf"}}}https://example.com/x\par}`), [
    "https://example.com/doc.pdf",
    "https://example.com/x",
  ]);
  assert.deepEqual(links("[mirror]\nurl=https://example.com/a.iso\n"), ["https://example.com/a.iso"]);
});

test("things that only look like links are counted as invalid, not listed", () => {
  const found = findLinks("https:// http://nodots/x https://example..com https://123.456 https://-/ ftp://example.com/a mailto:a@b.test");
  assert.deepEqual(found.links, []);
  assert.equal(found.invalid, 5);
  assert.ok(isValidLink("https://xn--80ak6aa92e.com/"));
  assert.ok(!isValidLink("javascript:alert(1)"));
});

test("duplicates are removed, order of first discovery kept, sources remembered", () => {
  const result = extractUrls([
    { text: "https://example.com/1 https://example.com/2", name: "a.txt" },
    { text: "https://example.com/2 https://example.com/3 https://example.com/1", name: "b.json" },
  ]);
  assert.deepEqual(result.urls.map((u) => u.url), ["https://example.com/1", "https://example.com/2", "https://example.com/3"]);
  assert.equal(result.discovered, 5);
  assert.equal(result.duplicates, 2);
  assert.equal(result.urls[1].source, "a.txt");
  assert.deepEqual(result.urls[1].sources, ["a.txt", "b.json"]);
  assert.equal(result.urls[0].count, 2);
});

test("hundreds of links on one line, and a large input, quickly", () => {
  const line = Array.from({ length: 500 }, (_, i) => `x${i} https://example.com/f${i}.bin?k=${i}`).join(" ");
  assert.equal(links(line).length, 500);
  const big = Array.from({ length: 200_000 }, (_, i) => `row ${i} https://cdn.example.com/${i % 50_000}.jpg`).join("\n");
  const started = Date.now();
  const result = extractUrls([{ text: big, name: "big.log" }]);
  assert.equal(result.urls.length, 50_000);
  assert.equal(result.duplicates, 150_000);
  assert.ok(Date.now() - started < 5000, "took too long");
});

test("trailing punctuation is trimmed with care", () => {
  assert.equal(trimLinkEnd("https://example.com/a)."), "https://example.com/a");
  assert.equal(trimLinkEnd("https://example.com/a_(b)"), "https://example.com/a_(b)");
  assert.equal(trimLinkEnd("https://example.com/a?"), "https://example.com/a");
  assert.equal(trimLinkEnd("https://example.com/a#"), "https://example.com/a#");
});

test("exports: one link per line, and CSV with Index, URL, Source File", () => {
  assert.equal(toTxt(["https://a.test/1", "https://a.test/2"]), "https://a.test/1\nhttps://a.test/2\n");
  assert.equal(toTxt([]), "");
  assert.equal(
    toCsv([
      { url: "https://a.test/1?x=1,2", source: "my list.csv" },
      { url: "https://a.test/2", source: null },
    ]),
    'Index,URL,Source File\r\n1,"https://a.test/1?x=1,2",my list.csv\r\n2,https://a.test/2,\r\n',
  );
});
