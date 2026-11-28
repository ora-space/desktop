import { useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createChatStore, type SessionConversation } from "@ora/chat";
import type { GraphWorkflowRun } from "@ora/workflow-runtime";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { createAgentMemory, agentHandlers } from "../../test/memory/agents";
import { appI18n } from "../../i18n/i18n-instance";
import { RunTheater } from "./run-theater";

beforeEach(async () => {
  await appI18n.changeLanguage("en-US");
  // Exercise history selection without coupling assertions to animation frame timing.
  const matchMedia = window.matchMedia.bind(window);
  vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
    ...matchMedia(query),
    matches:
      query === "(prefers-reduced-motion: reduce)" || matchMedia(query).matches,
  }));
});

afterEach(() => vi.restoreAllMocks());

function fixture(): GraphWorkflowRun {
  const now = "2026-09-28T10:00:00+08:00";
  return {
    id: "run",
    projectId: "project",
    definitionId: "definition",
    name: "Loop test",
    status: "running",
    createdAt: now,
    updatedAt: now,
    openHitls: [],
    definitionSnapshot: {
      id: "definition",
      name: "Loop test",
      description: "",
      updatedAt: now,
      viewport: { x: 0, y: 0, zoom: 1 },
      nodes: [
        {
          id: "start",
          type: "workflow",
          position: { x: 0, y: 0 },
          data: { kind: "start", title: "Start", description: "" },
        },
        {
          id: "loop",
          type: "workflow",
          position: { x: 300, y: 0 },
          data: {
            kind: "loop",
            title: "Review loop",
            description: "",
            loopConfig: {
              maxIterations: 4,
              variables: [],
              until: { logic: "and", conditions: [] },
              outputs: [],
            },
          },
        },
        {
          id: "writer",
          parentId: "loop",
          type: "workflow",
          position: { x: 10, y: 0 },
          data: {
            kind: "agent",
            containerId: "loop",
            title: "Writer",
            description: "",
            instruction: "Write this round",
          },
        },
        {
          id: "exit",
          parentId: "loop",
          type: "workflow",
          position: { x: 300, y: 0 },
          data: {
            kind: "loopExit",
            containerId: "loop",
            title: "Exit",
            description: "",
          },
        },
        {
          id: "out",
          type: "workflow",
          position: { x: 900, y: 0 },
          data: { kind: "output", title: "Output", description: "" },
        },
      ],
      edges: [
        { id: "s-l", source: "start", target: "loop" },
        { id: "l-o", source: "loop", target: "out" },
        { id: "w-e", source: "writer", target: "exit" },
      ],
    },
    nodeStates: {
      start: { status: "succeeded" },
      loop: { status: "running" },
      out: { status: "idle" },
    },
    rounds: [
      {
        id: "scope-1",
        parentLoopNodeId: "loop",
        parentLoopNodeRunId: "parent-run",
        roundIndex: 1,
        status: "succeeded",
        createdAt: now,
        updatedAt: now,
        nodeStates: {
          writer: {
            status: "succeeded",
            sessionId: "session-1",
            output: { summary: "first draft" },
          },
        },
      },
      {
        id: "scope-2",
        parentLoopNodeId: "loop",
        parentLoopNodeRunId: "parent-run",
        roundIndex: 2,
        status: "running",
        createdAt: now,
        updatedAt: now,
        nodeStates: {
          writer: {
            status: "running",
            sessionId: "session-2",
            output: { summary: "partial stream must stay hidden" },
          },
        },
      },
    ],
  };
}

function conversation(text: string): SessionConversation {
  return {
    configOptions: [],
    providerLost: false,
    modelChanges: [],
    historyNotices: [],
    availableCommands: [],
    sessionTitle: null,
    sessionUpdatedAt: null,
    isLoaded: true,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    usage: {
      context: { status: "hidden" },
      lastTurnTokens: { status: "none" },
    },
    error: null,
    turns: [
      {
        id: text,
        userMessage: {
          kind: "message",
          id: `${text}-user`,
          role: "user",
          content: "Write",
          createdAt: 1,
        },
        items: [
          {
            kind: "message",
            id: `${text}-reply`,
            role: "assistant",
            content: text,
            createdAt: 2,
          },
        ],
        status: "completed",
        stopReason: "end_turn",
        error: null,
        createdAt: 1,
      },
    ],
  };
}

function mount(
  run: GraphWorkflowRun,
  {
    initialFocus = "out",
    handlers = {},
  }: { initialFocus?: string | null; handlers?: TestHandlers } = {},
) {
  const state = { ...createPluginMemory(), ...createAgentMemory() };
  const client = createTestClient({
    ...pluginHandlers(state),
    ...agentHandlers(state),
    ...handlers,
  });
  const chat = createChatStore(client.session);
  chat.setState({
    conversations: {
      "session-1": conversation("FIRST SESSION"),
      "session-2": conversation("SECOND SESSION"),
    },
  });
  function View({ value }: { value: GraphWorkflowRun }) {
    const [focus, setFocus] = useState<string | null>(initialFocus);
    const [session, setSession] = useState<string | null>(null);
    return (
      <RunTheater
        run={value}
        focusNodeId={focus}
        onFocusNode={(id) => {
          setFocus(id);
          if (id !== session) setSession(null);
        }}
        onClearFocus={() => setFocus(null)}
        artifacts={[]}
        conversationByNodeId={new Map()}
        revealedArtifactId={null}
        onShowOverview={() => {}}
        sessionConversationNodeId={session}
        onSessionConversationNodeIdChange={setSession}
      />
    );
  }
  const view = render(<View value={run} />, {
    wrapper: createHookWrapper(client, createTestQueryClient(), chat),
  });
  return {
    ...view,
    update: (value: GraphWorkflowRun) => view.rerender(<View value={value} />),
  };
}

it("keeps the debug receipt visible on the terminal result until the editor is closed", async () => {
  const user = userEvent.setup();
  const run = fixture();
  run.nodeStates.loop.nodeRunId = "parent-run";
  const target = {
    nodeId: "loop",
    scopeId: "root",
    nodeRunId: "parent-run",
    round: null,
    index: null,
    containerNodeRunId: null,
  };
  const view = mount(run, {
    initialFocus: null,
    handlers: {
      getWorkflowDebugContext: async () => ({
        target,
        baseRevision: 0,
        graph: JSON.stringify(run.definitionSnapshot),
      }),
      getWorkflowDebugHistory: async () => ({ entries: [] }),
      previewWorkflowDebug: async () => ({
        target,
        baseRevision: 0,
        previewId: "p",
        keep: [],
        stop: [],
        invalidate: [],
        schedule: [],
        predictedBranch: null,
        workspace: "keep",
        externalActionsMayRepeat: true,
      }),
      commitWorkflowDebug: async ({ requestId }) => ({
        requestId,
        operationId: "op",
        baseRevision: 0,
        state: "applied",
        revision: 1,
      }),
    },
  });
  await user.click(screen.getByRole("button", { name: "Open node details" }));
  await user.click(
    screen.getByRole("button", { name: "Edit and resume from here" }),
  );
  fireEvent.change(await screen.findByRole("spinbutton"), {
    target: { value: "8" },
  });
  await user.click(screen.getByRole("button", { name: "Preview changes" }));
  await user.click(
    await screen.findByRole("button", { name: "Confirm and resume" }),
  );
  expect(await screen.findByText(/revision 1 is in effect/i)).toBeVisible();
  view.update({ ...run, status: "succeeded" });
  const inspector = screen.getByRole("complementary", { name: "Act details" });
  expect(within(inspector).getByText(/revision 1 is in effect/i)).toBeVisible();
  await user.click(
    within(inspector).getByRole("button", { name: "Close editor" }),
  );
  expect(
    screen.queryByRole("complementary", { name: "Act details" }),
  ).toBeNull();
});

it("leaves historical round tabs available without selecting a member of an older execution", async () => {
  const user = userEvent.setup();
  const run = fixture();
  run.nodeStates.loop.nodeRunId = "new-container";
  run.rounds![0].nodeStates.writer.nodeRunId = "old-writer";
  mount(run, { initialFocus: "loop" });
  await user.click(screen.getByRole("button", { name: "Open node details" }));
  const inspector = within(
    screen.getByRole("complementary", { name: "Act details" }),
  );
  const old = inspector.getByRole("tab", { name: "Round 1" });
  expect(old).toHaveAttribute("aria-selected", "false");
  expect(inspector.queryByText("old-writer")).toBeNull();
  await user.click(old);
  expect(old).toHaveAttribute("aria-selected", "true");
  expect(inspector.getByText("old-writer")).toBeVisible();
  expect(
    screen.getByRole("region", { name: "Round overview" }),
  ).toHaveTextContent("first draft");
});

it("groups the outer path, defaults to round one and keeps scope selection through live updates", async () => {
  const user = userEvent.setup();
  const run = fixture();
  const view = mount(run);
  const outer = document.querySelector(
    '[data-slot="theater-top-level-path"]',
  ) as HTMLElement;
  expect(
    within(outer).queryByRole("button", { name: /Writer/ }),
  ).not.toBeInTheDocument();
  await user.click(within(outer).getByRole("button", { name: /Review loop/ }));
  expect(screen.getByRole("button", { name: /Round 1 ·/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(
    screen.getByRole("region", { name: "Round overview" }),
  ).toHaveTextContent("first draft");
  await user.click(screen.getByRole("button", { name: /Round 2 ·/ }));
  expect(
    screen.queryByText("partial stream must stay hidden"),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /Round 1 ·/ }));
  const updated = structuredClone(run);
  updated.rounds!.push({ ...updated.rounds![1], id: "scope-3", roundIndex: 3 });
  view.update(updated);
  expect(screen.getByRole("button", { name: /Round 1 ·/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(screen.getByRole("button", { name: /Round 3 ·/ })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /Round 2 ·/ }));
  await user.click(within(outer).getByRole("button", { name: /Output/ }));
  await user.click(within(outer).getByRole("button", { name: /Review loop/ }));
  expect(screen.getByRole("button", { name: /Round 1 ·/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

it("switches the conversation to the selected round and exposes committed loop results", async () => {
  const user = userEvent.setup();
  const run = fixture();
  const view = mount(run);
  await user.click(screen.getByRole("button", { name: /Review loop:/ }));
  await user.click(screen.getByRole("button", { name: "Writer: Succeeded" }));
  await user.click(
    screen.getByRole("button", { name: "View node conversation" }),
  );
  expect(await screen.findByText("FIRST SESSION")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /Round 2 ·/ }));
  expect(await screen.findByText("SECOND SESSION")).toBeInTheDocument();
  expect(screen.queryByText("FIRST SESSION")).not.toBeInTheDocument();
  const finished = structuredClone(run);
  finished.status = "succeeded";
  finished.nodeStates.loop = {
    status: "succeeded",
    stopReason: "loop_exit",
    output: { summary: '{"result":"approved"}' },
  };
  finished.rounds![1].status = "succeeded";
  finished.rounds![1].nodeStates.exit = { status: "succeeded" };
  view.update(finished);
  await user.click(screen.getByRole("button", { name: "Loop results" }));
  expect(
    screen.getByRole("region", { name: "Loop results" }),
  ).toHaveTextContent('"result": "approved"');
  const result = within(screen.getByRole("region", { name: "Loop results" }));
  const toggle = result.getByRole("button", { pressed: false });
  await user.click(toggle);
  expect(toggle).toHaveAttribute("aria-pressed", "true");
});

it("keeps unexecuted branch alternatives out of the round path while allowing inspection", async () => {
  const user = userEvent.setup();
  mount(fixture());
  await user.click(screen.getByRole("button", { name: /Review loop:/ }));
  const path = screen.getByLabelText("Nodes executed this round");
  expect(
    within(path).getByRole("button", { name: "Writer: Succeeded" }),
  ).toBeInTheDocument();
  expect(
    within(path).queryByRole("button", { name: /Exit/ }),
  ).not.toBeInTheDocument();
  await user.click(screen.getByText("Not executed this round (1)"));
  await user.click(screen.getByRole("button", { name: /Exit: Not run/ }));
  expect(screen.getByRole("button", { name: /Exit: Not run/ })).toHaveAttribute(
    "aria-current",
    "step",
  );
});

it("shows budget completion and the final output without claiming the condition was met", async () => {
  const user = userEvent.setup();
  const run = fixture();
  run.status = "succeeded";
  run.nodeStates.loop = {
    status: "succeeded",
    stopReason: "loop_max_iterations",
    output: { summary: '{"result":"last draft"}' },
  };
  run.rounds![1].status = "succeeded";
  mount(run);
  await user.click(screen.getByRole("button", { name: /Review loop:/ }));
  await user.click(screen.getByRole("button", { name: "Loop results" }));
  const result = screen.getByRole("region", { name: "Loop results" });
  expect(result).toHaveTextContent(
    "Maximum rounds reached; continuing workflow",
  );
  expect(result).toHaveTextContent('"result": "last draft"');
  expect(result).not.toHaveTextContent("end condition met");
});

it("shows only the selected round's committed output in the inspector", async () => {
  const user = userEvent.setup();
  mount(fixture());
  await user.click(screen.getByRole("button", { name: /Review loop:/ }));
  await user.click(screen.getByRole("button", { name: "Writer: Succeeded" }));
  await user.click(screen.getByRole("button", { name: "Open node details" }));
  const inspector = await screen.findByRole("complementary", {
    name: "Act details",
  });
  expect(within(inspector).getByText("first draft")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /Round 2 ·/ }));
  expect(within(inspector).queryByText("first draft")).not.toBeInTheDocument();
  expect(
    within(inspector).queryByText("partial stream must stay hidden"),
  ).not.toBeInTheDocument();
});
