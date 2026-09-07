import { createInstance } from "i18next";
import { initReactI18next } from "react-i18next";

import { translationResources } from "./resources";
import type { Locale } from "./resource-bundle";

export type { Locale } from "./resource-bundle";
export { translationResources } from "./resources";

export type TranslationKey = keyof (typeof translationResources)["zh-CN"];
const LOCALE_STORAGE_KEY = "ora.locale";

/** Reads the persisted locale without making application startup depend on browser storage availability. */
function readInitialLocale(): Locale {
  if (typeof window === "undefined") return "zh-CN";
  try {
    return window.localStorage.getItem(LOCALE_STORAGE_KEY) === "en-US"
      ? "en-US"
      : "zh-CN";
  } catch {
    return "zh-CN";
  }
}

export const appI18n = createInstance();
const initialLocale = readInitialLocale();

void appI18n.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: translationResources["zh-CN"] },
    "en-US": { translation: translationResources["en-US"] },
  },
  lng: initialLocale,
  fallbackLng: "zh-CN",
  supportedLngs: ["zh-CN", "en-US"],
  keySeparator: false,
  interpolation: { escapeValue: false },
  initAsync: false,
  showSupportNotice: false,
});

if (typeof document !== "undefined")
  document.documentElement.lang = initialLocale;

/** Returns the locale currently applied by the app i18n instance. */
export function activeLocale(): Locale {
  return appI18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
}
appI18n.on("languageChanged", (language) => {
  const locale: Locale = language === "en-US" ? "en-US" : "zh-CN";
  if (typeof document !== "undefined") document.documentElement.lang = locale;
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
    } catch {
      // Storage is an enhancement; language switching still works for the current runtime.
    }
  }
});
