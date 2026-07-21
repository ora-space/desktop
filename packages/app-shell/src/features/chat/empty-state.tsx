import { useTranslation } from "react-i18next";
import type { TranslationKey } from "../../i18n/i18n";
import type { ChatSuggestion } from "../../lib/types";

const SUGGESTIONS: TranslationKey[] = [
  "chat.suggestion.runtime",
  "chat.suggestion.layout",
  "chat.suggestion.worktree",
  "chat.suggestion.testing",
];

interface LandingSuggestionsProps {
  onSend: (text: string) => void;
  isResponding: boolean;
  disabled: boolean;
  suggestions: readonly ChatSuggestion[];
}

/**
 * The landing copy shown above the composer while a session has no messages.
 * It is deliberately separate from the composer so ChatView can keep the
 * composer mounted across the empty/thread switch and animate it between them.
 */
export function LandingHeading() {
  const { t } = useTranslation();
  return (
    <div className="mb-7">
      <h1 className="text-2xl font-medium tracking-[-0.035em] text-foreground sm:text-[28px]">{t("chat.heading")}</h1>
      <p className="mt-2 text-sm text-muted-foreground">{t("chat.subheading")}</p>
    </div>
  );
}

/** Starter prompts shown under the landing composer. */
export function LandingSuggestions({ onSend, isResponding, disabled, suggestions }: LandingSuggestionsProps) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  return (
    <div className="mt-3 flex flex-wrap gap-2">
      {SUGGESTIONS.map((suggestionKey) => {
        const suggestion = t(suggestionKey);
        return (
          <button
            key={suggestionKey}
            type="button"
            disabled={isResponding || disabled}
            onClick={() => onSend(suggestion)}
            className="min-h-9 rounded-lg border border-border bg-background px-3 py-2 text-left text-xs text-muted-foreground outline-none transition-colors duration-150 hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-background disabled:hover:text-muted-foreground"
          >
            {suggestion}
          </button>
        );
      })}
      {suggestions.map((suggestion) => {
        const text = suggestion.text[locale];
        return (
          <button
            key={suggestion.id}
            type="button"
            disabled={isResponding || disabled}
            onClick={() => onSend(text)}
            className="min-h-9 rounded-lg border border-border bg-background px-3 py-2 text-left text-xs text-muted-foreground outline-none transition-colors duration-150 hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-background disabled:hover:text-muted-foreground"
          >
            {text}
          </button>
        );
      })}
    </div>
  );
}
