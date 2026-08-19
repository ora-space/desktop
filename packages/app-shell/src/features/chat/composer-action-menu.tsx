import type {
  MouseEvent,
  MutableRefObject,
  PointerEvent,
  ReactNode,
} from "react";
import { useMemo, useRef } from "react";
import {
  IconBolt,
  IconLoader2,
  IconPhoto,
  IconPlug,
  IconSparkles,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { WorkspaceFileIcon } from "../files/workspace-file-visuals";
import {
  COLLAPSED_ACTION_GROUP_SIZE,
  COMPOSER_ACTION_GROUPS,
  visibleComposerActions,
  type ComposerAction,
  type ComposerActionGroup,
} from "./composer-actions";

interface ComposerActionMenuProps {
  id: string;
  actions: ComposerAction[];
  activeIndex: number;
  expandedGroups: ReadonlySet<ComposerActionGroup>;
  optionRefs: MutableRefObject<Array<HTMLButtonElement | null>>;
  onActiveIndexChange: (index: number) => void;
  onToggleGroup: (group: ComposerActionGroup) => void;
  onSelect: (action: ComposerAction) => void;
  /** When set, the palette shows a status row instead of (or above) empty groups. */
  status?: "ready" | "loading" | "empty" | "error";
  statusMessage?: string;
  truncated?: boolean;
  /** Tighter @ file picker: shorter list, single-line rows, no group header. */
  filesPalette?: boolean;
  /** When true, file rows stay visible but cannot be committed (debounce / fetch). */
  selectionLocked?: boolean;
}

/** Renders the compact Cursor-style capability palette shared by slash, at, and plus. */
export function ComposerActionMenu({
  id,
  actions,
  activeIndex,
  expandedGroups,
  optionRefs,
  onActiveIndexChange,
  onToggleGroup,
  onSelect,
  status = "ready",
  statusMessage,
  truncated = false,
  filesPalette = false,
  selectionLocked = false,
}: ComposerActionMenuProps) {
  const { t } = useTranslation();
  const allVisibleActions = visibleComposerActions(actions, expandedGroups);
  const indexById = useMemo(() => {
    const map = new Map<string, number>();
    allVisibleActions.forEach((action, index) => {
      map.set(action.id, index);
    });
    return map;
  }, [allVisibleActions]);
  const showStatus = status !== "ready" || statusMessage !== undefined;
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  /** After a wheel scroll, sync highlight to the row still under the cursor. */
  const syncHighlightUnderPointer = () => {
    const last = lastPointerRef.current;
    const list = listRef.current;
    if (last === null || list === null) return;
    const hit = document.elementFromPoint(last.x, last.y);
    if (!(hit instanceof Element) || !list.contains(hit)) return;
    const option = hit.closest('[role="option"]');
    if (!(option instanceof HTMLElement)) return;
    const optionId = option.id;
    const prefix = `${id}-option-`;
    if (!optionId.startsWith(prefix)) return;
    const index = Number(optionId.slice(prefix.length));
    if (!Number.isFinite(index) || index === activeIndex) return;
    onActiveIndexChange(index);
  };

  return (
    <div
      id={id}
      role="listbox"
      aria-label={t("chat.actionMenu.label")}
      aria-busy={status === "loading"}
      className={
        filesPalette
          ? "absolute bottom-[calc(100%+6px)] left-2 z-40 w-[min(280px,calc(100vw-32px))] overflow-hidden rounded-lg border border-border/80 bg-popover p-1 text-popover-foreground shadow-[0_8px_24px_rgba(0,0,0,0.12),0_1px_4px_rgba(0,0,0,0.06)] ring-1 ring-foreground/5 dark:shadow-[0_12px_32px_rgba(0,0,0,0.4)]"
          : "absolute bottom-[calc(100%+8px)] left-2 z-40 w-[min(324px,calc(100vw-32px))] overflow-hidden rounded-lg border border-border bg-popover p-1.5 text-popover-foreground shadow-[0_12px_32px_rgba(0,0,0,0.16),0_2px_8px_rgba(0,0,0,0.08)] ring-1 ring-foreground/5 dark:shadow-[0_16px_40px_rgba(0,0,0,0.45)]"
      }
    >
      <div
        ref={listRef}
        onScroll={syncHighlightUnderPointer}
        className={
          filesPalette
            ? "max-h-[min(220px,36vh)] overflow-y-auto overscroll-contain scroll-py-1"
            : "max-h-[min(320px,48vh)] overflow-y-auto overscroll-contain scroll-py-6"
        }
      >
        {showStatus && statusMessage !== undefined && (
          <p
            role="status"
            className={
              filesPalette
                ? "flex min-h-7 items-center gap-1.5 px-2 py-1 text-[11px] text-muted-foreground"
                : "flex min-h-8 items-center gap-1.5 px-2 py-1.5 text-[12px] text-muted-foreground"
            }
          >
            {status === "loading" && (
              <IconLoader2
                className="size-3.5 shrink-0 animate-spin motion-reduce:animate-none"
                aria-hidden="true"
              />
            )}
            {statusMessage}
          </p>
        )}
        {COMPOSER_ACTION_GROUPS.map((group) => {
          const groupActions = actions.filter(
            (action) => action.group === group,
          );
          if (groupActions.length === 0) return null;
          const expanded = expandedGroups.has(group);
          const visibleGroupActions =
            group === "files" || expanded
              ? groupActions
              : groupActions.slice(0, COLLAPSED_ACTION_GROUP_SIZE);
          const hiddenCount = groupActions.length - visibleGroupActions.length;
          const hideFilesHeader = filesPalette && group === "files";

          return (
            <section
              key={group}
              role="group"
              aria-labelledby={hideFilesHeader ? undefined : `${id}-${group}`}
              className={filesPalette ? "pb-0" : "pb-1 last:pb-0"}
            >
              {!hideFilesHeader && (
                <p
                  id={`${id}-${group}`}
                  className="flex h-7 items-center px-2 text-[11px] font-medium text-muted-foreground"
                >
                  {t(`chat.actionMenu.${group}`)}
                </p>
              )}
              {visibleGroupActions.map((action) => {
                const index = indexById.get(action.id) ?? -1;
                if (index < 0) return null;
                const rowLocked = selectionLocked && action.group === "files";
                return (
                  <ActionOption
                    key={action.id}
                    id={`${id}-option-${index}`}
                    action={action}
                    active={index === activeIndex}
                    compactFile={filesPalette && action.group === "files"}
                    selectionLocked={rowLocked}
                    buttonRef={(node) => {
                      optionRefs.current[index] = node;
                    }}
                    onPointerMove={(event) => {
                      // Wheel scroll synthesizes pointermove with an unchanged
                      // cursor, which would otherwise retarget the highlight
                      // and feel like the list is fighting the user.
                      const last = lastPointerRef.current;
                      if (
                        last !== null &&
                        last.x === event.clientX &&
                        last.y === event.clientY
                      ) {
                        return;
                      }
                      lastPointerRef.current = {
                        x: event.clientX,
                        y: event.clientY,
                      };
                      onActiveIndexChange(index);
                    }}
                    onSelect={() => {
                      if (rowLocked) return;
                      onSelect(action);
                    }}
                  />
                );
              })}
              {group !== "files" &&
                groupActions.length > COLLAPSED_ACTION_GROUP_SIZE && (
                  <button
                    type="button"
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => onToggleGroup(group)}
                    className="flex h-8 w-full cursor-pointer items-center rounded-md px-2 text-left text-xs text-muted-foreground outline-none transition-colors duration-150 hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    {expanded
                      ? t("chat.actionMenu.showLess")
                      : t("chat.actionMenu.showMore", { count: hiddenCount })}
                  </button>
                )}
            </section>
          );
        })}
        {truncated && (
          <p
            className={
              filesPalette
                ? "px-2 py-1 text-[10px] text-muted-foreground"
                : "px-2 py-1.5 text-[11px] text-muted-foreground"
            }
          >
            {t("chat.actionMenu.filesTruncated")}
          </p>
        )}
      </div>
    </div>
  );
}

/** Renders one stable-height palette row without shifting on hover or selection. */
function ActionOption({
  id,
  action,
  active,
  compactFile,
  selectionLocked,
  buttonRef,
  onPointerMove,
  onSelect,
}: {
  id: string;
  action: ComposerAction;
  active: boolean;
  compactFile: boolean;
  selectionLocked: boolean;
  buttonRef: (node: HTMLButtonElement | null) => void;
  onPointerMove: (event: PointerEvent<HTMLButtonElement>) => void;
  onSelect: () => void;
}) {
  // Prefer aria-disabled over the disabled attribute so clicks still land and
  // can be ignored in onSelect — native disabled swallows the event entirely.
  const sharedProps = {
    ref: buttonRef,
    id,
    type: "button" as const,
    role: "option" as const,
    "aria-selected": active,
    "aria-disabled": selectionLocked,
    onMouseDown: (event: MouseEvent<HTMLButtonElement>) =>
      event.preventDefault(),
    onPointerMove,
    onClick: onSelect,
  };

  if (action.group === "files") {
    if (compactFile) {
      return (
        <button
          {...sharedProps}
          title={action.path}
          className="group flex h-7 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left outline-none transition-colors duration-150 hover:bg-accent aria-selected:bg-accent focus-visible:ring-2 focus-visible:ring-ring aria-disabled:cursor-default aria-disabled:opacity-55 aria-disabled:hover:bg-transparent"
        >
          <WorkspaceFileIcon
            path={action.path}
            kind={action.entryKind}
            className="size-3.5 shrink-0"
          />
          <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-foreground group-aria-selected:text-accent-foreground">
            {action.label}
          </span>
          {action.description !== "." && (
            <span className="max-w-[42%] shrink-0 truncate font-mono text-[10px] text-muted-foreground group-aria-selected:text-accent-foreground/60">
              {action.description}
            </span>
          )}
        </button>
      );
    }
    return (
      <button
        {...sharedProps}
        title={action.path}
        className="group flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-1.5 text-left outline-none transition-colors duration-150 hover:bg-accent aria-selected:bg-accent aria-selected:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring aria-disabled:cursor-default aria-disabled:opacity-55 aria-disabled:hover:bg-transparent"
      >
        <WorkspaceFileIcon
          path={action.path}
          kind={action.entryKind}
          className="size-4"
        />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] font-medium leading-tight">
            {action.label}
          </span>
          <span className="block truncate font-mono text-[10px] leading-tight text-muted-foreground group-aria-selected:text-accent-foreground/70">
            {action.description}
          </span>
        </span>
      </button>
    );
  }

  return (
    <button
      {...sharedProps}
      title={action.description || undefined}
      className="group flex h-8 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left text-[13px] outline-none transition-colors duration-150 hover:bg-accent aria-selected:bg-accent aria-selected:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring"
    >
      {actionIcon(action)}
      <span className="min-w-0 flex-1 truncate">{action.label}</span>
      {action.group === "commands" && action.hint && (
        <span className="max-w-24 truncate font-mono text-[10px] text-muted-foreground">
          {action.hint}
        </span>
      )}
    </button>
  );
}

/** Chooses a consistent line icon for each capability group; plugins show their own brand mark. */
function actionIcon(
  action: Exclude<ComposerAction, { group: "files" }>,
): ReactNode {
  const commonClassName =
    "size-4 shrink-0 text-muted-foreground group-aria-selected:text-foreground";
  switch (action.group) {
    case "skills":
      return <IconSparkles className={commonClassName} aria-hidden="true" />;
    case "commands":
      return <IconBolt className={commonClassName} aria-hidden="true" />;
    case "plugins":
      return <IconPlug className={commonClassName} aria-hidden="true" />;
    case "actions":
      return <IconPhoto className={commonClassName} aria-hidden="true" />;
  }
}
