// The browser extension's rules for catching downloads, and its store
// packages (extension/shared.js, extension/build.mjs).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { inflateRawSync } from "node:zlib";
import vm from "node:vm";
import { FILES, crc32, manifestFor, zip } from "../extension/build.mjs";

// shared.js is a plain browser script; it is run here as one.
const sandbox = { module: { exports: {} }, URL, navigator: { userAgent: "Mozilla/5.0 Firefox/133.0" } };
vm.runInNewContext(readFileSync(new URL("../extension/shared.js", import.meta.url), "utf8"), sandbox);
const { rushShouldCatch, rushHostIn, rushTypeOf, rushBrowserName, RUSH_DEFAULTS } = sandbox.module.exports;

const item = (patch = {}) => ({ id: 1, url: "https://files.example.com/setup.exe", referrer: "https://example.com/download", totalBytes: 50_000_000, ...patch });
const settings = (patch = {}) => ({ ...RUSH_DEFAULTS, ...patch });

test("downloads the browser starts are caught unless the settings say otherwise", () => {
  assert.equal(rushShouldCatch(item(), settings(), "me"), true);
  assert.equal(rushShouldCatch(item(), settings({ capture: false }), "me"), false, "turned off");
  assert.equal(rushShouldCatch(item({ byExtensionId: "me" }), settings(), "me"), false, "Download with the browser");
  assert.equal(rushShouldCatch(item({ byExtensionId: "other" }), settings(), "me"), true);
  for (const url of ["blob:https://a.test/1", "data:text/plain,hi", "file:///C:/a.txt", "not a url"]) {
    assert.equal(rushShouldCatch(item({ url }), settings(), "me"), false, url);
  }
});

test("excluded sites, file types and small files stay in the browser", () => {
  const s = settings({ excludedSites: ["example.com"], excludedTypes: [".PDF", "jpg"], minSizeKb: 100 });
  assert.equal(rushShouldCatch(item(), s, "me"), false, "a file of an excluded site's subdomain");
  assert.equal(rushShouldCatch(item({ url: "https://cdn.test/a.zip", referrer: "https://www.example.com/" }), s, "me"), false, "started from an excluded site");
  assert.equal(rushShouldCatch(item({ url: "https://cdn.test/a.zip", referrer: "" }), s, "me"), true);
  assert.equal(rushShouldCatch(item({ url: "https://cdn.test/report.pdf?x=1", referrer: "" }), s, "me"), false, "type from the address");
  assert.equal(rushShouldCatch(item({ url: "https://cdn.test/get?id=1", filename: "C:\\Users\\me\\photo.JPG", referrer: "" }), s, "me"), false, "type from the name");
  assert.equal(rushShouldCatch(item({ url: "https://cdn.test/a.zip", referrer: "", totalBytes: 10_000 }), s, "me"), false, "smaller than 100 KB");
  assert.equal(rushShouldCatch(item({ url: "https://cdn.test/a.zip", referrer: "", totalBytes: 0, fileSize: -1 }), s, "me"), true, "size unknown: sent");
});

test("sites match whole names and file types are read from names", () => {
  assert.equal(rushHostIn("www.example.com", "example.com"), true);
  assert.equal(rushHostIn("example.com.", "*.example.com"), true);
  assert.equal(rushHostIn("badexample.com", "example.com"), false);
  assert.equal(rushTypeOf("https://a.test/dir/File.Tar.GZ?x=1#y"), "gz");
  assert.equal(rushTypeOf("https://a.test/download"), "");
  assert.equal(rushBrowserName(), "Firefox 133");
});

test("each browser gets a manifest of its own, with the app's version", () => {
  const version = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")).version;
  const chromium = manifestFor("chromium", version, false);
  assert.equal(chromium.name, "RushDownload");
  assert.equal(chromium.version, version);
  assert.deepEqual(chromium.background, { service_worker: "background.js" });
  assert.ok(chromium.permissions.includes("downloads") && chromium.permissions.includes("cookies"));
  assert.equal(chromium.browser_specific_settings, undefined);
  const firefox = manifestFor("firefox", version, false);
  assert.deepEqual(firefox.background, { scripts: ["shared.js", "background.js"] });
  assert.equal(firefox.browser_specific_settings.gecko.id, "rushdownload@rushdownload.app");
  assert.equal(firefox.minimum_chrome_version, undefined);
  const dev = manifestFor("firefox", version, true);
  assert.equal(dev.name, "RushDownload (Dev)");
  assert.notEqual(dev.browser_specific_settings.gecko.id, firefox.browser_specific_settings.gecko.id, "a development build installs beside the real one");
  for (const size of [16, 32, 48, 128]) assert.ok(FILES.includes(chromium.icons[size]), `icon ${size}`);
  assert.ok(!FILES.some((f) => f.endsWith(".mjs") || f.endsWith(".md")), "neither the build script nor notes are packaged");
});

test("the zip writer makes archives that unpack to what went in", () => {
  assert.equal(crc32(Buffer.from("123456789")), 0xcbf43926);
  const files = [["manifest.json", Buffer.from('{"a":1}')], ["icons/16.png", Buffer.alloc(300, 7)]];
  const archive = zip(files);
  // walk the local entries
  let at = 0;
  for (const [name, data] of files) {
    assert.equal(archive.readUInt32LE(at), 0x04034b50);
    const packedSize = archive.readUInt32LE(at + 18);
    const nameLen = archive.readUInt16LE(at + 26);
    assert.equal(archive.subarray(at + 30, at + 30 + nameLen).toString(), name);
    const body = archive.subarray(at + 30 + nameLen, at + 30 + nameLen + packedSize);
    assert.deepEqual(inflateRawSync(body), data);
    assert.equal(archive.readUInt32LE(at + 14), crc32(data));
    at += 30 + nameLen + packedSize;
  }
  assert.equal(archive.readUInt32LE(archive.length - 22), 0x06054b50);
  assert.deepEqual(zip(files), archive, "the same files give the same archive");
});
