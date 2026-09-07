/** Matches an "@" mention token ending at the cursor, e.g. the "Doc" in "check @Doc". */
export const AT_TRIGGER_PATTERN = /(?<=^|\s)@([^\s]*)$/;

/** Matches a `/` command token ending at the cursor, including after existing text. */
export const SLASH_TRIGGER_PATTERN = /(?<=^|\s)\/([^\s]*)$/;

/** Matches a slash token anywhere before the caret for inline variable insertion. */
export const INLINE_SLASH_TRIGGER_PATTERN = /\/([^\s/]*)$/;

export type SlashQueryMode = "command" | "inline";

export interface ComposerQueryState {
  isBlank: boolean;
  slashQuery: string | null;
  atQuery: string | null;
  atTriggerIndex: number | null;
}

export const EMPTY_COMPOSER_QUERY: ComposerQueryState = {
  isBlank: true,
  slashQuery: null,
  atQuery: null,
  atTriggerIndex: null,
};

/** Derives slash/@ menu state from textarea-like plain text around the caret. */
export function queryStateFromText(
  text: string,
  textBeforeCursor: string,
  slashQueryMode: SlashQueryMode = "command",
): ComposerQueryState {
  const atMatch = textBeforeCursor.match(AT_TRIGGER_PATTERN);
  const slashMatch = textBeforeCursor.match(
    slashQueryMode === "inline"
      ? INLINE_SLASH_TRIGGER_PATTERN
      : SLASH_TRIGGER_PATTERN,
  );
  return {
    isBlank: text.trim().length === 0,
    slashQuery: slashMatch?.[1] ?? null,
    atQuery: atMatch?.[1] ?? null,
    atTriggerIndex: atMatch !== null ? (atMatch.index ?? null) : null,
  };
}

export function queryStatesEqual(
  left: ComposerQueryState,
  right: ComposerQueryState,
): boolean {
  return (
    left.isBlank === right.isBlank &&
    left.slashQuery === right.slashQuery &&
    left.atQuery === right.atQuery &&
    left.atTriggerIndex === right.atTriggerIndex
  );
}
