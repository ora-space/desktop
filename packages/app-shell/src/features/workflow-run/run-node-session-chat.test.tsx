import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type * as acp from "@agentclientprotocol/sdk";
import type { ChatTurn, SessionConversation } from "@ora/chat";
import { createChatStore } from "@ora/chat";
import type { GraphWorkflowNodeStatus } from "@ora/workflow-runtime";
import { appI18n } from "../../i18n/i18n-instance";
import { useAgentModelStore } from "../../state/stores/agent-model-store";
import {
  createTestQueryClient,
  createHookWrapper,
} from "../../test/hook-harness";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { RunNodeSessionChat } from "./run-node-session-chat";

const sessionId = "session-1";
const runId = "run-1";
const nodeId = "node-a";

beforeEach(() => {
  useAgentModelStore.setState({ known: {} });
});

/** A loaded, quiet conversation so the dock does not stream anything during the test. */
function seededConversation(
  isResponding: boolean,
  turns: ChatTurn[] = [],
  configOptions: acp.SessionConfigOption[] = [],
): SessionConversation {
  return {
    configOptions,
    modelChanges: [],
    historyNotices: [],
    turns,
    availableCommands: [],
    sessionTitle: null,
    sessionUpdatedAt: null,
    isLoaded: true,
    isLoading: false,
    isResponding,
    pendingPermissions: [],
    error: null,
  };
}

function renderDock(
  status: GraphWorkflowNodeStatus,
  isResponding: boolean,
  turns: ChatTurn[] = [],
  sessionActions?: ReactNode,
  onNodeCompleted?: (nodeId: string) => void,
  configOptions: acp.SessionConfigOption[] = [],
) {
  const client = createMockClient(createMockClientState());
  const chatStore = createChatStore(client.session);
  chatStore.setState({
    conversations: {
      [sessionId]: seededConversation(isResponding, turns, configOptions),
    },
  });
  render(
    <RunNodeSessionChat
      sessionId={sessionId}
      status={status}
      interaction={{ runId, nodeId }}
      sessionActions={sessionActions}
      onNodeCompleted={onNodeCompleted}
    />,
    {
      wrapper: createHookWrapper(client, createTestQueryClient(), chatStore),
    },
  );
  return client;
}

/** Renders the same session surface without granting node interaction controls. */
function renderReadOnlyDock() {
  const client = createMockClient(createMockClientState());
  const loadSpy = vi.spyOn(client.session, "load");
  const chatStore = createChatStore(client.session);
  render(
    <RunNodeSessionChat
      sessionId={sessionId}
      status="running"
      sessionActions={<button type="button">返回阶段摘要</button>}
    />,
    {
      wrapper: createHookWrapper(client, createTestQueryClient(), chatStore),
    },
  );
  return loadSpy;
}

describe("RunNodeSessionChat", () => {
  it("renders the node session through the ordinary chat surface", () => {
    renderDock("awaiting_input", false);

    expect(screen.getByRole("main")).toHaveClass("flex-1");
    // TipTap exposes the empty-state hint via data-placeholder, not a native input placeholder.
    expect(
      screen
        .getByRole("textbox")
        .querySelector("[data-placeholder]")
        ?.getAttribute("data-placeholder"),
    ).toMatch(/描述一个任务/);
  });

  it("keeps the node model visible when another session changes models", async () => {
    const otherSessionOptions = createMockClientState().configOptions;
    const nodeSessionOptions: acp.SessionConfigOption[] = [
      {
        id: "model",
        name: "Model",
        category: "model",
        type: "select",
        currentValue: "workflow/node-model",
        options: [{ value: "workflow/node-model", name: "Workflow Model" }],
      },
    ];
    useAgentModelStore.setState({
      known: { "ora-space.opencode": otherSessionOptions },
    });
    renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      nodeSessionOptions,
    );

    const modelPicker = screen.getByRole("button", {
      name: appI18n.t("chat.modelSelector.label"),
    });
    expect(modelPicker).toBeDisabled();
    expect(modelPicker).toHaveTextContent("Workflow Model");

    act(() =>
      useAgentModelStore.setState({
        known: {
          "ora-space.opencode": [
            {
              id: "model",
              name: "Model",
              category: "model",
              type: "select",
              currentValue: "other/new-model",
              options: [
                { value: "other/new-model", name: "Other Session Model" },
              ],
            },
          ],
        },
      }),
    );
    expect(modelPicker).toHaveTextContent("Workflow Model");

    await userEvent.click(modelPicker);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("places the shared conversation navigator inside the node session", () => {
    renderDock("awaiting_input", false, [
      {
        id: "turn-1",
        userMessage: {
          kind: "message",
          id: "turn-1-user",
          role: "user",
          content: "Inspect the workflow",
          createdAt: 1,
        },
        items: [
          {
            kind: "message",
            id: "turn-1-agent",
            role: "assistant",
            content: "Inspection complete",
            createdAt: 2,
          },
        ],
        status: "completed",
        stopReason: "end_turn",
        error: null,
        createdAt: 1,
      },
    ]);

    const anchorList = screen.getByTestId("conversation-anchor-list");
    const navigator = anchorList.closest("nav");
    expect(navigator).not.toBeNull();
    expect(navigator).toHaveClass("absolute", "right-0");
    expect(navigator).not.toHaveClass("fixed");
    expect(
      anchorList.querySelectorAll("[data-conversation-tick]"),
    ).toHaveLength(2);
  });

  it("loads a non-interactive node through the same surface without interaction controls", async () => {
    const loadSpy = renderReadOnlyDock();

    await waitFor(() =>
      expect(loadSpy).toHaveBeenCalledWith({ sessionId }, expect.anything()),
    );
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByTestId("complete-current-node")).toBeNull();
    expect(
      screen.getByRole("button", { name: "返回阶段摘要" }),
    ).toBeInTheDocument();
  });

  it("keeps replaying an empty running session until its automatic prompt appears", async () => {
    const client = createMockClient(createMockClientState());
    const loadSpy = vi.spyOn(client.session, "load");
    const chatStore = createChatStore(client.session);
    chatStore.setState({
      conversations: { [sessionId]: seededConversation(false) },
    });

    render(<RunNodeSessionChat sessionId={sessionId} status="running" />, {
      wrapper: createHookWrapper(client, createTestQueryClient(), chatStore),
    });

    expect(
      screen.getByRole("status", { name: "正在加载历史记录…" }),
    ).toBeInTheDocument();

    await waitFor(
      () => {
        expect(loadSpy.mock.calls.length).toBeGreaterThanOrEqual(2);
        expect(loadSpy).toHaveBeenLastCalledWith(
          { sessionId },
          expect.anything(),
        );
      },
      { timeout: 1_500 },
    );
    expect(
      screen.getByRole("status", { name: "正在加载历史记录…" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("尚无消息")).toBeNull();
  });

  it("reveals a running node session as soon as its first turn is available", () => {
    const client = createMockClient(createMockClientState());
    const chatStore = createChatStore(client.session);
    chatStore.setState({
      conversations: { [sessionId]: seededConversation(false) },
    });
    render(<RunNodeSessionChat sessionId={sessionId} status="running" />, {
      wrapper: createHookWrapper(client, createTestQueryClient(), chatStore),
    });

    expect(
      screen.getByRole("status", { name: "正在加载历史记录…" }),
    ).toBeInTheDocument();

    act(() => {
      chatStore.setState({
        conversations: {
          [sessionId]: seededConversation(true, [
            {
              id: "turn-1",
              userMessage: {
                kind: "message",
                id: "turn-1-user",
                role: "user",
                content: "Automated workflow prompt",
                createdAt: 1,
              },
              items: [],
              status: "streaming",
              stopReason: null,
              error: null,
              createdAt: 1,
            },
          ]),
        },
      });
    });

    expect(
      screen.queryByRole("status", { name: "正在加载历史记录…" }),
    ).toBeNull();
    expect(screen.getByText("Automated workflow prompt")).toBeInTheDocument();
  });

  it("cancels the session's active prompt instead of aborting its load stream", async () => {
    const client = renderDock("running", true, [
      {
        id: "turn-1",
        userMessage: {
          kind: "message",
          id: "turn-1-user",
          role: "user",
          content: "Automated workflow prompt",
          createdAt: 1,
        },
        items: [
          {
            kind: "message",
            id: "turn-1-agent",
            role: "assistant",
            content: "Working",
            createdAt: 2,
          },
        ],
        status: "streaming",
        stopReason: null,
        error: null,
        createdAt: 1,
      },
    ]);
    const cancelSpy = vi.spyOn(client.session, "cancelPrompt");

    await userEvent.click(screen.getByRole("button", { name: /停止|stop/i }));

    await waitFor(() => expect(cancelSpy).toHaveBeenCalledWith({ sessionId }));
  });

  it("keeps the card fixed while the ordinary message list owns scrolling", () => {
    renderDock("awaiting_input", false, [
      {
        id: "turn-1",
        userMessage: {
          kind: "message",
          id: "turn-1-user",
          role: "user",
          content: "Inspect the workflow",
          createdAt: 1,
        },
        items: [],
        status: "completed",
        stopReason: null,
        error: null,
        createdAt: 1,
      },
    ]);

    expect(screen.getByTestId("message-list")).toHaveClass(
      "h-full",
      "overflow-y-auto",
    );
  });

  it("right-aligns completion and return actions without shifting the composer", () => {
    renderDock(
      "awaiting_input",
      false,
      [],
      <button type="button">返回阶段摘要</button>,
    );

    const actions = document.querySelector('[data-slot="composer-actions"]');
    const composerContainer = actions?.parentElement?.firstElementChild;
    expect(actions).not.toBeNull();
    expect(composerContainer).not.toBeNull();
    expect(actions).toHaveClass("absolute", "right-3");
    expect(actions?.parentElement).toHaveClass("relative");
    expect(composerContainer).toHaveClass(
      "mx-auto",
      "max-w-[760px]",
      "w-[calc(100%_-_13rem)]",
    );
    expect(
      within(actions as HTMLElement)
        .getAllByRole("button")
        .map(
          (button) => button.getAttribute("aria-label") ?? button.textContent,
        ),
    ).toEqual(["完成当前节点", "返回阶段摘要"]);
  });

  it("completes the node when the button is clicked while awaiting input", async () => {
    const client = renderDock("awaiting_input", false);
    const completeSpy = vi.spyOn(client.workflowRun, "completeNode");

    const button = screen.getByTestId("complete-current-node");
    expect(button).toBeEnabled();
    await userEvent.click(button);

    await waitFor(() =>
      expect(completeSpy).toHaveBeenCalledWith({ runId, nodeId }),
    );
  });

  it("shows completion progress until the request and run refresh settle", async () => {
    let releaseCompletion: () => void = () => {};
    const completionGate = new Promise<void>((resolve) => {
      releaseCompletion = resolve;
    });
    const onNodeCompleted = vi.fn();
    const client = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      onNodeCompleted,
    );
    vi.spyOn(client.workflowRun, "completeNode").mockImplementation(
      async () => {
        await completionGate;
        return {
          run: {
            id: runId,
            workspaceId: "workspace-1",
            workflowId: "workflow-1",
            snapshotId: "snapshot-1",
            name: "Run 1",
            status: "running",
            state: null,
            input: null,
            output: null,
            error: null,
            payload: null,
            startedAt: 1n,
            finishedAt: null,
            createdAt: 1n,
            updatedAt: 2n,
          },
        };
      },
    );

    const button = screen.getByTestId("complete-current-node");
    await userEvent.click(button);

    await waitFor(() => expect(button).toHaveAttribute("aria-busy", "true"));
    expect(button.querySelector('[data-slot="spinner"]')).not.toBeNull();
    expect(onNodeCompleted).not.toHaveBeenCalled();

    releaseCompletion();

    await waitFor(() => expect(button).toHaveAttribute("aria-busy", "false"));
    expect(onNodeCompleted).toHaveBeenCalledWith(nodeId);
  });

  it("disables the complete button while the agent is responding", () => {
    renderDock("awaiting_input", true);
    expect(screen.getByTestId("complete-current-node")).toBeDisabled();
  });

  it("hides the complete button once the node is terminal", () => {
    renderDock(
      "succeeded",
      false,
      [],
      <button type="button">返回阶段摘要</button>,
    );
    expect(screen.queryByTestId("complete-current-node")).toBeNull();
    expect(
      screen.getByRole("button", { name: "返回阶段摘要" }),
    ).toBeInTheDocument();
  });
});
