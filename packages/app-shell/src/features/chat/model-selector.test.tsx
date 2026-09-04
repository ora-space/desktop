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
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createWorkspaceMemory,
  workspaceHandlers,
} from "../../test/memory/workspaces";
import {
  createSessionMemory,
  sessionHandlers,
} from "../../test/memory/sessions";
import {
  createAgentRuntimeMemory,
  agentRuntimeHandlers,
} from "../../test/memory/agent-runtime";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import "../../i18n/i18n-instance";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useUiStore } from "../../state/stores/ui-store";
import {
  useSettingsStore,
  DEFAULT_SETTINGS,
} from "../../state/stores/settings-store";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { useAgentModelPreferenceStore } from "../../state/stores/agent-model-preference-store";
import type { AgentStatus } from "@ora/contracts";
import { ModelSelector } from "./model-selector";
import { agentRuntimeKeys } from "../../state/data/agent-runtime";
import { AGENT_REF } from "../../test/agent-identity";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return {
    ...createWorkspaceMemory(),
    ...createSessionMemory(),
    ...createAgentRuntimeMemory(),
    ...createPluginMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...workspaceHandlers(state),
    ...sessionHandlers(state),
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
  };
}

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: AGENT_REF.opencode },
  });
  usePendingAgentStore.setState({ selections: {}, switches: {}, models: {} });
  useAgentModelPreferenceStore.setState({ models: {} });
});

/** Replaces what the runtime reports about OpenCode, leaving every other agent detected. */
function reportOpenCode(status: AgentStatus) {
  return (state: FixtureState) => {
    state.agentRuntimeStatuses = state.agentRuntimeStatuses.map((candidate) =>
      candidate.agentRef === AGENT_REF.opencode
        ? { ...candidate, status }
        : candidate,
    );
  };
}

function renderModelSelector(seed: (state: FixtureState) => void = () => {}) {
  const state = createFixtureState();
  state.tasks = [
    {
      id: "t1",
      projectId: "p1",
      workspaceId: "workspace-t1",
      title: "Task 1",
    },
    {
      id: "t2",
      projectId: "p1",
      workspaceId: "workspace-t2",
      title: "Task 2",
    },
  ];
  state.workspaces = state.tasks.map((task) => ({
    id: task.workspaceId,
    projectId: task.projectId,
    kind: "isolated" as const,
    lifecycle: "active" as const,
  }));
  seed(state);
  const clientHandlers: TestHandlers = createFixtureHandlers(state);
  const client = createTestClient(clientHandlers);
  const discover = vi.spyOn(client.agentRuntime, "listModels");
  const setConfig = vi.spyOn(client.session, "setConfig");
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
  return { queryClient, state, discover, setConfig, chatStore };
}

/** One persisted session bound to OpenCode, selected as the surface the picker reads. */
function selectPersistedSession(state: FixtureState) {
  state.sessions = [
    {
      id: "s1",
      workspaceId: "workspace-t1",
      title: null,
      agentRef: AGENT_REF.opencode,
      status: "stopped",
      historyState: { type: "writable" },
    },
  ];
  useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
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

/**
 * Replays the conversation the way opening one does, which reports no options.
 *
 * A load that finishes without them is what settles the session as unattached; before it does,
 * "not answered yet" must not read as "has no provider".
 */
async function openDetachedConversation(
  chatStore: ReturnType<typeof createChatStore>,
) {
  await act(async () => {
    await chatStore.getState().loadSession("s1");
  });
}

describe("ModelSelector for a session that has not attached", () => {
  /**
   * Reading a conversation never reaches its agent, so a persisted session can be open with no
   * provider and nothing to configure. The picker is then in exactly the position of a chat that
   * has not started, and asking the agent for its own catalog is the only way to offer anything.
   */
  it("offers the agent's own catalog while the session reports no options", async () => {
    const { discover, chatStore } = renderModelSelector(selectPersistedSession);
    await openDetachedConversation(chatStore);

    await waitFor(() =>
      expect(discover).toHaveBeenCalledWith(
        expect.objectContaining({ agentRef: AGENT_REF.opencode }),
      ),
    );
    const user = userEvent.setup();
    await user.click(picker());
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByText(/Big Pickle/)).toBeInTheDocument();
  });

  /**
   * The pick has nowhere to go while no provider exists, so it is recorded and carried by the
   * message that attaches one. Writing it through `setSessionConfig` would address a provider
   * session the agent has never heard of.
   */
  it("records a pick as an intent instead of configuring an absent provider", async () => {
    const { setConfig, chatStore } = renderModelSelector(
      selectPersistedSession,
    );
    await openDetachedConversation(chatStore);
    const user = userEvent.setup();

    await user.click(picker());
    const menu = await screen.findByRole("menu");
    await user.click(within(menu).getByText(/Small Pickle/));

    await waitFor(() =>
      expect(
        usePendingAgentStore.getState().models[
          `session:s1|agent:${AGENT_REF.opencode}`
        ],
      ).toEqual("opencode/small-pickle"),
    );
    expect(setConfig).not.toHaveBeenCalled();
  });

  /**
   * Options are what say a provider is there to configure. Once one reports them the session is
   * authoritative about its own model, and the agent's pre-session catalog must not be asked for
   * or shown — it lists what the agent can serve, not what this conversation is running.
   */
  it("configures the live session once it reports options of its own", async () => {
    const { discover, setConfig, chatStore } = renderModelSelector(
      selectPersistedSession,
    );
    act(() => {
      chatStore.getState().setConfigOptions("s1", [
        {
          id: "model",
          name: "Model",
          category: "model",
          type: "select",
          currentValue: "session/current",
          options: [
            { value: "session/current", name: "Session Current" },
            { value: "session/other", name: "Session Other" },
          ],
        },
      ]);
    });

    const user = userEvent.setup();
    await user.click(picker());
    const menu = await screen.findByRole("menu");
    await user.click(within(menu).getByText(/Session Other/));

    await waitFor(() =>
      expect(setConfig).toHaveBeenCalledWith(
        expect.objectContaining({ sessionId: "s1", value: "session/other" }),
      ),
    );
    expect(discover).not.toHaveBeenCalled();
  });
});

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
  it("shows the agent plugin title instead of its contribution identifier", async () => {
    const user = userEvent.setup();
    renderModelSelector((state) => {
      const plugin = state.installedPlugins.find(
        (candidate) => candidate.id === AGENT_REF.opencode,
      );
      if (plugin?.kind === "agent") {
        plugin.displayName = "OpenCode Plugin";
        plugin.agentDisplayName = AGENT_REF.opencode;
      }
    });

    const menu = await openAgentList(user);

    await waitFor(() =>
      expect(within(menu).getByText("OpenCode Plugin")).not.toBeNull(),
    );
    expect(within(menu).queryByText(AGENT_REF.opencode)).toBeNull();
  });

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
        (status) => status.agentRef !== AGENT_REF.opencode,
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
      AGENT_REF.opencode,
    );
  });

  it("keeps a stored choice when its agent is temporarily unavailable", async () => {
    renderModelSelector(reportOpenCode("unavailable"));

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));

    await waitFor(() =>
      expect(within(picker()).queryByText("OpenCode")).toBeNull(),
    );
    expect(useSettingsStore.getState().settings.agentCli).toBe(
      AGENT_REF.opencode,
    );
    expect(picker().querySelectorAll("svg")).toHaveLength(1);
  });

  it("removes a disabled agent's previously discovered models", async () => {
    const user = userEvent.setup();
    const { queryClient, state } = renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText("Big Pickle")).not.toBeNull(),
    );
    await waitFor(() =>
      expect(
        queryClient.getQueryData(agentRuntimeKeys.agentRuntimeStatus),
      ).toEqual(state.agentRuntimeStatuses),
    );

    reportOpenCode("unavailable")(state);
    await act(() =>
      queryClient.invalidateQueries({
        queryKey: agentRuntimeKeys.agentRuntimeStatus,
      }),
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
      expect(
        queryClient.getQueryData(agentRuntimeKeys.agentRuntimeStatus),
      ).toEqual(state.agentRuntimeStatuses),
    );

    reportOpenCode("ready")(state);
    await act(() =>
      queryClient.invalidateQueries({
        queryKey: agentRuntimeKeys.agentRuntimeStatus,
      }),
    );

    const user = userEvent.setup();
    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText("OpenCode")).not.toBeNull(),
    );
    expect(within(picker()).queryByText("OpenCode")).toBeNull();
    expect(within(picker()).queryByText("NGA")).toBeNull();
  });

  it("waits for an installed plugin to become ready before discovering its models", async () => {
    const user = userEvent.setup();
    const { queryClient, state, discover } = renderModelSelector(
      reportOpenCode("unavailable"),
    );

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    await waitFor(() =>
      expect(
        queryClient.getQueryData(agentRuntimeKeys.agentRuntimeStatus),
      ).toEqual(state.agentRuntimeStatuses),
    );
    expect(discover).not.toHaveBeenCalled();

    reportOpenCode("starting")(state);
    await act(() =>
      queryClient.invalidateQueries({
        queryKey: agentRuntimeKeys.agentRuntimeStatus,
      }),
    );
    await waitFor(() =>
      expect(
        queryClient.getQueryData(agentRuntimeKeys.agentRuntimeStatus),
      ).toEqual(state.agentRuntimeStatuses),
    );
    expect(discover).not.toHaveBeenCalled();

    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText(/加载中|Loading/)).not.toBeNull(),
    );

    reportOpenCode("ready")(state);
    await act(() =>
      queryClient.invalidateQueries({
        queryKey: agentRuntimeKeys.agentRuntimeStatus,
      }),
    );

    await waitFor(() => expect(discover).toHaveBeenCalledOnce());
    await waitFor(() =>
      expect(within(menu).queryByText("Big Pickle")).not.toBeNull(),
    );
  });
});

describe("ModelSelector model search", () => {
  it("filters the model list by typed text without the menu swallowing keystrokes", async () => {
    const user = userEvent.setup();
    renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText("Big Pickle")).not.toBeNull(),
    );

    const search = within(menu).getByRole("textbox", {
      name: /搜索模型|Search models/,
    });
    await user.type(search, "Small");
    expect(search).toHaveValue("Small");
    expect(within(menu).queryByText("Big Pickle")).toBeNull();
    expect(within(menu).queryByText("Small Pickle")).not.toBeNull();

    await user.clear(search);
    expect(within(menu).queryByText("Big Pickle")).not.toBeNull();
    expect(within(menu).queryByText("Small Pickle")).not.toBeNull();
  });

  it("reports no matches for a query nothing in the model list satisfies", async () => {
    const user = userEvent.setup();
    renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText("Big Pickle")).not.toBeNull(),
    );

    const search = within(menu).getByRole("textbox", {
      name: /搜索模型|Search models/,
    });
    await user.type(search, "nonexistent-model");

    expect(within(menu).queryByText("Big Pickle")).toBeNull();
    expect(within(menu).queryByText("Small Pickle")).toBeNull();
    await waitFor(() =>
      expect(
        within(menu).queryByText(/未找到匹配的模型|No matching models/),
      ).not.toBeNull(),
    );
  });

  it("does not move focus into the search box just from opening the menu", async () => {
    const user = userEvent.setup();
    renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText("Big Pickle")).not.toBeNull(),
    );

    const search = within(menu).getByRole("textbox", {
      name: /搜索模型|Search models/,
    });
    expect(document.activeElement).not.toBe(search);
  });

  it("does not offer a search box in the agent group", async () => {
    const user = userEvent.setup();
    renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);
    await waitFor(() =>
      expect(within(menu).queryByText("Big Pickle")).not.toBeNull(),
    );

    expect(
      within(menu).getAllByRole("textbox", {
        name: /搜索模型|Search models/,
      }),
    ).toHaveLength(1);
  });
});

describe("ModelSelector with no agent package installed", () => {
  it("offers the install hint and opens the plugin marketplace on click", async () => {
    useUiStore.setState({ settingsCategory: "appearance" });
    const user = userEvent.setup();
    renderModelSelector((state) => {
      state.installedPlugins = [];
      state.agentRuntimeStatuses = [];
    });

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    const menu = await openAgentList(user);

    const hint =
      /Install an agent from the plugin marketplace|从插件市场安装 Agent/;
    await waitFor(() => expect(within(menu).queryByText(hint)).not.toBeNull());
    expect(within(menu).queryByText("OpenCode")).toBeNull();

    await user.click(within(menu).getByText(hint));
    expect(useUiStore.getState().settingsOpen).toBe(true);
    expect(useUiStore.getState().settingsCategory).toBe("plugins");
  });
});

/** Opens the picker, clicks the named model, and lets the menu close on the pick. */
async function pickModel(
  user: ReturnType<typeof userEvent.setup>,
  modelLabel: string,
) {
  await user.click(picker());
  const menu = await screen.findByRole("menu");
  await waitFor(() =>
    expect(within(menu).queryByText(modelLabel)).not.toBeNull(),
  );
  await user.click(within(menu).getByText(modelLabel));
}

/** A second agent whose catalog shares no model with OpenCode's. */
function claudeModels(state: FixtureState) {
  state.agentModelsByCli = {
    [AGENT_REF.claude]: [
      {
        id: "model",
        name: "Model",
        category: "model",
        type: "select",
        currentValue: "claude/sonnet",
        options: [
          { value: "claude/sonnet", name: "Sonnet" },
          { value: "claude/opus", name: "Opus" },
        ],
      },
    ],
  };
}

describe("ModelSelector remembered model for not-yet-started chats", () => {
  it("opens a new chat on the model last picked for its agent", async () => {
    const user = userEvent.setup();
    renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    await pickModel(user, "Small Pickle");
    await waitFor(() =>
      expect(within(picker()).queryByText("Small Pickle")).not.toBeNull(),
    );

    act(() => useWorkspaceSelectionStore.getState().selectTask("t2", "p1"));

    await waitFor(() =>
      expect(within(picker()).queryByText("Small Pickle")).not.toBeNull(),
    );
  });

  it("remembers one model per agent instead of one across agents", async () => {
    const user = userEvent.setup();
    renderModelSelector(claudeModels);

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    await pickModel(user, "Small Pickle");
    await pickAgent(user, /Claude Code/);
    await pickModel(user, "Opus");
    await waitFor(() =>
      expect(within(picker()).queryByText("Opus")).not.toBeNull(),
    );

    // An untouched surface follows the agent picked most recently, so this one
    // opens on Claude Code and must show Claude Code's remembered model.
    act(() => useWorkspaceSelectionStore.getState().selectTask("t2", "p1"));
    await waitFor(() =>
      expect(within(picker()).queryByText("Opus")).not.toBeNull(),
    );

    await pickAgent(user, /OpenCode/);

    await waitFor(() =>
      expect(within(picker()).queryByText("Small Pickle")).not.toBeNull(),
    );
  });

  it("falls back to the agent's default when the remembered model is gone", async () => {
    useAgentModelPreferenceStore.setState({
      models: { [AGENT_REF.opencode]: "opencode/retired-pickle" },
    });
    renderModelSelector();

    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));

    await waitFor(() =>
      expect(within(picker()).queryByText("Big Pickle")).not.toBeNull(),
    );
  });

  it("leaves a session that has already started on its own model", async () => {
    useAgentModelPreferenceStore.setState({
      models: { [AGENT_REF.opencode]: "opencode/small-pickle" },
    });
    const { chatStore } = renderModelSelector((state) => {
      state.sessions = [
        {
          id: "s1",
          workspaceId: "workspace-t1",
          title: "Started chat",
          agentRef: AGENT_REF.opencode,
          status: "running",
          historyState: { type: "writable" },
        },
      ];
    });

    act(() => {
      chatStore.getState().initializeSession("s1");
      chatStore.getState().setConfigOptions("s1", [
        {
          id: "model",
          name: "Model",
          category: "model",
          type: "select",
          currentValue: "opencode/big-pickle",
          options: [
            { value: "opencode/big-pickle", name: "Big Pickle" },
            { value: "opencode/small-pickle", name: "Small Pickle" },
          ],
        },
      ]);
      useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    });

    await waitFor(() =>
      expect(within(picker()).queryByText("Big Pickle")).not.toBeNull(),
    );
    expect(within(picker()).queryByText("Small Pickle")).toBeNull();
  });
});
