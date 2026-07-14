import { OraMark } from "../../components/ora-mark";
import { Composer } from "./composer";

interface EmptyStateProps {
  onSend: (text: string) => void;
}

const SUGGESTIONS = [
  "Summarize the agent runtime refactor",
  "Draft a layout for the web client",
  "Explain how worktree cleanup works",
  "Outline a test plan for session attach",
];

/** The centered landing view shown when no conversation is selected. */
export function EmptyState({ onSend }: EmptyStateProps) {
  return (
    <div className="flex flex-1 items-center justify-center overflow-y-auto px-4 py-10">
      <div className="w-full max-w-2xl">
        <div className="mb-6 flex flex-col items-center text-center">
          <OraMark size="xl" className="mb-5" />
          <h1 className="text-display-sm font-semibold text-primary">How can I help you today?</h1>
          <p className="mt-2 text-md text-tertiary">Ask anything, or start from one of these.</p>
        </div>
        <Composer autoFocus onSend={onSend} isResponding={false} />
        <div className="mt-4 flex flex-wrap justify-center gap-2">
          {SUGGESTIONS.map((suggestion) => (
            <button
              key={suggestion}
              type="button"
              onClick={() => onSend(suggestion)}
              className="rounded-full border border-secondary bg-primary px-3 py-1.5 text-sm text-secondary transition duration-100 hover:bg-primary_hover hover:text-secondary_hover"
            >
              {suggestion}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
