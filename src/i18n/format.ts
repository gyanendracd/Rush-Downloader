/* Locale-aware helpers the dictionaries and the interface build their text with.
 * They read the language that is switched on now (see `setLocale` in ./index). */

let locale = "en";
let plurals = new Intl.PluralRules("en");
let numbers = new Intl.NumberFormat("en");

/** Called by `setLocale` only. */
export function applyFormatLocale(code: string) {
  // Arabic keeps Western digits, as most software in Arabic does; the
  // percentages and sizes then read the same as in the progress bars.
  locale = code === "ar" ? "ar-u-nu-latn" : code;
  plurals = new Intl.PluralRules(locale);
  numbers = new Intl.NumberFormat(locale);
}

/** The BCP 47 tag the Intl formatters use now. */
export const formatLocale = () => locale;

/** The plural forms of one phrase. `#` is replaced by the count, formatted
 *  for the language. Give the forms the language has (English: one, other;
 *  Russian: one, few, many, other; Arabic: zero, one, two, few, many, other;
 *  Japanese, Chinese, Korean, Indonesian: other only). */
export type PluralForms = Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };

/** The right plural form of a phrase for `n`, with `#` replaced by `n`. */
export function plural(n: number, forms: PluralForms): string {
  const form = forms[plurals.select(n)] ?? forms.other;
  return form.replace(/#/g, num(n));
}

/** A whole or decimal number, written the language's way ("1,234", "1.234", "1 234"). */
export function num(n: number, digits?: number): string {
  if (digits == null) return numbers.format(n);
  return new Intl.NumberFormat(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(n);
}

/** A percentage from 0 to 100, written the language's way ("42%", "42 %",
 *  "%42"), with `digits` decimals ("0.312%"). */
export function pct(n: number, digits = 0): string {
  return new Intl.NumberFormat(locale, {
    style: "percent",
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(n / 100);
}

/** A date (and time), written the language's way. */
export function dateText(date: Date | number, options: Intl.DateTimeFormatOptions): string {
  return new Intl.DateTimeFormat(locale, options).format(date);
}

/** A list joined the language's way ("a, b and c"). */
export function listText(items: string[], type: "conjunction" | "disjunction" = "conjunction"): string {
  return new Intl.ListFormat(locale, { style: "long", type }).format(items);
}
