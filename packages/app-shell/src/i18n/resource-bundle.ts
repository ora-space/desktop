export type Locale = "zh-CN" | "en-US";

export type TranslationBundle = Readonly<
  Record<Locale, Readonly<Record<string, string>>>
>;

type TranslationKeys<Bundle> = Bundle extends TranslationBundle
  ? keyof Bundle["zh-CN"]
  : never;

const locales = ["zh-CN", "en-US"] as const;
const pluralSuffix = /_(zero|one|two|few|many|other)$/;

/** Compares logical keys while retaining each language's actual i18next plural forms. */
function logicalKeys(
  messages: Readonly<Record<string, string>>,
  locale: Locale,
) {
  const keys = new Set<string>();
  const categories = new Intl.PluralRules(locale).resolvedOptions()
    .pluralCategories;
  for (const key of Object.keys(messages)) {
    const suffix = key.match(pluralSuffix);
    const logical = suffix ? key.slice(0, -suffix[0].length) : key;
    if (suffix) {
      for (const category of categories) {
        if (!Object.hasOwn(messages, `${logical}_${category}`)) {
          throw new Error(
            `Missing ${locale} plural form: ${logical}_${category}`,
          );
        }
      }
    }
    keys.add(logical);
  }
  return [...keys].sort();
}

/** Combines explicit feature data once, rejecting missing translations and ambiguous ownership. */
export function composeTranslationResources<
  const Features extends Readonly<Record<string, TranslationBundle>>,
>(
  features: Features,
): Record<Locale, Record<TranslationKeys<Features[keyof Features]>, string>> {
  const combined: Record<Locale, Record<string, string>> = {
    "zh-CN": {},
    "en-US": {},
  };
  const owners = new Map<string, string>();
  for (const [owner, bundle] of Object.entries(features)) {
    const chinese = logicalKeys(bundle["zh-CN"], "zh-CN");
    const english = logicalKeys(bundle["en-US"], "en-US");
    if (
      chinese.length !== english.length ||
      chinese.some((key, index) => key !== english[index])
    ) {
      const missingChinese = english.filter((key) => !chinese.includes(key));
      const missingEnglish = chinese.filter((key) => !english.includes(key));
      throw new Error(
        `Translation keys differ in ${owner}: zh-CN missing [${missingChinese.join(", ")}]; en-US missing [${missingEnglish.join(", ")}]`,
      );
    }
    for (const key of chinese) {
      const previous = owners.get(key);
      if (previous !== undefined) {
        throw new Error(
          `Duplicate translation key ${key}: ${previous} and ${owner}`,
        );
      }
      owners.set(key, owner);
    }
    for (const locale of locales)
      Object.assign(combined[locale], bundle[locale]);
  }
  // The validated union is narrower than the dynamic merge's index signature, without maintaining
  // a second hand-written key catalog or erasing the consumer's literal TranslationKey type.
  return combined as Record<
    Locale,
    Record<TranslationKeys<Features[keyof Features]>, string>
  >;
}
