import type { TFunction } from "i18next";
import {
  activeLocale,
  translationResources,
  type Locale,
  type TranslationKey,
} from "./i18n-instance";

/** Looks up `key` in one fixed locale, so missing English copy cannot fall back to Chinese. */
function translateKnown(
  t: TFunction,
  key: string,
  fallback: TranslationKey,
  locale: Locale,
): string {
  if (key in translationResources[locale]) {
    return t(key as TranslationKey, { lng: locale });
  }
  return t(fallback, { lng: locale });
}

/** Localizes one stable skill-import error code into a user-facing sentence. */
export function localizeSkillImportReason(
  code: string | null | undefined,
  t: TFunction,
): string {
  const locale = activeLocale();
  if (code) {
    const importKey = `settings.skills.importReason.${code}`;
    if (importKey in translationResources[locale]) {
      return t(importKey as TranslationKey, { lng: locale });
    }
    const errorKey = `errors.${code}`;
    if (errorKey in translationResources[locale]) {
      return t(errorKey as TranslationKey, { lng: locale });
    }
  }
  return t("settings.skills.importReason.unknown", { lng: locale });
}

/** Localizes a candidate, result, or session status label. */
export function localizeSkillImportStatus(
  status: string,
  t: TFunction,
): string {
  const locale = activeLocale();
  return translateKnown(
    t,
    `settings.skills.importStatus.${status}`,
    "settings.skills.importStatus.unknown",
    locale,
  );
}

/** Explains why one committed candidate did not succeed, or `null` when it did. */
export function localizeSkillImportResultReason(
  result: { status: string; errorCode: string | null },
  t: TFunction,
): string | null {
  if (result.status === "failed")
    return localizeSkillImportReason(result.errorCode, t);
  if (result.status === "staleconflict") {
    const locale = activeLocale();
    return t("settings.skills.importReason.stale_conflict", { lng: locale });
  }
  return null;
}
