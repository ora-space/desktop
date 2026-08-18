import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "@ora/ui";
import { localizeContractError } from "../../i18n/contract-error";

/** Matches `ora_domain::MAX_SESSION_TITLE_CHARS`; sidebar rename uses the same cap. */
export const MAX_INLINE_RENAME_CHARS = 255;

/**
 * Inline tree-row rename shared by sessions, projects, worktrees, and workflow runs.
 *
 * IME Enter/Escape must not commit, blur after a successful save must not save
 * again, and choosing Rename while already editing must keep the draft.
 */
export function useInlineTreeRename({
  value,
  onCommit,
}: {
  value: string;
  onCommit: (next: string) => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);
  const skipBlurCommit = useRef(false);
  const committingRef = useRef(false);
  const restoreMenuFocus = useRef(true);

  useEffect(() => {
    if (!renaming) {
      restoreMenuFocus.current = true;
      return;
    }
    // Ignore blurs caused by focusing the input (and any leftover menu close).
    skipBlurCommit.current = true;
    inputRef.current?.focus();
    inputRef.current?.select();
    const settle = window.setTimeout(() => {
      skipBlurCommit.current = false;
    }, 0);
    return () => window.clearTimeout(settle);
  }, [renaming]);

  /** Starts inline rename, or refocuses if the editor is already open. */
  function beginRename() {
    // The closing menu would restore focus to the trigger and blur-commit.
    restoreMenuFocus.current = false;
    if (renaming) {
      skipBlurCommit.current = true;
      inputRef.current?.focus();
      inputRef.current?.select();
      window.setTimeout(() => {
        skipBlurCommit.current = false;
      }, 0);
      return;
    }
    setDraft(value);
    setRenaming(true);
  }

  /** Cancels rename without persisting; blur must not treat this as a commit. */
  function cancelRename() {
    skipBlurCommit.current = true;
    setRenaming(false);
    setDraft(value);
  }

  /** Persists a non-empty trimmed name; identical values just exit edit mode. */
  async function commitRename() {
    if (committingRef.current) return;
    const next = draft.trim();
    if (next === "" || next === value) {
      cancelRename();
      return;
    }
    if ([...next].length > MAX_INLINE_RENAME_CHARS) {
      toast.error(t("sidebar.renameTooLong"));
      inputRef.current?.focus();
      return;
    }
    committingRef.current = true;
    try {
      await onCommit(next);
      // Unmounting a focused input fires blur; do not treat that as a second save.
      skipBlurCommit.current = true;
      setRenaming(false);
    } catch (cause) {
      toast.error(localizeContractError(cause, t));
      inputRef.current?.focus();
    } finally {
      committingRef.current = false;
    }
  }

  /** IME uses Enter/Escape to confirm or cancel composition, not the rename. */
  function onInputKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter") {
      event.preventDefault();
      void commitRename();
    } else if (event.key === "Escape") {
      event.preventDefault();
      cancelRename();
    }
  }

  /** Commits on blur unless Escape, a successful save, or focus settling skipped it. */
  function onInputBlur() {
    if (skipBlurCommit.current) {
      skipBlurCommit.current = false;
      return;
    }
    void commitRename();
  }

  return {
    renaming,
    draft,
    setDraft,
    inputRef,
    restoreMenuFocus,
    beginRename,
    onInputKeyDown,
    onInputBlur,
    maxLength: MAX_INLINE_RENAME_CHARS,
  };
}
