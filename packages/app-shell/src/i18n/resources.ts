import { composeTranslationResources } from "./resource-bundle";
import { commonTranslations } from "./common-resources";
import { chatTranslations } from "../features/chat/translations";
import { diffTranslations } from "../features/diff/translations";
import { filesTranslations } from "../features/files/translations";
import { sidebarTranslations } from "../features/sidebar/translations";
import { surfaceTranslations } from "../features/surface/translations";
import { workspaceTranslations } from "../features/workspace/translations";
import { workflowEditorTranslations } from "../features/workflow-editor/translations";
import { workflowRunTranslations } from "../features/workflow-run/translations";
import { workflowNodeTranslations } from "../features/workflow-node-chrome/translations";
import {
  settingsTranslations,
  skillTranslations,
  roleTranslations,
  pluginTranslations,
} from "../features/settings/translations";

/** Explicit composition of feature-owned data; never initializes React or i18next. */
export const featureTranslationResources = {
  common: commonTranslations,
  chat: chatTranslations,
  diff: diffTranslations,
  files: filesTranslations,
  sidebar: sidebarTranslations,
  surface: surfaceTranslations,
  workspace: workspaceTranslations,
  workflowEditor: workflowEditorTranslations,
  workflowRun: workflowRunTranslations,
  workflowNode: workflowNodeTranslations,
  settings: settingsTranslations,
  skills: skillTranslations,
  roles: roleTranslations,
  plugins: pluginTranslations,
} as const;

export const translationResources = composeTranslationResources(
  featureTranslationResources,
);
