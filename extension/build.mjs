// Builds the browser extension for the stores, with no tools beyond Node:
//
//   node extension/build.mjs          production: dist-extension/chromium, firefox, and a .zip of each
//   node extension/build.mjs --dev    development builds, named "(Dev)", beside them
//
// One source tree serves every browser. extension/manifest.json is the
// development manifest (load the extension folder unpacked); the store
// manifests are made here, with the version taken from package.json so the
// extension and the app always carry the same number.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync, copyFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateRawSync } from "node:zlib";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");

/** What goes into a package: nothing else in the folder (this script, the README) does. */
export const FILES = [
  "background.js",
  "shared.js",
  "scan.js",
  "popup.html",
  "popup.js",
  "popup.css",
  "options.html",
  "options.js",
  "options.css",
  "icons/16.png",
  "icons/32.png",
  "icons/48.png",
  "icons/64.png",
  "icons/128.png",
];

const ICONS = { 16: "icons/16.png", 32: "icons/32.png", 48: "icons/48.png", 128: "icons/128.png" };

/** The manifest of one browser's package. */
export function manifestFor(browser, version, dev) {
  const base = JSON.parse(readFileSync(join(here, "manifest.json"), "utf8"));
  const m = {
    manifest_version: 3,
    name: dev ? "RushDownload (Dev)" : "RushDownload",
    version,
    description: base.description,
    permissions: base.permissions,
    host_permissions: base.host_permissions,
    action: { ...base.action, default_icon: ICONS },
    options_ui: base.options_ui,
    icons: ICONS,
  };
  if (browser === "chromium") {
    m.minimum_chrome_version = base.minimum_chrome_version;
    m.background = { service_worker: "background.js" };
  } else if (browser === "firefox") {
    // Firefox runs background scripts as an event page, loading both files.
    m.background = { scripts: ["shared.js", "background.js"] };
    m.browser_specific_settings = {
      gecko: {
        id: dev ? "rushdownload-dev@rushdownload.app" : "rushdownload@rushdownload.app",
        strict_min_version: "121.0",
        // Nothing is collected: links go only to the app on this computer.
        data_collection_permissions: { required: ["none"] },
      },
    };
  } else {
    throw new Error(`unknown browser ${browser}`);
  }
  return m;
}

/* ---------- a small zip writer (deflate, as the stores take) ---------- */

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

export function crc32(data) {
  let c = 0xffffffff;
  for (const byte of data) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** A zip of `entries` ([name, Buffer]), with a fixed date so builds repeat byte for byte. */
export function zip(entries) {
  const parts = [];
  const central = [];
  let offset = 0;
  const DOS_TIME = 0;
  const DOS_DATE = (2026 - 1980) << 9 | 1 << 5 | 1;
  for (const [name, data] of entries) {
    const nameBuf = Buffer.from(name, "utf8");
    const packed = deflateRawSync(data, { level: 9 });
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6); // names in UTF-8
    local.writeUInt16LE(8, 8); // deflate
    local.writeUInt16LE(DOS_TIME, 10);
    local.writeUInt16LE(DOS_DATE, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    local.writeUInt16LE(0, 28);
    parts.push(local, nameBuf, packed);
    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02014b50, 0);
    entry.writeUInt16LE(20, 4);
    entry.writeUInt16LE(20, 6);
    entry.writeUInt16LE(0x0800, 8);
    entry.writeUInt16LE(8, 10);
    entry.writeUInt16LE(DOS_TIME, 12);
    entry.writeUInt16LE(DOS_DATE, 14);
    entry.writeUInt32LE(crc, 16);
    entry.writeUInt32LE(packed.length, 20);
    entry.writeUInt32LE(data.length, 24);
    entry.writeUInt16LE(nameBuf.length, 28);
    entry.writeUInt32LE(offset, 42);
    central.push(entry, nameBuf);
    offset += local.length + nameBuf.length + packed.length;
  }
  const centralBuf = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(centralBuf.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, centralBuf, end]);
}

/* ---------- the build ---------- */

function build(dev) {
  const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
  const app = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8")).version;
  if (app !== version) console.warn(`warning: package.json says ${version} but tauri.conf.json says ${app}`);
  const out = join(root, "dist-extension");
  for (const browser of ["chromium", "firefox"]) {
    const name = dev ? `${browser}-dev` : browser;
    const dir = join(out, name);
    rmSync(dir, { recursive: true, force: true });
    const entries = [];
    for (const file of FILES) {
      const from = join(here, file);
      if (!existsSync(from)) throw new Error(`missing ${file}`);
      mkdirSync(dirname(join(dir, file)), { recursive: true });
      copyFileSync(from, join(dir, file));
      entries.push([file, readFileSync(from)]);
    }
    const manifest = Buffer.from(JSON.stringify(manifestFor(browser, version, dev), null, 2) + "\n");
    writeFileSync(join(dir, "manifest.json"), manifest);
    entries.unshift(["manifest.json", manifest]);
    const archive = zip(entries);
    const zipName = `rushdownload-${name}-${version}.zip`;
    writeFileSync(join(out, zipName), archive);
    const sha = createHash("sha256").update(archive).digest("hex");
    console.log(`${zipName}  ${archive.length} bytes  sha256 ${sha}`);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  build(process.argv.includes("--dev"));
}
