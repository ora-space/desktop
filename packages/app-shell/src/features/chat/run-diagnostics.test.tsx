import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { RunDiagnosticsPanel } from "./run-diagnostics";
import {
  createChatStore,
  type ChatSessionClient,
  type ChatToolCall,
  type ChatTurn,
} from "@ora/chat";

beforeEach(async () => {
  await act(() => appI18n.changeLanguage("en-US"));
});

describe("run diagnostics", () => {
  it.each([
    null,
    {},
    { id: " ", parentID: "root", depth: 1 },
    { id: "child", parentID: " ", depth: 1 },
    { id: "child", parentID: "root", depth: 0 },
  ])(
    "shows cleared or invalid sources as parent activity and preserves retry: %j",
    async (source) => {
      let time = 100;
      let release = () => {};
      let markReady = () => {};
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      const ready = new Promise<void>((resolve) => {
        markReady = resolve;
      });
      const client: ChatSessionClient = {
        async *prompt() {
          yield {
            type: "session_update",
            update: {
              sessionUpdate: "tool_call",
              toolCallId: "tool",
              title: "Reattributed tool",
              status: "in_progress",
              _meta: {
                "opencode/child-session": {
                  id: "child",
                  parentID: "root",
                  depth: 1,
                },
              },
            },
          };
          time = 200;
          yield {
            type: "session_update",
            update: {
              sessionUpdate: "tool_call_update",
              toolCallId: "tool",
              _meta: { "opencode/child-session": source },
            },
          };
          time = 300;
          yield {
            type: "session_update",
            update: { sessionUpdate: "tool_call_update", toolCallId: "tool" },
          };
          yield {
            type: "session_update",
            update: {
              sessionUpdate: "session_info_update",
              _meta: {
                "opencode/child-session": source,
                "opencode/retry": { attempt: 2 },
              },
            },
          };
          markReady();
          await gate;
          yield { type: "completed", stopReason: "end_turn" };
        },
        async *load() {
          yield { type: "completed" };
        },
        getOptions: async () => ({ configOptions: null }),
        setConfig: async () => ({ configOptions: [] }),
        respondToPermission: async () => ({}),
      };
      const store = createChatStore(client, { now: () => time });
      let sending: Promise<void> | undefined;
      try {
        await act(async () => {
          sending = store
            .getState()
            .sendMessage({ oraSessionId: "one", text: "go" });
          await ready;
        });
        const conversation = store.getState().conversations.one!;
        const view = render(
          <RunDiagnosticsPanel
            diagnostics={conversation.diagnostics}
            turns={conversation.turns}
            isResponding={conversation.isResponding}
            pendingPermissions={[]}
          />,
        );
        expect(
          screen.getByText("Parent tools").nextElementSibling,
        ).toHaveTextContent("Reattributed tool");
        expect(screen.queryByText("Child tools")).not.toBeInTheDocument();
        expect(
          screen.queryByText("Tools with unconfirmed source"),
        ).not.toBeInTheDocument();
        expect(view.container.querySelectorAll("time")[0]).toHaveAttribute(
          "datetime",
          new Date(300).toISOString(),
        );
        expect(view.container.querySelectorAll("time")[1]).toHaveAttribute(
          "datetime",
          new Date(100).toISOString(),
        );
        expect(screen.getByText(/retry.*2/i)).toBeInTheDocument();
        view.unmount();
      } finally {
        await act(async () => {
          release();
          await sending;
          store.getState().dispose();
        });
      }
    },
  );

  it("does not invent provider health or turn replay into live activity", () => {
    render(
      <RunDiagnosticsPanel
        turns={[]}
        isResponding={false}
        pendingPermissions={[]}
      />,
    );
    expect(screen.getByText("Not reported (unknown)")).toBeInTheDocument();
    expect(
      screen.getByText("Unknown; replay is not new activity"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/healthy|stalled/i)).not.toBeInTheDocument();
  });

  it("says no parent activity was observed yet for a fresh attempt with a live snapshot", () => {
    render(
      <RunDiagnosticsPanel
        diagnostics={{}}
        turns={[]}
        isResponding
        pendingPermissions={[]}
      />,
    );
    expect(
      screen.getByText("No parent activity observed in this attempt yet"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Unknown; replay is not new activity"),
    ).not.toBeInTheDocument();
  });

  it("keeps the unknown wording while a load replays an open turn", () => {
    // `loadSession` publishes an unfinished replayed turn as responding without
    // recording live observations; that state must not read as "nothing happened".
    render(
      <RunDiagnosticsPanel turns={[]} isResponding pendingPermissions={[]} />,
    );
    expect(
      screen.getByText("Unknown; replay is not new activity"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/No parent activity observed/),
    ).not.toBeInTheDocument();
  });

  it("drops the 'yet' once a streamed attempt ended without observations", () => {
    render(
      <RunDiagnosticsPanel
        diagnostics={{}}
        turns={[]}
        isResponding={false}
        pendingPermissions={[]}
      />,
    );
    expect(
      screen.getByText("No parent activity was observed in this attempt"),
    ).toBeInTheDocument();
  });

  it("reports elapsed silence without creating activity and releases its timer", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(100_000);
      const view = render(
        <RunDiagnosticsPanel
          diagnostics={{ lastActivity: { kind: "tool", receivedAt: 90_000 } }}
          turns={[]}
          isResponding
          pendingPermissions={[]}
        />,
      );
      expect(
        screen.getByText(/No new parent activity for 10 seconds/),
      ).toBeInTheDocument();
      act(() => vi.advanceTimersByTime(5000));
      expect(
        screen.getByText(/No new parent activity for 15 seconds/),
      ).toBeInTheDocument();
      view.unmount();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("labels parent, child and unobserved tools separately", () => {
    const tool = (id: string, title: string): ChatToolCall => ({
      kind: "toolCall",
      id,
      title,
      status: "in_progress",
      content: [],
      locations: [],
      createdAt: 1,
      updatedAt: 1,
    });
    const turn: ChatTurn = {
      id: "turn",
      userMessage: {
        kind: "message",
        id: "user",
        role: "user",
        content: "go",
        createdAt: 1,
      },
      items: [
        tool("parent", "Parent delegate"),
        tool("child", "Background worker"),
        tool("old", "Historical tool"),
      ],
      status: "streaming",
      stopReason: null,
      error: null,
      createdAt: 1,
    };
    const view = render(
      <RunDiagnosticsPanel
        diagnostics={{
          lastActivity: { kind: "message", receivedAt: 100 },
          lastChildActivity: { kind: "tool", receivedAt: 200 },
          toolSources: { parent: "parent", child: "child" },
        }}
        turns={[turn]}
        isResponding
        pendingPermissions={[]}
      />,
    );
    expect(
      screen.getByText("Parent tools").nextElementSibling,
    ).toHaveTextContent("Parent delegate");
    expect(
      screen.getByText("Parent tools").nextElementSibling,
    ).not.toHaveTextContent("Background worker");
    expect(
      screen.getByText("Child tools").nextElementSibling,
    ).toHaveTextContent("Background worker");
    expect(
      screen.getByText("Tools with unconfirmed source").nextElementSibling,
    ).toHaveTextContent("Historical tool");
    expect(screen.getByText("Latest child activity")).toBeInTheDocument();
    expect(
      screen.getByText(/Automatically handled permissions/),
    ).toBeInTheDocument();
    view.unmount();
  });
});
