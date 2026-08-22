import { create } from "zustand";

export interface ComposerFileSelection {
  path: string;
  startLine: number;
  endLine: number;
  /** Line text captured at quote time for eager agent context. */
  snippet?: string;
  origin?: "diff";
  /** Present when every quoted line is the same side; mixed add/delete omits it. */
  diffSide?: "old" | "new";
}

type ComposerFileDelivery = (selections: ComposerFileSelection[]) => void;

interface ComposerFileContextState {
  /**
   * Quotes waiting because that conversation's composer is not mounted.
   * Bound composers receive quotes via `bindDelivery` and never re-read this
   * list, so a session switch cannot replay chips the user already deleted.
   */
  pendingByConversation: Record<string, ComposerFileSelection[] | undefined>;
  /** Queues a workspace-relative line range for one conversation's composer. */
  addSelection: (
    conversationKey: string,
    selection: ComposerFileSelection,
  ) => boolean;
  /**
   * Delivers quotes to the bound composer, or queues them until it binds.
   *
   * Returns false when every item is a duplicate of one already waiting —
   * either queued for an unmounted composer, or handed to a bound composer
   * within the current microtask. Re-quoting the same range later is always
   * accepted: once a chip is in the document the user may legitimately want a
   * second one, and the store cannot see what the document holds.
   */
  addSelections: (
    conversationKey: string,
    selections: ComposerFileSelection[],
  ) => boolean;
  /**
   * Registers the active composer's insert handler for `conversationKey`.
   * Drains any queued batches once, then every later quote goes to the
   * handler (not back into this list). Unbind on unmount / key change so a
   * stale composer cannot steal another session's quotes.
   */
  bindDelivery: (
    conversationKey: string,
    deliver: ComposerFileDelivery,
  ) => () => void;
}

/** Live insert handlers, keyed like `pendingByConversation`. Not in Zustand so binding does not re-render. */
const deliveries = new Map<string, ComposerFileDelivery>();

/**
 * Quotes handed to a bound composer but not yet inserted (one microtask wide).
 * Deduping against this window stops a single gesture that fires twice — a
 * Strict Mode double effect, or a `+` whose mousedown and click both quote —
 * from inserting the same chip twice, without blocking a deliberate re-quote.
 */
const scheduled = new Map<string, ComposerFileSelection[]>();

/** Drops live handlers so tests cannot leak a bound composer into the next case. */
export function resetComposerFileDeliveriesForTests(): void {
  deliveries.clear();
  scheduled.clear();
}

function sameSelection(
  left: ComposerFileSelection,
  right: ComposerFileSelection,
): boolean {
  return (
    left.path === right.path &&
    left.startLine === right.startLine &&
    left.endLine === right.endLine &&
    left.origin === right.origin &&
    left.diffSide === right.diffSide
  );
}

/** Keeps only selections that no entry in `against` already covers. */
function withoutDuplicates(
  selections: readonly ComposerFileSelection[],
  against: readonly ComposerFileSelection[],
): ComposerFileSelection[] {
  return selections.filter(
    (selection) =>
      !against.some((candidate) => sameSelection(candidate, selection)),
  );
}

/** Releases this batch's hold on the in-flight window once it is delivered. */
function releaseScheduled(
  conversationKey: string,
  delivered: readonly ComposerFileSelection[],
): void {
  const inFlight = scheduled.get(conversationKey);
  if (inFlight === undefined) return;
  const rest = inFlight.filter((candidate) => !delivered.includes(candidate));
  if (rest.length === 0) scheduled.delete(conversationKey);
  else scheduled.set(conversationKey, rest);
}

/** Bridges file-explorer actions to the conversation composer without coupling the two views. */
export const useComposerFileContextStore = create<ComposerFileContextState>(
  (set, get) => ({
    pendingByConversation: {},
    addSelection: (conversationKey, selection) =>
      get().addSelections(conversationKey, [selection]),
    addSelections: (conversationKey, selections) => {
      if (selections.length === 0) return false;
      const deliver = deliveries.get(conversationKey);
      if (deliver !== undefined) {
        const fresh = withoutDuplicates(
          selections,
          scheduled.get(conversationKey) ?? [],
        );
        if (fresh.length === 0) return false;
        scheduled.set(conversationKey, [
          ...(scheduled.get(conversationKey) ?? []),
          ...fresh,
        ]);
        // TipTap insert must not run inside the quote event / Zustand stack.
        // Do not write `pendingByConversation`: rebinding the composer would
        // replay chips the user already had (or had deleted).
        queueMicrotask(() => {
          releaseScheduled(conversationKey, fresh);
          const current = deliveries.get(conversationKey);
          if (current !== deliver) {
            get().addSelections(conversationKey, fresh);
            return;
          }
          try {
            current(fresh);
          } catch {
            // Last-resort guard for a handler that throws instead of reporting.
            // Not re-queued: the same handler is still bound, so a retry would
            // throw again forever. The composer surfaces its own insert
            // failures, so this path should stay unreachable in practice.
          }
        });
        return true;
      }
      const queue = get().pendingByConversation[conversationKey] ?? [];
      const fresh = withoutDuplicates(selections, queue);
      if (fresh.length === 0) return false;
      set({
        pendingByConversation: {
          ...get().pendingByConversation,
          [conversationKey]: [...queue, ...fresh],
        },
      });
      return true;
    },
    bindDelivery: (conversationKey, deliver) => {
      deliveries.set(conversationKey, deliver);
      // Drain after hydrate's microtask (bind is declared later in Composer).
      let cancelled = false;
      queueMicrotask(() => {
        if (cancelled) return;
        if (deliveries.get(conversationKey) !== deliver) return;
        const queued = get().pendingByConversation[conversationKey];
        if (queued === undefined || queued.length === 0) return;
        set((state) => {
          if (state.pendingByConversation[conversationKey] === undefined) {
            return state;
          }
          const pendingByConversation = { ...state.pendingByConversation };
          delete pendingByConversation[conversationKey];
          return { pendingByConversation };
        });
        deliver(queued);
      });
      return () => {
        cancelled = true;
        if (deliveries.get(conversationKey) === deliver) {
          deliveries.delete(conversationKey);
        }
      };
    },
  }),
);
