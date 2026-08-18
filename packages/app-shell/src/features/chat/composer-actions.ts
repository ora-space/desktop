import type * as acp from "@agentclientprotocol/sdk";
import type { Skill } from "@ora/contracts";
import type { PluginEntry } from "../settings/plugin-catalog";
import { availableSkills } from "../../state/hooks/use-skills";

export type ComposerActionGroup = "skills" | "commands" | "plugins" | "actions";

export type ComposerAction =
  | {
      id: string;
      group: "skills";
      label: string;
      description: string;
      skill: Skill;
    }
  | {
      id: string;
      group: "commands";
      label: string;
      description: string;
      hint?: string;
      command: acp.AvailableCommand;
    }
  | {
      id: string;
      group: "plugins";
      label: string;
      description: string;
      plugin: PluginEntry;
    }
  | {
      id: "action:add-images";
      group: "actions";
      label: string;
      description: string;
    };

export const COMPOSER_ACTION_GROUPS: readonly ComposerActionGroup[] = [
  "skills",
  "commands",
  "plugins",
  "actions",
];
export const COLLAPSED_ACTION_GROUP_SIZE = 5;

/** Builds searchable actions from provider capabilities, Ora's configured skills, and the plugin catalog. */
export function buildComposerActions({
  skills,
  commands,
  plugins,
  translatePluginSummary,
  includeAttachments,
  attachmentLabel,
  attachmentDescription,
}: {
  skills: Skill[];
  commands: acp.AvailableCommand[];
  plugins: PluginEntry[];
  translatePluginSummary: (summaryKey: string) => string;
  includeAttachments: boolean;
  attachmentLabel: string;
  attachmentDescription: string;
}): ComposerAction[] {
  return [
    ...availableSkills(skills).map((skill): ComposerAction => ({
      id: `skill:${skill.id}`,
      group: "skills",
      label: skill.name,
      description: skill.description,
      skill,
    })),
    ...commands.map((command): ComposerAction => ({
      id: `command:${command.name}`,
      group: "commands",
      label: command.name,
      description: command.description,
      ...(command.input == null ? {} : { hint: command.input.hint }),
      command,
    })),
    ...plugins.map((plugin): ComposerAction => ({
      id: `plugin:${plugin.id}`,
      group: "plugins",
      label: plugin.name,
      description: translatePluginSummary(plugin.summaryKey),
      plugin,
    })),
    ...(includeAttachments
      ? [
          {
            id: "action:add-images" as const,
            group: "actions" as const,
            label: attachmentLabel,
            description: attachmentDescription,
          },
        ]
      : []),
  ];
}

/** Filters actions with one predictable name-and-description search rule. */
export function filterComposerActions(
  actions: ComposerAction[],
  query: string,
): ComposerAction[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  if (normalizedQuery === "") return actions;
  return actions.filter(
    (action) =>
      action.label.toLocaleLowerCase().includes(normalizedQuery) ||
      action.description.toLocaleLowerCase().includes(normalizedQuery),
  );
}

/** Limits long capability groups until the user explicitly asks to reveal them. */
export function visibleComposerActions(
  actions: ComposerAction[],
  expandedGroups: ReadonlySet<ComposerActionGroup>,
): ComposerAction[] {
  return COMPOSER_ACTION_GROUPS.flatMap((group) => {
    const groupActions = actions.filter((action) => action.group === group);
    return expandedGroups.has(group)
      ? groupActions
      : groupActions.slice(0, COLLAPSED_ACTION_GROUP_SIZE);
  });
}
