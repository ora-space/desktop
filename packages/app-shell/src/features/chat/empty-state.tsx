import { OraMark } from "../../components/ora-mark";
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
    <div className="flex flex-1 items-center justify-center overflow-y-auto px-4 py-10">
      <div className="w-full max-w-2xl">
        <div className="mb-6 flex flex-col items-center text-center">
          <OraMark size="lg" className="mb-5" />
          <h1 className="text-2xl font-semibold text-foreground">{t("chat.heading")}</h1>
          <p className="mt-2 text-sm text-muted-foreground">{t("chat.subheading")}</p>
        </div>
        {error && <p role="alert" className="mb-2 text-xs text-destructive">{error}</p>}
        <Composer autoFocus onSend={onSend} isResponding={isResponding} disabled={disabled} />
        <div className="mt-4 flex flex-wrap justify-center gap-2">
          {SUGGESTIONS.map((suggestionKey) => {
            const suggestion = t(suggestionKey);
            return (
            <button
              key={suggestionKey}
              type="button"
              disabled={isResponding || disabled}
              onClick={() => onSend(suggestion)}
              className="rounded-full border border-border bg-background px-3 py-1.5 text-sm text-muted-foreground transition duration-100 hover:bg-accent hover:text-accent-foreground disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-background disabled:hover:text-muted-foreground"
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
