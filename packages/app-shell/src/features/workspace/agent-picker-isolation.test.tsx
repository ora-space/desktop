import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "../../platform";
import { describe, expect, it, beforeEach } from "vitest";
import type { Project, Task } from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import {
  useSettingsStore,
  DEFAULT_SETTINGS,
} from "../../state/stores/settings-store";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { WorkspaceSidebar } from "./workspace-sidebar";
import { WorkspaceView } from "./workspace-view";

const USER = { name: "Eric", email: "eric@example.com" };
const PROJECT: Project = { id: "p1", name: "Ora Desktop" };
const TASK1: Task = {
  id: "t1",
  projectId: "p1",
  workspaceId: "workspace-t1",
  title: "Task One",
};
const TASK2: Task = {
  id: "t2",
  projectId: "p1",
  workspaceId: "workspace-t2",
  title: "Task Two",
};

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useDraftSessionsStore.getState().clear();
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: "ora-space.opencode" },
  });
  usePendingAgentStore.setState({ selections: {} });
});

/** Renders the sidebar and the workspace view together, as AppShell composes them. */
function renderWorkspace() {
  const state = createMockClientState();
  state.projects = [PROJECT];
  state.tasks = [TASK1, TASK2];
  const client = createMockClient(state);
  const chatStore = createChatStore(client.session);
  const Wrapper = createHookWrapper(client, createTestQueryClient(), chatStore);
  render(
    <Wrapper>
      <AppI18nProvider>
        <PlatformProvider adapter={createStubPlatform()}>
          <TooltipProvider>
            <WorkspaceSidebar user={USER} onSignOut={() => undefined} />
            <WorkspaceView userName={USER.name} />
          </TooltipProvider>
        </PlatformProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
}

/**
 * Opens a worktree's new-chat surface through that Task row's create menu.
 * Row click alone only toggles expand and does not select a composer.
 */
async function openTaskComposer(
  user: ReturnType<typeof userEvent.setup>,
  title: string,
) {
  const label = await screen.findByText(title);
  const row = label.closest(".group\\/tree");
  expect(row).not.toBeNull();
  await user.click(label);
  await user.click(
    within(row as HTMLElement).getByRole("button", {
      name: /在此任务中新建|Create in this task/,
    }),
  );
  await user.click(
    await screen.findByRole("button", { name: /新建任务|New task/ }),
  );
}

/** The collapsed picker, which names the agent the selected surface is on. */
function picker() {
  return screen.getByRole("button", { name: /选择模型|Select model/ });
}

/**
 * Opens the composer's picker, clicks the named agent, then closes the menu.
 *
 * Choosing an agent deliberately leaves the menu open, so the same label is on
 * screen twice until it is dismissed. Closing here keeps each assertion about
 * what the picker settled on rather than what the open list still offers.
 */
async function pickAgent(
  user: ReturnType<typeof userEvent.setup>,
  agentLabel: RegExp,
) {
  await user.click(picker());
  const menu = await screen.findByRole("menu");
  await user.click(within(menu).getByText(agentLabel));
  await user.keyboard("{Escape}");
}

describe("agent picker isolation across real sidebar navigation", () => {
  it("keeps each task's picked agent stable when switching via real clicks", async () => {
    const user = userEvent.setup();
    renderWorkspace();

    await openTaskComposer(user, "Task One");
    await pickAgent(user, /Claude Code/);
    expect(within(picker()).getByText("Claude Code")).not.toBeNull();

    await openTaskComposer(user, "Task Two");
    await pickAgent(user, /OpenCode/);
    expect(within(picker()).getByText("OpenCode")).not.toBeNull();

    await openTaskComposer(user, "Task One");
    expect(within(picker()).getByText("Claude Code")).not.toBeNull();

    await openTaskComposer(user, "Task Two");
    expect(within(picker()).getByText("OpenCode")).not.toBeNull();

    await openTaskComposer(user, "Task One");
    expect(within(picker()).getByText("Claude Code")).not.toBeNull();
  }, 15_000);
});
