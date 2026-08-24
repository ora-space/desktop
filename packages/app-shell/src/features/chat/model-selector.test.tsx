import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "../../platform";
import { describe, expect, it, beforeEach, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { createChatStore } from "@ora/chat";
import {
  createMockClient,
  createMockClientState,
  type MockClientState,
} from "../../test/mock-client";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  useSettingsStore,
  DEFAULT_SETTINGS,
} from "../../state/stores/settings-store";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import type { AgentStatus } from "@ora/contracts";
import { ModelSelector } from "./model-selector";
import { queryKeys } from "../../state/hooks/query-keys";
import { useAgentModelStore } from "../../state/stores/agent-model-store";

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: "ora-space.opencode" },
  });
  usePendingAgentStore.setState({ selections: {} });
  useAgentModelStore.setState({ known: {} });
});

/** Replaces what the runtime reports about OpenCode, leaving every other agent detected. */
function reportOpenCode(status: AgentStatus) {
  return (state: MockClientState) => {
    state.agentRuntimeStatuses = state.agentRuntimeStatuses.map((candidate) =>
      candidate.agentRef === "ora-space.opencode"
        ? { ...candidate, status }
        : candidate,
    );
  };
}

function renderModelSelector(
  seed: (state: MockClientState) => void = () => {},
) {
  const state = createMockClientState();
  seed(state);
  const client = createMockClient(state);
  const warm = vi.spyOn(client.session, "warm");
  const chatStore = createChatStore(client.session);
  const queryClient = createTestQueryClient();
  const Wrapper = createHookWrapper(client, queryClient, chatStore);
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
  return { queryClient, state, warm };
}

/** The collapsed trigger, which names the agent this surface is currently on. */
function picker() {
  return screen.getByRole("button", { name: /选择模型|Select model/ });
}

/**
 * Opens the picker, clicks the named agent, then closes the menu.
 *
 * Choosing an agent deliberately leaves the menu open, so the same label is on
 * screen twice until it is dismissed. Closing here keeps each assertion about
 * what the trigger settled on rather than what the open list still offers.
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

describe("ModelSelector agent isolation across not-yet-started chats", () => {
  it("keeps one task's picked agent stable while another task's pick changes", async () => {
    const user = userEvent.setup();
    renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    await pickAgent(user, /Claude Code/);
    expect(within(picker()).getByText("Claude Code")).not.toBeNull();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t2", "p1"));
    await pickAgent(user, /OpenCode/);
    expect(within(picker()).getByText("OpenCode")).not.toBeNull();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    expect(within(picker()).getByText("Claude Code")).not.toBeNull();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t2", "p1"));
    expect(within(picker()).getByText("OpenCode")).not.toBeNull();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    expect(within(picker()).getByText("Claude Code")).not.toBeNull();
  });
});

/**
 * Opens the picker and returns its agent list, which keeps re-rendering as queries settle.
 *
 * Assertions are made against this element rather than by reopening the menu: which agents are
 * offered depends on the installed-plugin snapshot, and the loading answer is the whole catalog.
 */
async function openAgentList(user: ReturnType<typeof userEvent.setup>) {
  await user.click(picker());
  return await screen.findByRole("menu");
}

describe("ModelSelector agent availability", () => {
  it("offers every agent the runtime reports reaching", async () => {
    const user = userEvent.setup();
    renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);

    await waitFor(() =>
      expect(within(menu).queryByText("OpenCode")).not.toBeNull(),
    );
    expect(within(menu).queryByText("Claude Code")).not.toBeNull();
  });

  it("withholds an agent whose runtime nothing answered for", async () => {
    const user = userEvent.setup();
    renderModelSelector(reportOpenCode("unavailable"));

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);

    await waitFor(() =>
      expect(within(menu).queryByText("OpenCode")).toBeNull(),
    );
    expect(within(menu).queryByText("Claude Code")).not.toBeNull();
  });

  it("withholds an agent nothing supervises at all", async () => {
    const user = userEvent.setup();
    renderModelSelector((state) => {
      state.agentRuntimeStatuses = state.agentRuntimeStatuses.filter(
        (status) => status.agentRef !== "ora-space.opencode",
      );
    });

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);

    await waitFor(() =>
      expect(within(menu).queryByText("OpenCode")).toBeNull(),
    );
    expect(within(picker()).queryByText("OpenCode")).toBeNull();
    expect(picker().querySelectorAll("svg")).toHaveLength(1);
    expect(useSettingsStore.getState().settings.agentCli).toBe(
      "ora-space.opencode",
    );
  });

  it("keeps a stored choice when its agent is temporarily unavailable", async () => {
    renderModelSelector(reportOpenCode("unavailable"));

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));

    await waitFor(() =>
      expect(within(picker()).queryByText("OpenCode")).toBeNull(),
    );
    expect(useSettingsStore.getState().settings.agentCli).toBe(
      "ora-space.opencode",
    );
    expect(picker().querySelectorAll("svg")).toHaveLength(1);
  });

  it("removes a disabled agent's previously warmed models", async () => {
    const user = userEvent.setup();
    const { queryClient, state } = renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText("Big Pickle")).not.toBeNull(),
    );
    await waitFor(() =>
      expect(queryClient.getQueryData(queryKeys.agentRuntimeStatus)).toEqual(
        state.agentRuntimeStatuses,
      ),
    );

    reportOpenCode("unavailable")(state);
    await act(() =>
      queryClient.invalidateQueries({ queryKey: queryKeys.agentRuntimeStatus }),
    );

    await waitFor(() =>
      expect(within(menu).queryByText("OpenCode")).toBeNull(),
    );
    expect(within(menu).queryByText("Big Pickle")).toBeNull();
    expect(within(menu).queryByText("Small Pickle")).toBeNull();
    expect(within(picker()).queryByText("Big Pickle")).toBeNull();
    expect(within(picker()).queryByText("OpenCode")).toBeNull();
    expect(picker().querySelectorAll("svg")).toHaveLength(1);
  });

  it("does not select a newly enabled agent for an untouched surface", async () => {
    useSettingsStore.setState({ settings: { ...DEFAULT_SETTINGS } });
    const { queryClient, state } = renderModelSelector(
      reportOpenCode("unavailable"),
    );

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    expect(within(picker()).queryByText("OpenCode")).toBeNull();
    expect(within(picker()).queryByText("NGA")).toBeNull();
    await waitFor(() =>
      expect(queryClient.getQueryData(queryKeys.agentRuntimeStatus)).toEqual(
        state.agentRuntimeStatuses,
      ),
    );

    reportOpenCode("ready")(state);
    await act(() =>
      queryClient.invalidateQueries({ queryKey: queryKeys.agentRuntimeStatus }),
    );

    const user = userEvent.setup();
    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText("OpenCode")).not.toBeNull(),
    );
    expect(within(picker()).queryByText("OpenCode")).toBeNull();
    expect(within(picker()).queryByText("NGA")).toBeNull();
  });

  it("waits for an enabled plugin to become ready before warming its models", async () => {
    const user = userEvent.setup();
    const { queryClient, state, warm } = renderModelSelector(
      reportOpenCode("unavailable"),
    );

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    await waitFor(() =>
      expect(queryClient.getQueryData(queryKeys.agentRuntimeStatus)).toEqual(
        state.agentRuntimeStatuses,
      ),
    );
    expect(warm).not.toHaveBeenCalled();

    reportOpenCode("starting")(state);
    await act(() =>
      queryClient.invalidateQueries({ queryKey: queryKeys.agentRuntimeStatus }),
    );
    await waitFor(() =>
      expect(queryClient.getQueryData(queryKeys.agentRuntimeStatus)).toEqual(
        state.agentRuntimeStatuses,
      ),
    );
    expect(warm).not.toHaveBeenCalled();

    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText(/加载中|Loading/)).not.toBeNull(),
    );

    reportOpenCode("ready")(state);
    await act(() =>
      queryClient.invalidateQueries({ queryKey: queryKeys.agentRuntimeStatus }),
    );

    await waitFor(() => expect(warm).toHaveBeenCalledOnce());
    await waitFor(() =>
      expect(within(menu).queryByText("Big Pickle")).not.toBeNull(),
    );
  });
});
