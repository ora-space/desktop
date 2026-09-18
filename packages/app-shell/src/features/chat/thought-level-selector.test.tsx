import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TooltipProvider } from "@ora/ui";
import type * as acp from "@agentclientprotocol/sdk";
import { PlatformProvider } from "../../platform";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
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
import {
  useSettingsStore,
  DEFAULT_SETTINGS,
} from "../../state/stores/settings-store";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { ThoughtLevelSelector } from "./thought-level-selector";
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
    // Replays the options the persisted conversation was left with, the way the
    // backend answers a load for a session that still has a provider behind it.
    loadSession: async function* () {
      if (state.configOptions.length > 0) {
        yield {
          type: "session_update" as const,
          update: {
            sessionUpdate: "config_option_update" as const,
            configOptions: state.configOptions,
          },
        };
      }
      yield { type: "completed" as const };
    },
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
  };
}

/** The levels Claude Code reports, without the `default` pointer it lists in front of them. */
const LEVELS: acp.SessionConfigSelectOption[] = [
  { value: "low", name: "Low" },
  { value: "high", name: "High" },
  { value: "max", name: "Max" },
];

/** Builds the effort selector as an agent reports it alongside its model option. */
function thoughtLevelOption(
  options: acp.SessionConfigSelectOption[],
  currentValue = "high",
): acp.SessionConfigOption {
  return {
    id: "effort",
    name: "Effort",
    description: "Available effort levels for this model",
    category: "thought_level",
    type: "select",
    currentValue,
    options,
  };
}

const THOUGHT_LEVEL_OPTION = thoughtLevelOption(LEVELS);

/** The selector as Claude Code reports it, with its `default` pointer in front. */
const THOUGHT_LEVEL_WITH_DEFAULT = thoughtLevelOption([
  { value: "default", name: "Default" },
  ...LEVELS,
]);

const MODEL_OPTION: acp.SessionConfigOption = {
  id: "model",
  name: "Model",
  category: "model",
  type: "select",
  currentValue: "sonnet",
  options: [{ value: "sonnet", name: "Sonnet" }],
};

// jsdom implements pointer events but not pointer capture, which the slider
// takes on press and releases on lift; without these the press throws.
beforeAll(() => {
  const prototype = Element.prototype as Element & {
    setPointerCapture?: (pointerId: number) => void;
    releasePointerCapture?: (pointerId: number) => void;
    hasPointerCapture?: (pointerId: number) => boolean;
  };
  prototype.setPointerCapture ??= () => {};
  prototype.releasePointerCapture ??= () => {};
  prototype.hasPointerCapture ??= () => false;
});

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: AGENT_REF.opencode },
  });
  usePendingAgentStore.setState({ selections: {}, switches: {}, models: {} });
});

function renderThoughtLevelSelector(
  seed: (state: FixtureState) => void = () => {},
  handlers: Partial<TestHandlers> = {},
) {
  const state = createFixtureState();
  state.tasks = [
    { id: "t1", projectId: "p1", workspaceId: "workspace-t1", title: "Task 1" },
  ];
  state.workspaces = [
    {
      id: "workspace-t1",
      projectId: "p1",
      kind: "isolated" as const,
      lifecycle: "active" as const,
    },
  ];
  seed(state);
  const client = createTestClient({
    ...createFixtureHandlers(state),
    ...handlers,
  });
  const setConfig = vi.spyOn(client.session, "setConfig");
  const chatStore = createChatStore(client.session);
  const queryClient = createTestQueryClient();
  const Wrapper = createHookWrapper(client, queryClient, chatStore);
  render(
    <Wrapper>
      <AppI18nProvider>
        <PlatformProvider adapter={createStubPlatform()}>
          <TooltipProvider>
            <ThoughtLevelSelector />
          </TooltipProvider>
        </PlatformProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
  return { state, setConfig, chatStore };
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
      mcpSelection: { mode: "automatic" },
    },
  ];
  useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
}

/**
 * The slider's thumb, which is the element that receives keyboard steps.
 *
 * Queried with `hidden` because the thumb keeps `visibility: hidden` until it
 * has measured its own size, and jsdom performs no layout, so the thumb never
 * becomes "visible" here even though the popover it sits in is.
 */
async function sliderThumb() {
  return await screen.findByRole("slider", { hidden: true });
}

/**
 * A provider whose `setSessionConfig` answers only when told to, so a test can
 * inspect the control mid-round-trip. Each answer echoes the requested level
 * as the value in effect, the way an agent that accepted the pick reports it.
 */
function deferredProvider() {
  const pending: Array<{
    value: string;
    resolve: (response: { configOptions: acp.SessionConfigOption[] }) => void;
    reject: (error: Error) => void;
  }> = [];
  return {
    handlers: {
      setSessionConfig: (request) =>
        new Promise((resolve, reject) => {
          pending.push({ value: request.value, resolve, reject });
        }),
    } satisfies Partial<TestHandlers>,
    pending: () => [...pending],
    answer: async () => {
      const next = pending.shift();
      if (next === undefined) throw new Error("no request to answer");
      await act(async () =>
        next.resolve({
          configOptions: [MODEL_OPTION, thoughtLevelOption(LEVELS, next.value)],
        }),
      );
    },
    fail: async () => {
      const next = pending.shift();
      if (next === undefined) throw new Error("no request to fail");
      await act(async () => next.reject(new Error("agent unreachable")));
    },
  };
}

/**
 * Records every position the thumb passes through from now on.
 *
 * `aria-valuenow` is what the primitive writes on each value change, so a
 * snap-back — however brief — shows up as an extra entry in this list even
 * when the position has settled by the time an assertion looks.
 */
function recordPositions(thumb: HTMLElement) {
  const seen = [thumb.getAttribute("aria-valuenow") ?? ""];
  const observer = new MutationObserver(() => {
    const now = thumb.getAttribute("aria-valuenow") ?? "";
    if (now !== seen.at(-1)) seen.push(now);
  });
  observer.observe(thumb, {
    attributes: true,
    attributeFilter: ["aria-valuenow"],
  });
  return () => {
    observer.disconnect();
    return [...seen];
  };
}

/**
 * Presses and releases the pointer on the slider's band at the named level.
 *
 * The primitive derives the picked value from where the pointer lands on the
 * track, which jsdom lays out with no size, so the track is given a width for
 * the press and the pointer aimed at the level's share of it.
 */
async function clickBand(
  user: ReturnType<typeof userEvent.setup>,
  thumb: HTMLElement,
  level: (typeof LEVELS)[number]["value"],
) {
  const root = thumb.closest<HTMLElement>("[data-slot=step-slider]")!;
  const control = root.querySelector<HTMLElement>(
    "[data-slot=step-slider-track]",
  )!.parentElement!;
  const width = 300;
  const rect = {
    x: 0,
    y: 0,
    left: 0,
    top: 0,
    width,
    height: 32,
    right: width,
    bottom: 32,
    toJSON: () => ({}),
  } as DOMRect;
  const restore = [control, root].map((element) => {
    const original = element.getBoundingClientRect;
    element.getBoundingClientRect = () => rect;
    return () => {
      element.getBoundingClientRect = original;
    };
  });
  try {
    const index = LEVELS.findIndex((candidate) => candidate.value === level);
    const clientX = (index / (LEVELS.length - 1)) * width;
    await user.pointer([
      {
        keys: "[MouseLeft>]",
        target: control,
        coords: { clientX, clientY: 16 },
      },
      {
        keys: "[/MouseLeft]",
        target: control,
        coords: { clientX, clientY: 16 },
      },
    ]);
  } finally {
    restore.forEach((undo) => undo());
  }
}

function picker() {
  return screen.queryByRole("button", {
    name: /选择思考强度|Select thought level/,
  });
}

describe("ThoughtLevelSelector visibility", () => {
  /**
   * There is no pre-session catalog of effort levels, so a chat that has not
   * started has nothing trustworthy to show and shows nothing.
   */
  it("renders nothing for a chat that has not started", () => {
    renderThoughtLevelSelector();
    act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
    expect(picker()).toBeNull();
  });

  /** An agent that reports options without a `thought_level` selector offers no effort control. */
  it("renders nothing when the session reports no thought-level option", async () => {
    const { chatStore } = renderThoughtLevelSelector(selectPersistedSession);
    act(() => chatStore.getState().setConfigOptions("s1", [MODEL_OPTION]));
    await waitFor(() => expect(picker()).toBeNull());
  });

  /**
   * The provider's own report is the only source: once the first send's
   * handshake stores the session's options, the level in effect is named.
   */
  it("shows the level in effect once the session reports its options", async () => {
    const { chatStore } = renderThoughtLevelSelector(selectPersistedSession);
    act(() =>
      chatStore
        .getState()
        .setConfigOptions("s1", [MODEL_OPTION, THOUGHT_LEVEL_OPTION]),
    );
    const trigger = await screen.findByRole("button", {
      name: /选择思考强度|Select thought level/,
    });
    expect(within(trigger).getByText("High")).toBeInTheDocument();
  });

  /** Replaying a persisted conversation restores the option the same way the handshake reports it. */
  it("shows the level replayed from a persisted conversation", async () => {
    const { chatStore } = renderThoughtLevelSelector((state) => {
      selectPersistedSession(state);
      state.configOptions = [MODEL_OPTION, THOUGHT_LEVEL_OPTION];
    });
    await act(async () => {
      await chatStore.getState().loadSession("s1");
    });
    const trigger = await screen.findByRole("button", {
      name: /选择思考强度|Select thought level/,
    });
    expect(within(trigger).getByText("High")).toBeInTheDocument();
  });

  /**
   * A recorded agent move means the conversation's options describe the agent it
   * is leaving; the incoming one has not reported its levels yet.
   */
  it("withdraws while a move onto another agent is pending", async () => {
    const { chatStore } = renderThoughtLevelSelector(selectPersistedSession);
    act(() =>
      chatStore
        .getState()
        .setConfigOptions("s1", [MODEL_OPTION, THOUGHT_LEVEL_OPTION]),
    );
    await screen.findByRole("button", {
      name: /选择思考强度|Select thought level/,
    });

    act(() =>
      usePendingAgentStore.getState().setPendingSwitch("s1", AGENT_REF.claude),
    );
    await waitFor(() => expect(picker()).toBeNull());
  });
});

describe("ThoughtLevelSelector scale", () => {
  /**
   * `default` is a pointer to some other level, not a rung, and listing it
   * first would read as "below low" — while in fact it may resolve to high.
   * The scale spans only the real levels, so "high" is the middle of three.
   */
  it("leaves the agent's default entry off the scale", async () => {
    const { chatStore } = renderThoughtLevelSelector(selectPersistedSession);
    act(() =>
      chatStore
        .getState()
        .setConfigOptions("s1", [MODEL_OPTION, THOUGHT_LEVEL_WITH_DEFAULT]),
    );
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", {
        name: /选择思考强度|Select thought level/,
      }),
    );
    const thumb = await sliderThumb();
    expect(thumb).toHaveAttribute("max", "2");
    expect(thumb).toHaveAttribute("aria-valuenow", "1");
    expect(screen.queryByText("Default")).toBeNull();
  });

  /**
   * A session still on `default` sits on no rung: the label says so in the
   * agent's own words, the thumb is withheld instead of parked on "low", and
   * the first pick puts it somewhere real.
   */
  it("withholds the thumb while the session is on the agent's default", async () => {
    const { chatStore, setConfig } = renderThoughtLevelSelector(
      selectPersistedSession,
    );
    act(() =>
      chatStore
        .getState()
        .setConfigOptions("s1", [
          MODEL_OPTION,
          thoughtLevelOption(
            [{ value: "default", name: "Default" }, ...LEVELS],
            "default",
          ),
        ]),
    );
    const trigger = await screen.findByRole("button", {
      name: /选择思考强度|Select thought level/,
    });
    expect(within(trigger).getByText("Default")).toBeInTheDocument();

    const user = userEvent.setup();
    await user.click(trigger);
    const thumb = await sliderThumb();
    expect(thumb.closest("[data-slot=step-slider]")).toHaveAttribute(
      "data-unresolved",
    );

    act(() => thumb.focus());
    await user.keyboard("{ArrowRight}");
    await waitFor(() =>
      expect(setConfig).toHaveBeenCalledWith({
        sessionId: "s1",
        configId: "effort",
        value: "high",
      }),
    );
  });
});

describe("ThoughtLevelSelector selection", () => {
  /**
   * The slider spans the levels in the agent's order, so stepping the thumb
   * one notch down from "high" lands on "low". Releasing it — which a keyboard
   * step does at once — applies the level immediately, addressed by the option
   * id the agent reported.
   */
  it("configures the session with the level the slider is released on", async () => {
    const { chatStore, setConfig } = renderThoughtLevelSelector(
      selectPersistedSession,
    );
    act(() =>
      chatStore
        .getState()
        .setConfigOptions("s1", [MODEL_OPTION, THOUGHT_LEVEL_OPTION]),
    );
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", {
        name: /选择思考强度|Select thought level/,
      }),
    );
    const thumb = await sliderThumb();
    expect(thumb).toHaveAttribute("aria-valuenow", "1");

    act(() => thumb.focus());
    await user.keyboard("{ArrowLeft}");

    await waitFor(() =>
      expect(setConfig).toHaveBeenCalledWith({
        sessionId: "s1",
        configId: "effort",
        value: "low",
      }),
    );
  });

  /**
   * The thumb must not flinch on release. Until the provider answers, the
   * reported value is still the old one, so falling back to it would snap the
   * thumb back and then forward again when the answer lands; the chosen
   * position is held across the round trip instead. Nor may the control dim
   * or lock for the round trip — that reads as the same flicker.
   */
  it("keeps the thumb on the chosen level while the provider answers", async () => {
    const provider = deferredProvider();
    const { chatStore } = renderThoughtLevelSelector(
      selectPersistedSession,
      provider.handlers,
    );
    act(() =>
      chatStore
        .getState()
        .setConfigOptions("s1", [MODEL_OPTION, THOUGHT_LEVEL_OPTION]),
    );
    const user = userEvent.setup();
    const trigger = await screen.findByRole("button", {
      name: /选择思考强度|Select thought level/,
    });
    await user.click(trigger);
    const thumb = await sliderThumb();
    const positions = recordPositions(thumb);

    await clickBand(user, thumb, "max");

    await waitFor(() => expect(provider.pending()).toHaveLength(1));
    expect(thumb).toHaveAttribute("aria-valuenow", "2");
    expect(thumb).not.toBeDisabled();
    expect(within(trigger).getByText("Max")).toBeInTheDocument();

    await provider.answer();
    expect(thumb).toHaveAttribute("aria-valuenow", "2");
    expect(positions()).toEqual(["1", "2"]);
    expect(chatStore.getState().conversations["s1"]?.configOptions).toEqual([
      MODEL_OPTION,
      thoughtLevelOption(LEVELS, "max"),
    ]);
  });

  /**
   * A second pick made before the first is answered overtakes it. The first
   * answer, arriving with its own value, must not pull the thumb back onto
   * that value while the newer request is still out.
   */
  it("lets a newer pick overtake one still in flight without snapping back", async () => {
    const provider = deferredProvider();
    const { chatStore } = renderThoughtLevelSelector(
      selectPersistedSession,
      provider.handlers,
    );
    act(() =>
      chatStore
        .getState()
        .setConfigOptions("s1", [MODEL_OPTION, THOUGHT_LEVEL_OPTION]),
    );
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", {
        name: /选择思考强度|Select thought level/,
      }),
    );
    const thumb = await sliderThumb();
    const positions = recordPositions(thumb);

    await clickBand(user, thumb, "max");
    await clickBand(user, thumb, "low");
    await waitFor(() => expect(provider.pending()).toHaveLength(2));

    await provider.answer();
    expect(thumb).toHaveAttribute("aria-valuenow", "0");
    await provider.answer();
    expect(thumb).toHaveAttribute("aria-valuenow", "0");
    expect(positions()).toEqual(["1", "2", "0"]);
  });

  /** A round trip that never reaches the agent leaves the level it still runs on in effect. */
  it("returns the thumb to the reported level when the request fails", async () => {
    const provider = deferredProvider();
    const { chatStore } = renderThoughtLevelSelector(
      selectPersistedSession,
      provider.handlers,
    );
    act(() =>
      chatStore
        .getState()
        .setConfigOptions("s1", [MODEL_OPTION, THOUGHT_LEVEL_OPTION]),
    );
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", {
        name: /选择思考强度|Select thought level/,
      }),
    );
    const thumb = await sliderThumb();

    await clickBand(user, thumb, "max");
    await waitFor(() => expect(provider.pending()).toHaveLength(1));
    expect(thumb).toHaveAttribute("aria-valuenow", "2");

    await provider.fail();
    expect(thumb).toHaveAttribute("aria-valuenow", "1");
    expect(chatStore.getState().conversations["s1"]?.error).toEqual(
      "agent unreachable",
    );
  });

  /** Releasing on the level already in effect is not a change and sends nothing. */
  it("sends nothing when released on the current level", async () => {
    const { chatStore, setConfig } = renderThoughtLevelSelector(
      selectPersistedSession,
    );
    act(() =>
      chatStore
        .getState()
        .setConfigOptions("s1", [MODEL_OPTION, THOUGHT_LEVEL_OPTION]),
    );
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", {
        name: /选择思考强度|Select thought level/,
      }),
    );
    const thumb = await sliderThumb();
    act(() => thumb.focus());
    await user.keyboard("{ArrowRight}{ArrowLeft}");

    await waitFor(() =>
      expect(setConfig).toHaveBeenCalledWith(
        expect.objectContaining({ value: "max" }),
      ),
    );
    expect(setConfig).toHaveBeenCalledTimes(1);
  });
});
