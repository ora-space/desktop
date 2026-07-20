import { Composer } from "./composer";
import { useTranslation } from "react-i18next";
import type { TranslationKey } from "../../i18n/i18n";

interface EmptyStateProps {
  onSend: (text: string) => void;
  isResponding: boolean;
  error: string | null;
  disabled: boolean;
}

const SUGGESTIONS: TranslationKey[] = [
  "chat.suggestion.runtime",
  "chat.suggestion.layout",
  "chat.suggestion.worktree",
  "chat.suggestion.testing",
];

/** The centered landing view shown when no conversation is selected. */
export function EmptyState({ onSend, isResponding, error, disabled }: EmptyStateProps) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-1 items-center justify-center overflow-y-auto px-3 pb-[12vh] pt-10 sm:px-6">
      <div className="w-full max-w-[760px]">
        <div className="mb-7">
          <h1 className="text-2xl font-medium tracking-[-0.035em] text-foreground sm:text-[28px]">{t("chat.heading")}</h1>
          <p className="mt-2 text-sm text-muted-foreground">{t("chat.subheading")}</p>
        </div>
        {error && <p role="alert" className="mb-2 px-1 text-xs text-destructive">{error}</p>}
        <Composer autoFocus onSend={onSend} isResponding={isResponding} disabled={disabled} />
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
        </div>
      </div>
    </div>
  );
}
