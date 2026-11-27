import type { CSSProperties } from "react";
import {
  AlignLeft,
  Archive,
  BookOpen,
  Braces,
  Clapperboard,
  CodeXml,
  Cog,
  Database,
  Disc3,
  Feather,
  File as FileGlyph,
  Grid3x3,
  Hash,
  Image as ImageGlyph,
  Magnet,
  Music,
  Package,
  PenTool,
  Presentation,
  Sheet,
  Smartphone,
  Smile,
  Terminal,
  Type,
  Users,
  type LucideIcon,
} from "lucide-react";
import { categoryOf, type Category, type Download } from "./api";
import { t } from "./i18n";

/** How a kind of file is drawn: its colour and the mark on the page. */
interface Look {
  tone: string;
  glyph: LucideIcon;
}

const GREEN = "#16a34a";
const RED = "#ef4444";
const BLUE = "#2563eb";
const NAVY = "#1e40af";
const ORANGE = "#f59e0b";
const PURPLE = "#9333ea";
const ROSE = "#e11d48";
const PINK = "#db2777";
const TEAL = "#0d9488";
const SLATE = "#64748b";

/** By file extension, the way file managers tell types apart. */
const BY_EXT: Record<string, Look> = {};
function looks(exts: string, look: Look) {
  for (const ext of exts.split(" ")) BY_EXT[ext] = look;
}
looks("csv xls xlsx ods tsv", { tone: GREEN, glyph: Sheet });
looks("pdf", { tone: RED, glyph: AlignLeft });
looks("doc docx odt rtf txt md", { tone: BLUE, glyph: AlignLeft });
looks("ppt pptx odp key", { tone: ORANGE, glyph: Presentation });
looks("epub mobi azw3", { tone: TEAL, glyph: BookOpen });
looks("zip rar 7z tar gz bz2 xz zst tgz", { tone: PURPLE, glyph: Archive });
looks("iso img dmg", { tone: BLUE, glyph: Disc3 });
looks("exe msi msix bat cmd", { tone: BLUE, glyph: Cog });
looks("apk aab", { tone: GREEN, glyph: Smartphone });
looks("pkg deb rpm appimage", { tone: SLATE, glyph: Package });
looks("png bmp tiff tif", { tone: NAVY, glyph: Grid3x3 });
looks("jpg jpeg jfif webp avif heic", { tone: GREEN, glyph: ImageGlyph });
looks("gif", { tone: ORANGE, glyph: Smile });
looks("svg ai eps", { tone: ORANGE, glyph: PenTool });
looks("psd xcf", { tone: NAVY, glyph: Feather });
looks("mp4 mkv avi mov webm m4v flv wmv mpg mpeg 3gp ts", { tone: ROSE, glyph: Clapperboard });
looks("mp3 wav flac aac m4a ogg opus wma", { tone: PINK, glyph: Music });
looks("html htm", { tone: GREEN, glyph: CodeXml });
looks("xml", { tone: PURPLE, glyph: CodeXml });
looks("js ts jsx tsx java py rs go c cpp cs json", { tone: RED, glyph: Braces });
looks("css scss", { tone: BLUE, glyph: Hash });
looks("sh ps1", { tone: SLATE, glyph: Terminal });
looks("ttf otf woff woff2", { tone: SLATE, glyph: Type });
looks("sql db sqlite pbf osm", { tone: TEAL, glyph: Database });

/** For a file whose extension says nothing, by what kind of file it is. */
const BY_CATEGORY: Record<Category, Look> = {
  video: { tone: ROSE, glyph: Clapperboard },
  audio: { tone: PINK, glyph: Music },
  image: { tone: GREEN, glyph: ImageGlyph },
  document: { tone: BLUE, glyph: AlignLeft },
  archive: { tone: PURPLE, glyph: Archive },
  software: { tone: BLUE, glyph: Cog },
  other: { tone: SLATE, glyph: FileGlyph },
};

function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  return /^[a-z0-9]{1,8}$/.test(ext) ? ext : "";
}

/** Short forms for extensions too long for the badge. */
const SHORT: Record<string, string> = { appimage: "APP", torrent: "TOR" };

/** ".PDF" for a short extension; one too long loses its dot or is cut, so
 *  the badge never holds more than five characters. */
function badgeText(ext: string): string {
  const shown = (SHORT[ext] ?? ext).toUpperCase();
  return shown.length <= 4 ? `.${shown}` : shown.slice(0, 5);
}

/** The colour, mark and badge text a download's file is drawn with. */
export function docLook(d: Download): Look & { label: string } {
  if (d.profile) return { tone: SLATE, glyph: Users, label: t.common.badge.profile };
  const ext = extensionOf(d.filename);
  if (d.torrent && !BY_EXT[ext]) return { tone: TEAL, glyph: Magnet, label: ext ? badgeText(ext) : t.common.badge.torrent };
  const category = categoryOf(d);
  const look = BY_EXT[ext] ?? BY_CATEGORY[category];
  return { ...look, label: ext ? badgeText(ext) : t.common.badge[category] };
}

/**
 * A sheet of paper with its corner folded over in the file type's colour,
 * the type's mark on it, and the extension on a badge across its foot. It
 * fills whatever box it is put in, so the places that show a smaller icon
 * size it as before.
 */
export function DocIcon({ d, large }: { d: Download; large?: boolean }) {
  const { tone, glyph: Glyph, label } = docLook(d);
  // Long words get a smaller badge font so they still fit.
  const long = label.length > 5 ? " is-longer" : label.length > 4 ? " is-long" : "";
  return (
    <span
      className={`file-icon file-doc${large ? " file-icon-lg" : ""}`}
      style={{ "--tone": tone } as CSSProperties}
      aria-hidden="true"
    >
      <svg className="doc-page" viewBox="0 0 48 48" focusable="false">
        {/* the page, its top-right corner cut away */}
        <path
          className="doc-sheet"
          d="M10 1.5h23L43 11.5V42a4.5 4.5 0 0 1-4.5 4.5H10A4.5 4.5 0 0 1 5.5 42V6A4.5 4.5 0 0 1 10 1.5Z"
        />
        {/* the folded-over corner */}
        <path className="doc-fold" d="M33 1.5 43 11.5h-7a3 3 0 0 1-3-3Z" />
      </svg>
      <Glyph className="doc-glyph" strokeWidth={2.25} />
      <span className={`doc-badge${long}`}>{label}</span>
    </span>
  );
}
