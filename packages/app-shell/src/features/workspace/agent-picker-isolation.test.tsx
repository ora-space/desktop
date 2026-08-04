import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "@ora/platform";
import { describe, expect, it, beforeEach } from "vitest";
import type { Project, Task } from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { AppI18nProvider } from "../../i18n/i18n";
import { createHookWrapper, createTestQueryClient } from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useSettingsStore, DEFAULT_SETTINGS } from "../../state/stores/settings-store";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { WorkspaceSidebar } from "./workspace-sidebar";
import { WorkspaceView } from "./workspace-view";

const USER = { name: "Eric", email: "eric@example.com" };
const PROJECT: Project = { id: "p1", name: "Ora Desktop", rootPath: "/ora" };
const TASK1: Task = { id: "t1", projectId: "p1", title: "Task One", status: "todo", workspaceMode: "worktree" };
const TASK2: Task = { id: "t2", projectId: "p1", title: "Task Two", status: "todo", workspaceMode: "worktree" };

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useSettingsStore.setState({ settings: DEFAULT_SETTINGS });
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

/** Clicks a task row in the sidebar tree by its visible title. */
async function clickTask(user: ReturnType<typeof userEvent.setup>, title: string) {
  await user.click(await screen.findByText(title));
}

/** Opens the composer's picker dropdown and clicks the named agent's entry. */
async function pickAgent(user: ReturnType<typeof userEvent.setup>, agentLabel: RegExp) {
  await user.click(screen.getByRole("button", { name: /选择模型|Select model/ }));
  const menu = await screen.findByRole("menu");
  await user.click(within(menu).getByText(agentLabel));
}

describe("agent picker isolation across real sidebar navigation", () => {
  it("keeps each task's picked agent stable when switching via real clicks", async () => {
    const user = userEvent.setup();
    renderWorkspace();

    await clickTask(user, "Task One");
    await pickAgent(user, /Claude Code/);
    expect(screen.getByText("Claude Code")).not.toBeNull();

    await clickTask(user, "Task Two");
    await pickAgent(user, /OpenCode/);
    expect(screen.getByText("OpenCode")).not.toBeNull();

    await clickTask(user, "Task One");
    expect(screen.getByText("Claude Code")).not.toBeNull();

    await clickTask(user, "Task Two");
    expect(screen.getByText("OpenCode")).not.toBeNull();

    await clickTask(user, "Task One");
    expect(screen.getByText("Claude Code")).not.toBeNull();
  });
});
