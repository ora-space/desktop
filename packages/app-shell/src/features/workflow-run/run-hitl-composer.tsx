import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button, cn } from "@ora/ui";
import {
  IconArrowUp,
  IconCheck,
  IconChevronDown,
  IconHandClick,
  IconLoader2,
} from "@tabler/icons-react";
import { formatRunClock } from "../../lib/format";
import type {
  HitlField,
  HitlGateKind,
  HitlRequest,
} from "@ora/workflow-runtime";
import {
  ComposerEditor,
  type ComposerEditorHandle,
} from "../editor/composer-editor";

export interface HitlGateOption {
  request: HitlRequest;
  nodeTitle: string;
}

const KIND_LABEL_KEY: Record<HitlGateKind, string> = {
  approval: "workflowRun.hitl.kind.approval",
  feedback: "workflowRun.hitl.kind.feedback",
  clarify: "workflowRun.hitl.kind.clarify",
};

/** Truncates prompt text for the collapsed reopen subtitle. */
function promptPreview(prompt: string, max = 72): string {
  const trimmed = prompt.trim().replace(/\s+/g, " ");
  if (trimmed.length <= max) {
    return trimmed;
  }
  return `${trimmed.slice(0, max - 1)}…`;
}

function initialValues(fields: HitlField[]): Record<string, string> {
  const next: Record<string, string> = {};
  for (const field of fields) {
    next[field.name] = "";
  }
  return next;
}

interface RunHitlComposerProps {
  gates: HitlGateOption[];
  selectedRequestId: string;
  onSelectRequest: (requestId: string) => void;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  submitting?: boolean;
  /** When set, only this gate shows pending chrome. */
  submittingRequestId?: string | null;
  submitError?: string | null;
  onSubmit: (payload: Record<string, string>) => void | Promise<void>;
  /**
   * `overlay` —standalone dock card.
   * `embedded` —lives inside the act card (no second chrome shell).
   */
  layout?: "overlay" | "embedded";
  /**
   * Overlay only: user engaged the dock while focus is elsewhere.
   * Parent should spotlight the selected gate’s act (defer if needed so
   * choice/submit clicks finish before the surface remounts as embedded).
   */
  onEngage?: () => void;
  /** Lifted drafts keyed by request id (survives overlay↔embedded remount). */
  drafts?: Record<string, Record<string, string>>;
  onDraftsChange?: (next: Record<string, Record<string, string>>) => void;
  /**
   * Companion control rendered inside HITL chrome (e.g. session dock) so the
   * gate and conversation entry read as one action cluster.
   */
  accessory?: ReactNode;
}

/**
 * One expandable HITL surface:
 * - Collapsed —warm amber prompt (must handle / open)
 * - Expanded —amber wait pulse + model prompt body + tiles / composer
 * Closing collapses only; it does not cancel the gate.
 */
export function RunHitlComposer({
  gates,
  selectedRequestId,
  onSelectRequest,
  expanded,
  onExpandedChange,
  submitting = false,
  submittingRequestId = null,
  submitError = null,
  onSubmit,
  layout = "overlay",
  onEngage,
  drafts: draftsProp,
  onDraftsChange,
  accessory,
}: RunHitlComposerProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  const primaryEditorRef = useRef<ComposerEditorHandle>(null);
  const editorByFieldRef = useRef(new Map<string, ComposerEditorHandle>());
  const selected = useMemo(
    () =>
      gates.find((gate) => gate.request.id === selectedRequestId) ??
      gates[0] ??
      null,
    [gates, selectedRequestId],
  );
  const request = selected?.request ?? null;
  const waitingNodeTitle = selected?.nodeTitle ?? "";
  const multi = gates.length > 1;
  const selectFields =
    request?.schema.fields.filter((field) => field.type === "select") ?? [];
  const textFields =
    request?.schema.fields.filter(
      (field) => field.type === "text" || field.type === "textarea",
    ) ?? [];
  const primaryTextField = textFields[0] ?? null;
  const choiceOnly = selectFields.length > 0 && textFields.length === 0;
  const schemaTitle = request?.schema.title;
  const gateBusy =
    submittingRequestId !== null
      ? submittingRequestId === request?.id
      : submitting;

  const [internalDrafts, setInternalDrafts] = useState<
    Record<string, Record<string, string>>
  >(() =>
    Object.fromEntries(
      gates.map((gate) => [
        gate.request.id,
        initialValues(gate.request.schema.fields),
      ]),
    ),
  );
  const controlled = onDraftsChange !== undefined;
  const drafts = draftsProp ?? internalDrafts;

  function replaceDrafts(next: Record<string, Record<string, string>>): void {
    if (controlled) {
      onDraftsChange(next);
      return;
    }
    setInternalDrafts(next);
  }

  // Derive the gate-aligned draft projection instead of writing it back in an
  // effect: seed entries for newly opened gates and drop entries for closed
  // ones, while keeping user edits in `drafts` untouched until the next write.
  const effectiveDrafts = useMemo(() => {
    const next = { ...drafts };
    for (const gate of gates) {
      if (next[gate.request.id] === undefined) {
        next[gate.request.id] = initialValues(gate.request.schema.fields);
      }
    }
    for (const id of Object.keys(next)) {
      if (!gates.some((gate) => gate.request.id === id)) {
        delete next[id];
      }
    }
    return next;
  }, [drafts, gates]);

  const [localError, setLocalError] = useState<string | null>(null);
  // Reset per-gate validation state through the documented render-adjust pattern
  // rather than a state-syncing effect.
  const [previousRequestId, setPreviousRequestId] = useState(request?.id);
  if (request?.id !== previousRequestId) {
    setPreviousRequestId(request?.id);
    setLocalError(null);
  }
  const values =
    request === null
      ? {}
      : (effectiveDrafts[request.id] ?? initialValues(request.schema.fields));

  // Focus once when the surface opens or the active gate changes —not per keystroke.
  useEffect(() => {
    if (!expanded || request === null || primaryTextField === null) {
      return;
    }
    // preventScroll avoids Theater jump-scrolling the stage onto the composer
    // when a gate opens or the active request changes.
    primaryEditorRef.current?.focus({ preventScroll: true });
  }, [expanded, request, primaryTextField]);

  if (request === null) {
    return null;
  }

  function setField(name: string, value: string): void {
    replaceDrafts({
      ...effectiveDrafts,
      [request.id]: {
        ...(effectiveDrafts[request.id] ?? values),
        [name]: value,
      },
    });
    setLocalError(null);
  }

  function missingRequired(next: Record<string, string>): string | null {
    for (const field of request.schema.fields) {
      if (field.required === true && (next[field.name] ?? "").trim() === "") {
        return field.type === "select"
          ? t("workflowRun.hitl.chooseRequired", { label: field.label })
          : t("workflowRun.hitl.required");
      }
    }
    return null;
  }

  function collectFieldValues(): Record<string, string> {
    const next = { ...values };
    for (const [name, handle] of editorByFieldRef.current) {
      next[name] = handle.getText();
    }
    return next;
  }

  async function submitWith(next: Record<string, string>): Promise<void> {
    const missing = missingRequired(next);
    if (missing !== null) {
      setLocalError(missing);
      return;
    }
    await onSubmit(next);
  }

  async function submit(): Promise<void> {
    await submitWith(collectFieldValues());
  }

  function onSelectOption(fieldName: string, optionValue: string): void {
    const next = { ...values, [fieldName]: optionValue };
    replaceDrafts({ ...effectiveDrafts, [request.id]: next });
    setLocalError(null);
    if (!choiceOnly || gateBusy) {
      return;
    }
    const selectsComplete = selectFields.every((field) => {
      if (field.required !== true) {
        return true;
      }
      return (next[field.name] ?? "").trim() !== "";
    });
    if (selectsComplete) {
      void submitWith(next);
    }
  }

  const errorMessage = localError ?? submitError;
  const canSend = !gateBusy && missingRequired(values) === null;
  const schemaPrompt = request.schema.prompt?.trim() ?? "";
  const gateKind = request.schema.kind;
  const collapsedSummary = multi
    ? t("workflowRun.hitl.multiWaiting", { count: gates.length })
    : waitingNodeTitle || schemaTitle || t("workflowRun.hitl.reopenTitle");
  const collapsedDetail = multi
    ? t("workflowRun.hitl.multiWaitingHint")
    : schemaPrompt !== ""
      ? promptPreview(schemaPrompt)
      : (schemaTitle ?? t("workflowRun.hitl.reopenTitle"));
  const detail = schemaTitle ?? t("workflowRun.hitl.reopenTitle");
  const embedded = layout === "embedded";
  const showTimeout =
    request.timeoutAt !== undefined && request.policy !== "wait";
  const timeoutLabel =
    showTimeout && request.timeoutAt !== undefined
      ? formatRunClock(request.timeoutAt, locale)
      : "";

  /** Spotlight this gate when the user pointer-engages the under-stage dock. */
  function engageOverlay(event?: { target: EventTarget | null }): void {
    if (embedded) {
      return;
    }
    if (
      event?.target instanceof Element &&
      event.target.closest("[data-hitl-collapse], [data-hitl-accessory]") !==
        null
    ) {
      return;
    }
    onEngage?.();
  }

  if (!expanded) {
    return (
      <div
        className={cn(
          "flex items-center gap-2",
          !embedded && "mx-auto w-full max-w-xl",
        )}
      >
        <button
          type="button"
          className={cn(
            "group flex min-w-0 flex-1 cursor-pointer items-center gap-3 text-left",
            "transition-[background-color,border-color,box-shadow] duration-200",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-500/25",
            embedded
              ? "rounded-xl border border-amber-500/30 bg-amber-500/5 px-3 py-2.5 hover:border-amber-500/45 hover:bg-amber-500/[0.08]"
              : cn(
                  "rounded-2xl border border-amber-500/30 bg-amber-500/5 px-3.5 py-3",
                  "shadow-[0_1px_2px_rgba(180,83,9,0.04)]",
                  "hover:border-amber-500/45 hover:bg-amber-500/[0.08]",
                ),
          )}
          aria-expanded={false}
          onClick={() => onExpandedChange(true)}
        >
          <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-amber-500/15 text-amber-800 dark:text-amber-200">
            <IconHandClick className="size-3.5" aria-hidden />
          </span>
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-medium text-foreground">
              {collapsedSummary}
            </p>
            <p className="truncate text-xs text-muted-foreground">
              {collapsedDetail}
            </p>
          </div>
          <span className="inline-flex h-7 shrink-0 items-center gap-1 text-xs font-medium text-amber-900/80 transition-colors group-hover:text-amber-950 dark:text-amber-200/80">
            {t("workflowRun.hitl.reopenAction")}
            <IconChevronDown
              className="size-3.5 rotate-180 opacity-70"
              aria-hidden
            />
          </span>
        </button>
        {accessory !== undefined && accessory !== null && (
          <div data-hitl-accessory="" className="shrink-0">
            {accessory}
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      className={cn(
        "animate-in fade-in slide-in-from-bottom-1 duration-200 fill-mode-both motion-reduce:animate-none",
        !embedded && "mx-auto w-full max-w-xl",
      )}
      onPointerDown={engageOverlay}
    >
      <section
        className={cn(
          embedded
            ? "rounded-xl border border-amber-500/25 bg-amber-500/[0.05] p-3 dark:bg-amber-400/[0.07]"
            : cn(
                "rounded-2xl border border-amber-500/20 bg-card p-4",
                "shadow-[0_1px_2px_rgba(180,83,9,0.04),0_8px_24px_rgba(0,0,0,0.04)]",
                "dark:shadow-[0_1px_2px_rgba(0,0,0,0.28),0_10px_28px_rgba(0,0,0,0.16)]",
              ),
        )}
      >
        <div className="flex items-start gap-3">
          <span
            className="relative mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-full bg-amber-500/10"
            aria-hidden
          >
            <span className="absolute inset-0 animate-ping rounded-full bg-amber-500/25 motion-reduce:animate-none" />
            <span className="relative size-2.5 animate-pulse rounded-full bg-amber-500 motion-reduce:animate-none" />
          </span>
          <div className="min-w-0 flex-1 pt-0.5">
            {multi ? (
              <div
                className="flex flex-wrap gap-1"
                role="tablist"
                aria-label={t("workflowRun.hitl.gatesLabel")}
              >
                {gates.map((gate) => {
                  const active = gate.request.id === request.id;
                  return (
                    <button
                      key={gate.request.id}
                      type="button"
                      role="tab"
                      aria-selected={active}
                      className={cn(
                        "inline-flex max-w-full cursor-pointer items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors duration-150",
                        active
                          ? "border-foreground/20 bg-foreground text-background"
                          : "border-border bg-background text-muted-foreground hover:border-foreground/15 hover:text-foreground",
                      )}
                      onClick={() => {
                        onSelectRequest(gate.request.id);
                        setLocalError(null);
                      }}
                    >
                      <span className="truncate">{gate.nodeTitle}</span>
                      <span
                        className={cn(
                          "shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium",
                          active
                            ? "bg-background/20 text-background"
                            : "bg-muted text-muted-foreground",
                        )}
                      >
                        {t(KIND_LABEL_KEY[gate.request.schema.kind])}
                      </span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <>
                <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                  <p className="truncate text-sm font-medium tracking-tight text-foreground">
                    {waitingNodeTitle}
                  </p>
                  <span className="rounded-full border border-border/80 bg-background/80 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                    {t(KIND_LABEL_KEY[gateKind])}
                  </span>
                </div>
                <p className="mt-0.5 truncate text-xs text-muted-foreground">
                  {detail}
                </p>
              </>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-0.5">
            {accessory !== undefined && accessory !== null && (
              <div data-hitl-accessory="">{accessory}</div>
            )}
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              data-hitl-collapse=""
              className="size-7 shrink-0 cursor-pointer rounded-full text-muted-foreground hover:bg-amber-500/10 hover:text-amber-950 dark:hover:text-amber-100"
              aria-label={t("workflowRun.hitl.collapseAction")}
              aria-expanded={true}
              onPointerDown={(event) => event.stopPropagation()}
              onClick={() => onExpandedChange(false)}
            >
              <IconChevronDown className="size-3.5" />
            </Button>
          </div>
        </div>

        {schemaPrompt !== "" && (
          <div
            className={cn(
              "mt-3 rounded-lg px-3 py-2.5",
              gateKind === "clarify"
                ? "border border-amber-500/30 bg-background/90"
                : "border border-border/70 bg-background/70",
            )}
          >
            {gateKind === "clarify" && (
              <p className="text-[10px] font-medium uppercase tracking-[0.04em] text-amber-900 dark:text-amber-200">
                {t("workflowRun.hitl.modelQuestion")}
              </p>
            )}
            <p
              className={cn(
                "whitespace-pre-wrap text-sm leading-6 text-foreground/90",
                gateKind === "clarify" && "mt-1",
              )}
            >
              {schemaPrompt}
            </p>
          </div>
        )}

        {showTimeout && (
          <p className="mt-2 text-[11px] tabular-nums text-amber-800/80 dark:text-amber-200/80">
            {t("workflowRun.hitl.timeoutAt", { at: timeoutLabel })}
          </p>
        )}

        {errorMessage !== null && errorMessage !== "" && (
          <p role="alert" className="mt-3 text-xs text-destructive">
            {errorMessage}
          </p>
        )}

        {choiceOnly ? (
          <div className="mt-4 space-y-3">
            {selectFields.map((field) => {
              const options = field.options ?? [];
              const evenPair = options.length === 2;
              return (
                <div
                  key={field.name}
                  className={cn(
                    "grid gap-2",
                    evenPair ? "grid-cols-2" : "grid-cols-1 sm:grid-cols-2",
                  )}
                >
                  {options.map((option, index) => {
                    const active = values[field.name] === option.value;
                    return (
                      <button
                        key={option.value}
                        type="button"
                        disabled={gateBusy}
                        aria-pressed={active}
                        style={{ animationDelay: `${index * 40}ms` }}
                        className={cn(
                          "group relative flex min-h-11 cursor-pointer items-center justify-center gap-2 rounded-xl border px-3 py-2.5 text-center text-sm font-medium",
                          "animate-in fade-in zoom-in-95 duration-200 fill-mode-both motion-reduce:animate-none",
                          "transition-[background-color,border-color,box-shadow,color,opacity] duration-200 ease-out",
                          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-500/30",
                          "disabled:pointer-events-none disabled:opacity-60",
                          active
                            ? "border-foreground/25 bg-foreground text-background shadow-sm"
                            : "border-border/90 bg-background text-foreground hover:border-amber-500/40 hover:bg-amber-500/[0.06]",
                        )}
                        onClick={() => onSelectOption(field.name, option.value)}
                      >
                        {gateBusy && active ? (
                          <IconLoader2 className="size-4 animate-spin" />
                        ) : (
                          <>
                            <span className="leading-none">{option.label}</span>
                            {active && (
                              <IconCheck
                                className="size-3.5 shrink-0 opacity-90"
                                aria-hidden
                              />
                            )}
                          </>
                        )}
                      </button>
                    );
                  })}
                </div>
              );
            })}
          </div>
        ) : (
          <div className="mt-4 space-y-3">
            {selectFields.length > 0 && (
              <HitlChoiceStrip
                fields={selectFields}
                values={values}
                submitting={gateBusy}
                onSelect={onSelectOption}
              />
            )}
            {textFields.map((field, index) => {
              const isPrimary = index === 0;
              return (
                <div
                  key={field.name}
                  data-slot={isPrimary ? "hitl-composer" : undefined}
                  className={cn(
                    "relative flex flex-col rounded-xl border border-border/80 bg-background",
                    "transition-[border-color,box-shadow] duration-200",
                    "focus-within:border-amber-500/40 focus-within:ring-2 focus-within:ring-amber-500/20",
                  )}
                >
                  <div className="flex flex-col p-2">
                    <label
                      htmlFor={`hitl-text-${request.id}-${field.name}`}
                      className={cn(
                        textFields.length > 1
                          ? "px-2 pb-1 text-[11px] font-medium text-muted-foreground"
                          : "sr-only",
                      )}
                    >
                      {field.label}
                    </label>
                    <ComposerEditor
                      key={`${request.id}-${field.name}`}
                      id={`hitl-text-${request.id}-${field.name}`}
                      ref={(handle) => {
                        if (handle === null) {
                          editorByFieldRef.current.delete(field.name);
                        } else {
                          editorByFieldRef.current.set(field.name, handle);
                        }
                        if (isPrimary) {
                          primaryEditorRef.current = handle;
                        }
                      }}
                      initialText={values[field.name] ?? ""}
                      placeholder={
                        field.placeholder ??
                        t("workflowRun.hitl.composerPlaceholder")
                      }
                      disabled={gateBusy}
                      enterKey={isPrimary ? "submit" : "newline"}
                      ariaLabel={field.label}
                      onSubmit={() => {
                        if (!gateBusy) {
                          void submit();
                        }
                      }}
                      onTextChange={(text) => setField(field.name, text)}
                    />
                    {index === textFields.length - 1 && (
                      <div className="flex min-h-8 items-center justify-end gap-2 pt-0.5">
                        <Button
                          type="button"
                          size="icon"
                          disabled={!canSend}
                          aria-label={t("workflowRun.hitl.submit")}
                          className="size-8 shrink-0 cursor-pointer rounded-full bg-foreground text-background shadow-sm transition-[background-color,color,box-shadow] duration-200 hover:bg-foreground/85 hover:shadow-md disabled:bg-muted disabled:text-muted-foreground disabled:shadow-none"
                          onClick={() => {
                            void submit();
                          }}
                        >
                          {gateBusy ? (
                            <IconLoader2 className="size-[18px] animate-spin" />
                          ) : (
                            <IconArrowUp className="size-[18px]" />
                          )}
                        </Button>
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}

/** Mixed-mode chips —same capsule language as the choice tiles. */
function HitlChoiceStrip({
  fields,
  values,
  submitting,
  onSelect,
}: {
  fields: HitlField[];
  values: Record<string, string>;
  submitting: boolean;
  onSelect: (fieldName: string, value: string) => void;
}) {
  return (
    <div className="space-y-2.5">
      {fields.map((field) => (
        <div key={field.name} className="space-y-1.5">
          <p className="text-[11px] font-medium text-muted-foreground">
            {field.label}
          </p>
          <div className="flex flex-wrap gap-1.5">
            {(field.options ?? []).map((option) => {
              const active = values[field.name] === option.value;
              return (
                <button
                  key={option.value}
                  type="button"
                  disabled={submitting}
                  aria-pressed={active}
                  className={cn(
                    "inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-full border px-3 text-xs font-medium",
                    "transition-[background-color,border-color,color,box-shadow] duration-200 ease-out",
                    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-500/30",
                    "disabled:pointer-events-none disabled:opacity-60",
                    active
                      ? "border-foreground/20 bg-foreground text-background shadow-sm"
                      : "border-border bg-background text-muted-foreground hover:border-amber-500/40 hover:bg-amber-500/[0.06] hover:text-foreground",
                  )}
                  onClick={() => onSelect(field.name, option.value)}
                >
                  {option.label}
                  {active && (
                    <IconCheck className="size-3 opacity-90" aria-hidden />
                  )}
                </button>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}
