/* The interface's languages.
 *
 * Text is read from `t`, the dictionary of the language switched on now:
 *   import { t } from "./i18n";
 *   <h1>{t.views.home.title}</h1>
 *   t.common.deletedAgo.days(3)
 * `t` is a live binding, so it always holds the current language; the app
 * re-renders from the top when the language changes (see App.tsx).
 *
 * English is built in. Every other language is its own file in ./locales,
 * typed as `Dict`, so a missing key fails the build, and loaded on first use. */
import { en, type Dict } from "./en";
import { applyFormatLocale } from "./format";

export type { Dict };
export { dateText, formatLocale, listText, num, pct, plural } from "./format";

/** The languages on offer, each named in itself. */
export const LOCALES = [
  { code: "en", name: "English", english: "English" },
  { code: "hi", name: "हिन्दी", english: "Hindi" },
  { code: "es", name: "Español", english: "Spanish" },
  { code: "fr", name: "Français", english: "French" },
  { code: "de", name: "Deutsch", english: "German" },
  { code: "pt-BR", name: "Português (Brasil)", english: "Portuguese" },
  { code: "ja", name: "日本語", english: "Japanese" },
  { code: "zh-CN", name: "简体中文", english: "Chinese" },
  { code: "ru", name: "Русский", english: "Russian" },
  { code: "ko", name: "한국어", english: "Korean" },
  { code: "ar", name: "العربية", english: "Arabic" },
  { code: "id", name: "Bahasa Indonesia", english: "Indonesian" },
] as const;

export type Locale = (typeof LOCALES)[number]["code"];

const CODES = LOCALES.map((l) => l.code) as Locale[];

const LOADERS: Record<Exclude<Locale, "en">, () => Promise<{ default: Dict }>> = {
  hi: () => import("./locales/hi"),
  es: () => import("./locales/es"),
  fr: () => import("./locales/fr"),
  de: () => import("./locales/de"),
  "pt-BR": () => import("./locales/pt-BR"),
  ja: () => import("./locales/ja"),
  "zh-CN": () => import("./locales/zh-CN"),
  ru: () => import("./locales/ru"),
  ko: () => import("./locales/ko"),
  ar: () => import("./locales/ar"),
  id: () => import("./locales/id"),
};

/** The text of the language switched on now. */
export let t: Dict = en;
/** The language switched on now. */
export let locale: Locale = "en";

const RTL: readonly Locale[] = ["ar"];

/** The first of the system's languages the app has, else English. */
export function systemLocale(): Locale {
  const wanted = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of wanted) {
    if (!tag) continue;
    const lower = tag.toLowerCase();
    const exact = CODES.find((c) => c.toLowerCase() === lower);
    if (exact) return exact;
    const base = lower.split("-")[0];
    // "pt-PT" or "zh-TW" still read Brazilian Portuguese or Simplified Chinese
    // better than English.
    const near = CODES.find((c) => c.toLowerCase().split("-")[0] === base);
    if (near) return near;
  }
  return "en";
}

/** The language picked in Settings, or the system's on first run. */
export function savedLocale(): Locale {
  // The same store as loadPref/savePref in api.ts, read here so that this
  // file does not import api.ts, which imports it.
  try {
    const saved = localStorage.getItem("language") as Locale | null;
    if (saved && CODES.includes(saved)) return saved;
  } catch {
    /* storage unavailable */
  }
  return systemLocale();
}

/** Switches the language: loads its text, sets the document's language and
 *  direction. Remembers the choice when `remember` is set. */
export async function setLocale(code: Locale, remember = false): Promise<void> {
  const dict = code === "en" ? en : (await LOADERS[code]()).default;
  t = dict;
  locale = code;
  applyFormatLocale(code);
  const root = document.documentElement;
  root.lang = code;
  root.dir = RTL.includes(code) ? "rtl" : "ltr";
  for (const listener of listeners) listener();
  if (remember) {
    try {
      localStorage.setItem("language", code);
    } catch {
      /* storage unavailable */
    }
  }
}

/** Whether the language switched on now is written right to left. */
export const isRtl = () => RTL.includes(locale);

const listeners = new Set<() => void>();

/** Calls back after each change of language; returns the way to stop. */
export function onLocaleChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** "Change language", always with the English words too, so someone who
 *  picked a language they cannot read by mistake can still find the way back. */
export function changeLanguageLabel(): string {
  const own = t.common.changeLanguage;
  return own === en.common.changeLanguage ? own : `${own} · ${en.common.changeLanguage}`;
}

/** A language's name in the picker: its own name, and its English name
 *  where that differs ("हिन्दी · Hindi"). */
export function languageOptionLabel(l: (typeof LOCALES)[number]): string {
  return l.name === l.english ? l.name : `${l.name} · ${l.english}`;
}

/** "Change language" in English, whatever language is switched on. */
export const ENGLISH_CHANGE_LANGUAGE = en.common.changeLanguage;
