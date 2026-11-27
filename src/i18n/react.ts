import { useSyncExternalStore } from "react";
import { locale, onLocaleChange, type Locale } from ".";

/** The language switched on now; the calling component renders again when it
 *  changes. The app's root calls it, so the whole interface follows. */
export function useLocale(): Locale {
  return useSyncExternalStore(onLocaleChange, () => locale);
}
