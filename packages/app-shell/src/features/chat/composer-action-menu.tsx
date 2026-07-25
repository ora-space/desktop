import type { MutableRefObject, ReactNode } from "react";
import {
  IconBolt,
  IconCheck,
  IconListCheck,
  IconMessage,
  IconPhoto,
  IconSparkles,
} from "@tabler/icons-react";
import type { acp } from "@ora/contracts";
import { useTranslation } from "react-i18next";
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
  currentModeId?: acp.SessionModeId;
  expandedGroups: ReadonlySet<ComposerActionGroup>;
  optionRefs: MutableRefObject<Array<HTMLButtonElement | null>>;
  onActiveIndexChange: (index: number) => void;
  onToggleGroup: (group: ComposerActionGroup) => void;
  onSelect: (action: ComposerAction) => void;
}

/** Renders the compact Cursor-style capability palette shared by slash and plus. */
export function ComposerActionMenu({
  id,
  actions,
  activeIndex,
  currentModeId,
  expandedGroups,
  optionRefs,
  onActiveIndexChange,
  onToggleGroup,
  onSelect,
}: ComposerActionMenuProps) {
  const { t } = useTranslation();
  const allVisibleActions = visibleComposerActions(actions, expandedGroups);

  return (
    <div
      id={id}
      role="listbox"
      aria-label={t("chat.actionMenu.label")}
      className="absolute bottom-[calc(100%+8px)] left-2 z-40 w-[min(324px,calc(100vw-32px))] overflow-hidden rounded-lg border border-border bg-popover p-1.5 text-popover-foreground shadow-[0_12px_32px_rgba(0,0,0,0.16),0_2px_8px_rgba(0,0,0,0.08)] ring-1 ring-foreground/5 dark:shadow-[0_16px_40px_rgba(0,0,0,0.45)]"
    >
      <div className="max-h-[min(420px,55vh)] overflow-y-auto overscroll-contain scroll-py-6">
        {COMPOSER_ACTION_GROUPS.map((group) => {
          const groupActions = actions.filter((action) => action.group === group);
          if (groupActions.length === 0) return null;
          const expanded = expandedGroups.has(group);
          const visibleGroupActions = expanded
            ? groupActions
            : groupActions.slice(0, COLLAPSED_ACTION_GROUP_SIZE);
          const hiddenCount = groupActions.length - visibleGroupActions.length;

          return (
            <section key={group} role="group" aria-labelledby={`${id}-${group}`} className="pb-1 last:pb-0">
              <p id={`${id}-${group}`} className="flex h-7 items-center px-2 text-[11px] font-medium text-muted-foreground">
                {t(`chat.actionMenu.${group}`)}
              </p>
              {visibleGroupActions.map((action) => {
                const index = allVisibleActions.findIndex((candidate) => candidate.id === action.id);
                return (
                  <ActionOption
                    key={action.id}
                    id={`${id}-option-${index}`}
                    action={action}
                    active={index === activeIndex}
                    currentModeId={currentModeId}
                    buttonRef={(node) => { optionRefs.current[index] = node; }}
                    onPointerMove={() => onActiveIndexChange(index)}
                    onSelect={() => onSelect(action)}
                  />
                );
              })}
              {groupActions.length > COLLAPSED_ACTION_GROUP_SIZE && (
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
      </div>
    </div>
  );
}

/** Renders one stable-height palette row without shifting on hover or selection. */
function ActionOption({
  id,
  action,
  active,
  currentModeId,
  buttonRef,
  onPointerMove,
  onSelect,
}: {
  id: string;
  action: ComposerAction;
  active: boolean;
  currentModeId?: acp.SessionModeId;
  buttonRef: (node: HTMLButtonElement | null) => void;
  onPointerMove: () => void;
  onSelect: () => void;
}) {
  const selectedMode = action.group === "modes" && action.mode.id === currentModeId;
  return (
    <button
      ref={buttonRef}
      id={id}
      type="button"
      role="option"
      aria-selected={active}
      aria-current={selectedMode ? "true" : undefined}
      title={action.description || undefined}
      onMouseDown={(event) => event.preventDefault()}
      onPointerMove={onPointerMove}
      onClick={onSelect}
      className="group flex h-8 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left text-[13px] outline-none transition-colors duration-150 hover:bg-accent aria-selected:bg-accent aria-selected:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring"
    >
      {actionIcon(action)}
      <span className="min-w-0 flex-1 truncate">{action.label}</span>
      {action.group === "commands" && action.hint && (
        <span className="max-w-24 truncate font-mono text-[10px] text-muted-foreground">{action.hint}</span>
      )}
      {selectedMode && <IconCheck className="size-3.5 shrink-0 text-foreground" aria-hidden="true" />}
    </button>
  );
}

/** Chooses a consistent line icon for each capability while distinguishing mode intent. */
function actionIcon(action: ComposerAction): ReactNode {
  const commonClassName = "size-4 shrink-0 text-muted-foreground group-aria-selected:text-foreground";
  switch (action.group) {
    case "skills":
      return <IconSparkles className={commonClassName} aria-hidden="true" />;
    case "commands":
      return <IconBolt className={commonClassName} aria-hidden="true" />;
    case "modes":
      if (/plan/i.test(`${action.mode.id} ${action.mode.name}`)) {
        return <IconListCheck className="size-4 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />;
      }
      return (
        <IconMessage
          className={`size-4 shrink-0 ${/debug/i.test(`${action.mode.id} ${action.mode.name}`) ? "text-rose-500" : "text-muted-foreground"}`}
          aria-hidden="true"
        />
      );
    case "actions":
      return <IconPhoto className={commonClassName} aria-hidden="true" />;
  }
}
