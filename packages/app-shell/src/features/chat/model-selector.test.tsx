import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "@ora/platform";
import { describe, expect, it, beforeEach } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createHookWrapper, createTestQueryClient } from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { createChatStore } from "@ora/chat";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useSettingsStore, DEFAULT_SETTINGS } from "../../state/stores/settings-store";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { ModelSelector } from "./model-selector";

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useSettingsStore.setState({ settings: DEFAULT_SETTINGS });
  usePendingAgentStore.setState({ selections: {} });
});

function renderModelSelector() {
  const state = createMockClientState();
  const client = createMockClient(state);
  const chatStore = createChatStore(client.session);
  const Wrapper = createHookWrapper(client, createTestQueryClient(), chatStore);
  render(
    <Wrapper>
      <AppI18nProvider>
        <PlatformProvider adapter={createStubPlatform()}>
          <TooltipProvider>
            <ModelSelector />
          </TooltipProvider>
        </PlatformProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
}

/** Opens the picker dropdown and clicks the named agent's entry. */
async function pickAgent(user: ReturnType<typeof userEvent.setup>, agentLabel: RegExp) {
  await user.click(screen.getByRole("button", { name: /选择模型|Select model/ }));
  const menu = await screen.findByRole("menu");
  await user.click(within(menu).getByText(agentLabel));
}

describe("ModelSelector agent isolation across not-yet-started chats", () => {
  it("keeps one task's picked agent stable while another task's pick changes", async () => {
    const user = userEvent.setup();
    renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    await pickAgent(user, /Claude Code/);
    expect(screen.getByText("Claude Code")).not.toBeNull();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t2", "p1"));
    await pickAgent(user, /OpenCode/);
    expect(screen.getByText("OpenCode")).not.toBeNull();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    expect(screen.getByText("Claude Code")).not.toBeNull();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t2", "p1"));
    expect(screen.getByText("OpenCode")).not.toBeNull();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    expect(screen.getByText("Claude Code")).not.toBeNull();
  });
});
