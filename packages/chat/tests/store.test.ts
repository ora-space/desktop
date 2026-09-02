import type * as acp from "@agentclientprotocol/sdk";
import assert from "node:assert/strict";
import test from "node:test";
import {
  type LoadSessionEvent,
  type PromptSessionEvent,
  type PromptSessionRequest,
  LocalTransportError,
  RemoteContractError,
} from "@ora/contracts";
import { createChatStore, type ChatSessionClient } from "../src/index.js";

/** Builds one ACP text update without exposing protocol transport details to the tests. */
function textEvent(
  role: "user_message_chunk" | "agent_message_chunk",
  text: string,
  messageId: string,
): LoadSessionEvent {
  return {
    type: "session_update",
    update: {
      sessionUpdate: role,
      messageId,
      content: { type: "text", text },
    },
  };
}

/** Yields a deterministic finite stream in the same shape as the generated client. */
async function* events<Event>(items: Event[]): AsyncIterable<Event> {
  for (const item of items) yield item;
}

test("loads provider history and reconstructs turns from message boundaries", async () => {
  const client: ChatSessionClient = {
    load: () =>
      events([
        textEvent("user_message_chunk", "hel", "user-1"),
        textEvent("user_message_chunk", "lo", "user-1"),
        textEvent("user_message_chunk", "again", "user-2"),
        textEvent("agent_message_chunk", "hi", "agent-1"),
        { type: "completed" },
      ]),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");

  assert.deepEqual(store.getState().conversations["ora-1"], {
    configOptions: [],
    modelChanges: [],
    historyNotices: [],
    turns: [
      {
        id: "local-1",
        userMessage: {
          kind: "message",
          id: "local-2",
          role: "user",
          content: "hello",
          createdAt: 42,
          protocolMessageId: "user-1",
        },
        items: [],
        status: "completed",
        stopReason: null,
        error: null,
        createdAt: 42,
      },
      {
        id: "local-3",
        userMessage: {
          kind: "message",
          id: "local-4",
          role: "user",
          content: "again",
          createdAt: 42,
          protocolMessageId: "user-2",
        },
        items: [
          {
            kind: "message",
            id: "message-agent-1",
            role: "assistant",
            content: "hi",
            createdAt: 42,
            protocolMessageId: "agent-1",
          },
        ],
        status: "completed",
        stopReason: null,
        error: null,
        createdAt: 42,
      },
    ],
    availableCommands: [],
    sessionTitle: null,
    sessionUpdatedAt: null,
    isLoaded: true,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    error: null,
  });
});

test("publishes an active prompt incrementally while its session loads", async () => {
  let finishStream: () => void = () => {};
  const streamFinished = new Promise<void>((resolve) => {
    finishStream = resolve;
  });
  let finishLoad: () => void = () => {};
  const loadFinished = new Promise<void>((resolve) => {
    finishLoad = resolve;
  });
  const client: ChatSessionClient = {
    load: () => ({
      async *[Symbol.asyncIterator]() {
        yield textEvent("user_message_chunk", "automated prompt", "user-1");
        yield textEvent("agent_message_chunk", "partial ", "agent-1");
        yield textEvent("agent_message_chunk", "rep", "agent-1");
        yield textEvent("agent_message_chunk", "ly", "agent-1");
        await streamFinished;
        yield textEvent("agent_message_chunk", " final", "agent-1");
        yield { type: "turn_ended", stopReason: "end_turn" } as const;
        await loadFinished;
        yield { type: "completed" } as const;
      },
    }),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });
  let livePublicationCount = 0;
  const unsubscribe = store.subscribe((state) => {
    if (state.conversations["ora-1"]?.isResponding) {
      livePublicationCount += 1;
    }
  });

  const loading = store.getState().loadSession("ora-1");
  await new Promise<void>((resolve) => setTimeout(resolve, 25));

  const live = store.getState().conversations["ora-1"];
  assert.equal(live?.isLoaded, false);
  assert.equal(live?.isLoading, true);
  assert.equal(live?.isResponding, true);
  assert.equal(live?.turns[0]?.userMessage.content, "automated prompt");
  assert.equal(
    live?.turns[0]?.items[0]?.kind === "message" &&
      live.turns[0].items[0].content,
    "partial reply",
  );
  assert.equal(livePublicationCount, 1);

  finishStream();
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  const atTurnBoundary = store.getState().conversations["ora-1"];
  assert.equal(
    atTurnBoundary?.turns[0]?.items[0]?.kind === "message" &&
      atTurnBoundary.turns[0].items[0].content,
    "partial reply final",
  );
  assert.equal(livePublicationCount, 2);

  finishLoad();
  await loading;
  unsubscribe();

  const completed = store.getState().conversations["ora-1"];
  assert.equal(completed?.isLoading, false);
  assert.equal(completed?.isResponding, false);
  assert.equal(completed?.turns[0]?.status, "completed");
  assert.equal(completed?.turns[0]?.stopReason, "end_turn");
});

test("clears isResponding at each turn_ended during history replay", async () => {
  let finishAfterFirstTurn: () => void = () => {};
  const afterFirstTurn = new Promise<void>((resolve) => {
    finishAfterFirstTurn = resolve;
  });
  let finishAfterSecondOpen: () => void = () => {};
  const afterSecondOpen = new Promise<void>((resolve) => {
    finishAfterSecondOpen = resolve;
  });
  const client: ChatSessionClient = {
    load: () => ({
      async *[Symbol.asyncIterator]() {
        yield textEvent("user_message_chunk", "hello", "user-1");
        yield textEvent("agent_message_chunk", "hi", "agent-1");
        yield { type: "turn_ended", stopReason: "end_turn" } as const;
        await afterFirstTurn;
        yield textEvent("user_message_chunk", "again", "user-2");
        yield textEvent("agent_message_chunk", "there", "agent-2");
        await afterSecondOpen;
        yield { type: "turn_ended", stopReason: "end_turn" } as const;
        yield { type: "completed" } as const;
      },
    }),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });

  const loading = store.getState().loadSession("ora-1");
  await new Promise<void>((resolve) => setTimeout(resolve, 25));
  // Completed historical turns must not leave the working flag stuck.
  assert.equal(store.getState().conversations["ora-1"]?.isLoading, true);
  assert.equal(store.getState().conversations["ora-1"]?.isResponding, false);

  finishAfterFirstTurn();
  await new Promise<void>((resolve) => setTimeout(resolve, 25));
  assert.equal(store.getState().conversations["ora-1"]?.isResponding, true);

  finishAfterSecondOpen();
  await loading;

  assert.equal(store.getState().conversations["ora-1"]?.isResponding, false);
  assert.equal(store.getState().conversations["ora-1"]?.isLoaded, true);
});

test("flushes batched replay text together with a following tool boundary", async () => {
  let finishStream: () => void = () => {};
  const streamFinished = new Promise<void>((resolve) => {
    finishStream = resolve;
  });
  const client: ChatSessionClient = {
    load: () => ({
      async *[Symbol.asyncIterator]() {
        yield textEvent("user_message_chunk", "inspect", "user-1");
        yield textEvent("agent_message_chunk", "checking", "agent-1");
        yield {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "tool-1",
            title: "Read file",
            status: "in_progress",
          },
        } as const;
        await streamFinished;
        yield { type: "turn_ended", stopReason: "end_turn" } as const;
        yield { type: "completed" } as const;
      },
    }),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });

  const loading = store.getState().loadSession("ora-1");
  await new Promise<void>((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(
    store
      .getState()
      .conversations["ora-1"]?.turns[0]?.items.map((item) =>
        item.kind === "toolCall"
          ? [item.kind, item.title]
          : item.kind === "message"
            ? [item.kind, item.content]
            : [item.kind, null],
      ),
    [
      ["message", "checking"],
      ["toolCall", "Read file"],
    ],
  );

  finishStream();
  await loading;
});

test("retains durable-history notices after a successful replay", async () => {
  const client: ChatSessionClient = {
    load: () =>
      events<LoadSessionEvent>([
        {
          type: "history_notice",
          notice: { type: "unreadable_records", count: 2 },
        },
        {
          type: "history_notice",
          notice: {
            type: "unrecorded_content",
            reason: "no space left on device",
          },
        },
        { type: "completed" },
      ]),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client);

  await store.getState().loadSession("ora-1");

  assert.deepEqual(store.getState().conversations["ora-1"]?.historyNotices, [
    { type: "unreadable_records", count: 2 },
    { type: "unrecorded_content", reason: "no space left on device" },
  ]);
});

test("restores a cancelled turn and its unfinished tools from the recorded boundary", async () => {
  const client: ChatSessionClient = {
    load: () =>
      events<LoadSessionEvent>([
        textEvent("user_message_chunk", "run the suite", "user-1"),
        {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "t1",
            title: "Run tests",
            status: "in_progress",
          },
        },
        { type: "turn_ended", stopReason: "cancelled" },
        { type: "completed" },
      ]),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");

  // Provider replay could never carry this: it would restore the turn as
  // completed and leave the tool looking like it succeeded.
  const [turn] = store.getState().conversations["ora-1"]!.turns;
  assert.equal(turn?.status, "cancelled");
  assert.equal(turn?.stopReason, "cancelled");
  assert.deepEqual(turn?.items, [
    {
      kind: "toolCall",
      id: "t1",
      title: "Run tests",
      status: "cancelled",
      content: [],
      locations: [],
      createdAt: 42,
      updatedAt: 42,
    },
  ]);
});

test("keeps text that resumed after a tool call below it", async () => {
  // Replays the frame order an agent that omits messageId produces: reasoning,
  // the empty deltas it emits while switching to a tool, the call itself, more
  // reasoning, then the answer. Every text chunk here is unidentified, so the
  // run boundaries can only come from what arrived between them.
  const chunk = (
    kind: "agent_message_chunk" | "agent_thought_chunk",
    text: string,
  ) =>
    ({
      type: "session_update",
      update: { sessionUpdate: kind, content: { type: "text", text } },
    }) as PromptSessionEvent;
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([{ type: "completed" }]),
    prompt: () =>
      events<PromptSessionEvent>([
        chunk("agent_thought_chunk", " this"),
        chunk("agent_thought_chunk", ".\n"),
        chunk("agent_message_chunk", ""),
        chunk("agent_message_chunk", "\n\n\n"),
        {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "t1",
            title: "Terminal",
            kind: "execute",
            status: "pending",
          },
        },
        {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call_update",
            toolCallId: "t1",
            title: "ls -la",
            status: "completed",
          },
        },
        chunk("agent_thought_chunk", "The"),
        chunk("agent_thought_chunk", " command"),
        chunk("agent_message_chunk", "Here are the files."),
        { type: "completed", stopReason: "end_turn" },
      ]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");
  await store
    .getState()
    .sendMessage({ oraSessionId: "ora-1", text: "list files" });

  // The blank deltas open nothing, and the answer lands behind the call it
  // describes rather than merging back into an item positioned ahead of it.
  const [turn] = store.getState().conversations["ora-1"]!.turns;
  assert.deepEqual(
    turn?.items.map((item) => [
      item.kind,
      item.kind === "toolCall"
        ? item.title
        : (item as { content: string }).content,
    ]),
    [
      ["thought", " this.\n"],
      ["toolCall", "ls -la"],
      ["thought", "The command"],
      ["message", "Here are the files."],
    ],
  );
});

test("settles a live tool call the agent never reported finishing", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([{ type: "completed" }]),
    prompt: () =>
      events<PromptSessionEvent>([
        {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "t1",
            title: "Read file",
            status: "in_progress",
          },
        },
        // The agent moves straight on to unrelated output and ends the turn without
        // ever reporting the call finishing.
        textEvent(
          "agent_message_chunk",
          "here is what I found",
          "agent-1",
        ) as PromptSessionEvent,
        { type: "completed", stopReason: "end_turn" },
      ]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");
  await store
    .getState()
    .sendMessage({ oraSessionId: "ora-1", text: "read it" });

  const [turn] = store.getState().conversations["ora-1"]!.turns;
  assert.equal(turn?.status, "completed");
  assert.deepEqual(turn?.items, [
    {
      kind: "toolCall",
      id: "t1",
      title: "Read file",
      status: "completed",
      content: [],
      locations: [],
      createdAt: 42,
      updatedAt: 42,
    },
    {
      kind: "message",
      id: "message-agent-1",
      role: "assistant",
      content: "here is what I found",
      createdAt: 42,
      protocolMessageId: "agent-1",
    },
  ]);
});

test("shows an unsettled tool call as interrupted when the turn was cut short", async () => {
  for (const stopReason of [
    "max_tokens",
    "max_turn_requests",
    "refusal",
  ] as const) {
    const client: ChatSessionClient = {
      load: () => events<LoadSessionEvent>([{ type: "completed" }]),
      prompt: () =>
        events<PromptSessionEvent>([
          {
            type: "session_update",
            update: {
              sessionUpdate: "tool_call",
              toolCallId: "t1",
              title: "Read file",
              status: "in_progress",
            },
          },
          { type: "completed", stopReason },
        ]),
      respondToPermission: async () => ({}),
      setConfig: async () => ({ configOptions: [] }),
    };
    let nextId = 0;
    const store = createChatStore(client, {
      createId: () => `local-${++nextId}`,
      now: () => 42,
    });

    await store.getState().loadSession("ora-1");
    await store
      .getState()
      .sendMessage({ oraSessionId: "ora-1", text: "read it" });

    // The agent never said the call finished and never chose to stop, so nothing
    // observed an outcome worth reporting as success.
    const [turn] = store.getState().conversations["ora-1"]!.turns;
    assert.equal(
      turn?.items[0]?.kind === "toolCall" && turn.items[0].status,
      "cancelled",
      stopReason,
    );
  }
});

test("settles an unfinished tool call as interrupted when the stream fails", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([{ type: "completed" }]),
    prompt: async function* () {
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "tool_call",
          toolCallId: "t1",
          title: "Fetch data",
          status: "in_progress",
        },
      } as PromptSessionEvent;
      throw new Error("connection lost");
    },
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");
  await assert.rejects(
    store.getState().sendMessage({ oraSessionId: "ora-1", text: "fetch it" }),
  );

  // A broken stream leaves the tool's real outcome unknown, so the turn carries
  // the failure while the call reads as interrupted rather than as having failed.
  const [turn] = store.getState().conversations["ora-1"]!.turns;
  assert.equal(turn?.status, "failed");
  assert.deepEqual(turn?.items, [
    {
      kind: "toolCall",
      id: "t1",
      title: "Fetch data",
      status: "cancelled",
      content: [],
      locations: [],
      createdAt: 42,
      updatedAt: 42,
    },
  ]);
});

test("never reinterprets a tool failure the agent did report", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([{ type: "completed" }]),
    prompt: () =>
      events<PromptSessionEvent>([
        {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "t1",
            title: "Fetch data",
            status: "failed",
          },
        },
        { type: "completed", stopReason: "end_turn" },
      ]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");
  await store
    .getState()
    .sendMessage({ oraSessionId: "ora-1", text: "fetch it" });

  const [turn] = store.getState().conversations["ora-1"]!.turns;
  assert.equal(
    turn?.items[0]?.kind === "toolCall" && turn.items[0].status,
    "failed",
  );
});

test("restores an unfinished tool call as settled when the recorded turn completed", async () => {
  const client: ChatSessionClient = {
    load: () =>
      events<LoadSessionEvent>([
        textEvent("user_message_chunk", "read it", "user-1"),
        {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "t1",
            title: "Read file",
            status: "pending",
          },
        },
        { type: "turn_ended", stopReason: "end_turn" },
        { type: "completed" },
      ]),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");

  // Records written before the turn boundary settled tool status still replay as
  // a finished conversation rather than one that appears to still be working.
  const [turn] = store.getState().conversations["ora-1"]!.turns;
  assert.equal(turn?.status, "completed");
  assert.deepEqual(turn?.items, [
    {
      kind: "toolCall",
      id: "t1",
      title: "Read file",
      status: "completed",
      content: [],
      locations: [],
      createdAt: 42,
      updatedAt: 42,
    },
  ]);
});

test("keeps consecutive prompts apart when neither produced agent output", async () => {
  const client: ChatSessionClient = {
    load: () =>
      events<LoadSessionEvent>([
        textEvent("user_message_chunk", "first", "user-1"),
        { type: "turn_ended", stopReason: "end_turn" },
        textEvent("user_message_chunk", "second", "user-2"),
        { type: "turn_ended", stopReason: "end_turn" },
        { type: "completed" },
      ]),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: (() => {
      let nextId = 0;
      return () => `local-${++nextId}`;
    })(),
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");

  // Without the recorded boundary these two would merge into one user message,
  // because an empty turn looks exactly like one still receiving chunks.
  assert.deepEqual(
    store
      .getState()
      .conversations["ora-1"]?.turns.map((turn) => turn.userMessage.content),
    ["first", "second"],
  );
});

test("loads commands, session metadata, and structured content without creating metadata turns", async () => {
  const image = {
    type: "image" as const,
    data: "aGVsbG8=",
    mimeType: "image/png",
    uri: "file:///preview.png",
  };
  const client: ChatSessionClient = {
    load: () =>
      events<LoadSessionEvent>([
        {
          type: "session_update",
          update: { sessionUpdate: "usage_update", used: 1, size: 100 },
        },
        {
          type: "session_update",
          update: {
            sessionUpdate: "available_commands_update",
            availableCommands: [
              {
                name: "review",
                description: "Review current changes",
                input: { hint: "scope" },
              },
            ],
          },
        },
        {
          type: "session_update",
          update: {
            sessionUpdate: "session_info_update",
            title: "Review auth flow",
            updatedAt: "2026-07-24T12:00:00+09:00",
          },
        },
        {
          type: "session_update",
          update: {
            sessionUpdate: "user_message_chunk",
            messageId: "user-media",
            content: image,
          },
        },
        {
          type: "session_update",
          update: {
            sessionUpdate: "agent_message_chunk",
            messageId: "agent-media",
            content: image,
          },
        },
        { type: "completed" },
      ]),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");

  assert.deepEqual(store.getState().conversations["ora-1"], {
    configOptions: [],
    modelChanges: [],
    historyNotices: [],
    turns: [
      {
        id: "local-1",
        userMessage: {
          kind: "message",
          id: "local-2",
          role: "user",
          content: "",
          structuredContent: [image],
          createdAt: 42,
          protocolMessageId: "user-media",
        },
        items: [
          {
            kind: "content",
            id: "local-3",
            source: "message",
            content: image,
            createdAt: 42,
          },
        ],
        status: "completed",
        stopReason: null,
        error: null,
        createdAt: 42,
      },
    ],
    availableCommands: [
      {
        name: "review",
        description: "Review current changes",
        input: { hint: "scope" },
      },
    ],
    sessionTitle: "Review auth flow",
    sessionUpdatedAt: "2026-07-24T12:00:00+09:00",
    isLoaded: true,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    error: null,
  });
});

test("applies live command and partial session-info updates outside the response turn", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: () =>
      events<PromptSessionEvent>([
        {
          type: "session_update",
          update: {
            sessionUpdate: "available_commands_update",
            availableCommands: [{ name: "plan", description: "Create a plan" }],
          },
        },
        {
          type: "session_update",
          update: {
            sessionUpdate: "session_info_update",
            title: "Plan the migration",
          },
        },
        { type: "completed", stopReason: "end_turn" },
      ]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });

  await store
    .getState()
    .sendMessage({ oraSessionId: "ora-1", text: "plan it" });

  const conversation = store.getState().conversations["ora-1"];
  assert.deepEqual(conversation?.availableCommands, [
    { name: "plan", description: "Create a plan" },
  ]);
  assert.equal(conversation?.sessionTitle, "Plan the migration");
  assert.equal(conversation?.sessionUpdatedAt, null);
  assert.deepEqual(conversation?.turns[0]?.items, []);
});

test("sends structured image prompts", async () => {
  let promptRequest: PromptSessionRequest | undefined;
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([{ type: "completed" }]),
    prompt: (request) => {
      promptRequest = request;
      return events<PromptSessionEvent>([
        { type: "completed", stopReason: "end_turn" },
      ]);
    },
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");
  await store.getState().sendMessage({
    oraSessionId: "ora-1",
    text: "inspect",
    images: [{ data: "aGVsbG8=", mimeType: "image/png", uri: "diagram.png" }],
  });

  assert.deepEqual(promptRequest, {
    sessionId: "ora-1",
    prompt: [
      { type: "text", text: "inspect" },
      {
        type: "image",
        data: "aGVsbG8=",
        mimeType: "image/png",
        uri: "diagram.png",
      },
    ],
  });
  assert.deepEqual(
    store.getState().conversations["ora-1"]?.turns[0]?.userMessage
      .structuredContent,
    [
      {
        type: "image",
        data: "aGVsbG8=",
        mimeType: "image/png",
        uri: "diagram.png",
      },
    ],
  );
});

test("records the displayed prompt separately when the agent receives injected text", async () => {
  let promptRequest: PromptSessionRequest | undefined;
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([{ type: "completed" }]),
    prompt: (request) => {
      promptRequest = request;
      return events<PromptSessionEvent>([
        { type: "completed", stopReason: "end_turn" },
      ]);
    },
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");
  await store.getState().sendMessage({
    oraSessionId: "ora-1",
    text: "$test @role 原样输出",
    agentText: "$test @role 原样输出\n\n【角色：role】完整内容",
  });

  // The agent sees the injected expansion; the backend records the user's own
  // text so history keeps the skill/role tokens (and re-renders them as chips).
  assert.deepEqual(promptRequest, {
    sessionId: "ora-1",
    prompt: [
      { type: "text", text: "$test @role 原样输出\n\n【角色：role】完整内容" },
    ],
    recordPrompt: [{ type: "text", text: "$test @role 原样输出" }],
  });
});

test("aborting a prompt retains the partial response and marks the turn cancelled", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: (_request, options) => ({
      async *[Symbol.asyncIterator]() {
        yield textEvent(
          "agent_message_chunk",
          "partial",
          "agent-1",
        ) as PromptSessionEvent;
        yield {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "tool-1",
            title: "Run command",
            status: "in_progress",
          },
        } satisfies PromptSessionEvent;
        yield {
          type: "permission_request",
          permissionRequestId: "permission-1",
          toolCall: { toolCallId: "tool-1", title: "Run command" },
          options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
        } satisfies PromptSessionEvent;
        await new Promise<void>((_resolve, reject) => {
          options?.signal?.addEventListener(
            "abort",
            () => {
              const error = new Error("cancelled");
              error.name = "AbortError";
              reject(error);
            },
            { once: true },
          );
        });
      },
    }),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "id-1",
    now: () => 42,
  });
  const sending = store
    .getState()
    .sendMessage({ oraSessionId: "ora-1", text: " hello " });
  await new Promise<void>((resolve) => setTimeout(resolve, 0));

  store.getState().stopGeneration("ora-1");
  await sending;

  const conversation = store.getState().conversations["ora-1"];
  assert.deepEqual(conversation?.turns, [
    {
      id: "id-1",
      userMessage: {
        kind: "message",
        id: "id-1",
        role: "user",
        content: "hello",
        createdAt: 42,
      },
      items: [
        {
          kind: "message",
          id: "message-agent-1",
          role: "assistant",
          content: "partial",
          createdAt: 42,
          protocolMessageId: "agent-1",
        },
        {
          kind: "toolCall",
          id: "tool-1",
          title: "Run command",
          status: "cancelled",
          content: [],
          locations: [],
          createdAt: 42,
          updatedAt: 42,
        },
      ],
      status: "cancelled",
      stopReason: null,
      error: null,
      createdAt: 42,
    },
  ]);
  assert.equal(conversation?.isResponding, false);
  assert.deepEqual(conversation?.pendingPermissions, []);
});

test("a transport-level cancelled error is treated as a graceful cancel, not a failure", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: (_request, options) => ({
      async *[Symbol.asyncIterator]() {
        yield textEvent(
          "agent_message_chunk",
          "partial",
          "agent-1",
        ) as PromptSessionEvent;
        await new Promise<void>((_resolve, reject) => {
          options?.signal?.addEventListener(
            "abort",
            () => {
              // The Desktop stream surfaces a stop as a LocalTransportError of
              // kind "cancelled", not as a DOMException AbortError.
              reject(
                new LocalTransportError(
                  "cancelled",
                  "Desktop stream was cancelled",
                ),
              );
            },
            { once: true },
          );
        });
      },
    }),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "id-1",
    now: () => 42,
  });
  const sending = store
    .getState()
    .sendMessage({ oraSessionId: "ora-1", text: " hello " });
  await new Promise<void>((resolve) => setTimeout(resolve, 0));

  store.getState().stopGeneration("ora-1");
  await sending;

  const conversation = store.getState().conversations["ora-1"];
  assert.equal(conversation?.turns[0]?.status, "cancelled");
  assert.equal(conversation?.turns[0]?.error, null);
  assert.equal(conversation?.error, null);
});

test("settles active tools when the provider completes with a cancelled stop reason", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: () =>
      events<PromptSessionEvent>([
        {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "tool-1",
            title: "Run command",
            status: "in_progress",
          },
        },
        { type: "completed", stopReason: "cancelled" },
      ]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "id-1",
    now: () => 42,
  });

  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "run it" });

  assert.deepEqual(store.getState().conversations["ora-1"]?.turns[0], {
    id: "id-1",
    userMessage: {
      kind: "message",
      id: "id-1",
      role: "user",
      content: "run it",
      createdAt: 42,
    },
    items: [
      {
        kind: "toolCall",
        id: "tool-1",
        title: "Run command",
        status: "cancelled",
        content: [],
        locations: [],
        createdAt: 42,
        updatedAt: 42,
      },
    ],
    status: "cancelled",
    stopReason: "cancelled",
    error: null,
    createdAt: 42,
  });
});

test("shows the user turn before the session is persisted", async () => {
  let promptSessionId: string | undefined;
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: (request) => {
      promptSessionId = request.sessionId;
      return events<PromptSessionEvent>([
        textEvent(
          "agent_message_chunk",
          "done",
          "agent-1",
        ) as PromptSessionEvent,
        { type: "completed", stopReason: "end_turn" },
      ]);
    },
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  type Prepared = {
    availableCommands: [{ name: string; description: string }];
  };
  let finishPrepare: (result: Prepared) => void = () => {};
  const prepared = new Promise<Prepared>((resolve) => {
    finishPrepare = resolve;
  });

  const sending = store.getState().sendMessage({
    oraSessionId: "started-session",
    text: "hi",
    prepare: () => prepared,
  });

  // The started session id is final, so the turn is visible under it while the
  // session is still being persisted.
  const pending = store.getState().conversations["started-session"];
  assert.equal(pending?.turns.length, 1);
  assert.equal(pending?.turns[0]?.userMessage.content, "hi");
  assert.equal(pending?.isResponding, true);

  finishPrepare({
    availableCommands: [
      { name: "review", description: "Review current changes" },
    ],
  });
  await sending;

  assert.equal(promptSessionId, "started-session");
  const conversation = store.getState().conversations["started-session"];
  assert.equal(conversation?.isResponding, false);
  // The live turn is authoritative, so the conversation is already "loaded" and
  // the workspace never re-loads (and re-slides) it.
  assert.equal(conversation?.isLoaded, true);
  assert.deepEqual(conversation?.availableCommands, [
    { name: "review", description: "Review current changes" },
  ]);
  assert.deepEqual(conversation?.turns[0]?.items, [
    {
      kind: "message",
      id: "message-agent-1",
      role: "assistant",
      content: "done",
      createdAt: 42,
      protocolMessageId: "agent-1",
    },
  ]);
  assert.equal(conversation?.turns[0]?.status, "completed");
});

test("rolls back staged load updates when replay fails before completion", async () => {
  const client: ChatSessionClient = {
    load: () => ({
      async *[Symbol.asyncIterator]() {
        yield {
          type: "history_notice" as const,
          notice: { type: "unreadable_records" as const, count: 1 },
        };
        yield textEvent("agent_message_chunk", "uncommitted", "agent-new");
        throw new Error("load failed");
      },
    }),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });
  const previousTurn = {
    id: "old-turn",
    userMessage: {
      kind: "message" as const,
      id: "old-user",
      role: "user" as const,
      content: "prompt",
      createdAt: 1,
    },
    items: [
      {
        kind: "message" as const,
        id: "old",
        role: "assistant" as const,
        content: "history",
        createdAt: 1,
      },
    ],
    status: "completed" as const,
    stopReason: null,
    error: null,
    createdAt: 1,
  };
  store.setState({
    conversations: {
      "ora-1": {
        configOptions: [],
        modelChanges: [],
        historyNotices: [
          { type: "unrecorded_content", reason: "earlier write failure" },
        ],
        turns: [previousTurn],
        availableCommands: [],
        sessionTitle: null,
        sessionUpdatedAt: null,
        isLoaded: true,
        isLoading: false,
        isResponding: false,
        pendingPermissions: [],
        error: null,
      },
    },
  });

  await assert.rejects(store.getState().loadSession("ora-1"), /load failed/);

  assert.deepEqual(store.getState().conversations["ora-1"], {
    configOptions: [],
    modelChanges: [],
    historyNotices: [
      { type: "unrecorded_content", reason: "earlier write failure" },
    ],
    turns: [previousTurn],
    availableCommands: [],
    sessionTitle: null,
    sessionUpdatedAt: null,
    isLoaded: true,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    error: "load failed",
  });
});

/** Builds a model selector reporting `current` as the value in effect. */
function modelOptions(current: string): acp.SessionConfigOption[] {
  return [
    {
      id: "model",
      name: "Model",
      category: "model",
      type: "select",
      currentValue: current,
      options: [
        { value: "fast", name: "Fast" },
        { value: "smart", name: "Smart" },
      ],
    },
  ];
}

test("adopts the agent's answer to a model selection over the requested value", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    // The agent declined the switch and reported that it stayed on "fast".
    setConfig: async () => ({ configOptions: modelOptions("fast") }),
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });

  await store.getState().setSessionConfig("ora-1", "model", "smart");

  assert.deepEqual(store.getState().conversations["ora-1"], {
    configOptions: modelOptions("fast"),
    modelChanges: [],
    historyNotices: [],
    turns: [],
    availableCommands: [],
    sessionTitle: null,
    sessionUpdatedAt: null,
    isLoaded: false,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    error: null,
  });
});

test("reports an unreachable model selection instead of silently keeping the old value", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
    setConfig: async () => {
      throw new Error("session is gone");
    },
  };
  const store = createChatStore(client, {
    createId: () => "local",
    now: () => 42,
  });
  store.getState().setConfigOptions("ora-1", modelOptions("fast"));

  await assert.rejects(
    store.getState().setSessionConfig("ora-1", "model", "smart"),
    /session is gone/,
  );

  assert.deepEqual(store.getState().conversations["ora-1"], {
    configOptions: modelOptions("fast"),
    modelChanges: [],
    historyNotices: [],
    turns: [],
    availableCommands: [],
    sessionTitle: null,
    sessionUpdatedAt: null,
    isLoaded: false,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    error: "session is gone",
  });
});

test("marks the thread where the answering model changed", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: () =>
      events<PromptSessionEvent>([
        { type: "completed", stopReason: "end_turn" },
      ]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  // Establishing the first model on an empty thread is not a change.
  store.getState().setConfigOptions("ora-1", modelOptions("fast"));
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "hi" });
  store.getState().setConfigOptions("ora-1", modelOptions("smart"));

  assert.deepEqual(store.getState().conversations["ora-1"]?.modelChanges, [
    { id: "local-3", afterTurnCount: 1, modelName: "Smart", createdAt: 42 },
  ]);
});

test("marks an agent move before the message that carried it", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: () =>
      events<PromptSessionEvent>([
        { type: "completed", stopReason: "end_turn" },
      ]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  store.getState().setConfigOptions("ora-1", modelOptions("fast"));
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "first" });
  // The move is performed by the send that carries it, so the turn is already in
  // the thread when the incoming agent's options arrive.
  await store.getState().sendMessage({
    oraSessionId: "ora-1",
    text: "second",
    prepare: async () => {
      store.getState().adoptSwitchedAgent("ora-1", modelOptions("smart"));
      return { availableCommands: [] };
    },
  });

  assert.deepEqual(store.getState().conversations["ora-1"]?.modelChanges, [
    // Before turn 1 — the second message — rather than after the exchange it began.
    { id: "local-5", afterTurnCount: 1, modelName: "Smart", createdAt: 42 },
  ]);
});

test("leaves an agent move unmarked when nothing has been exchanged yet", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: () =>
      events<PromptSessionEvent>([
        { type: "completed", stopReason: "end_turn" },
      ]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  store.getState().setConfigOptions("ora-1", modelOptions("fast"));
  await store.getState().sendMessage({
    oraSessionId: "ora-1",
    text: "first",
    prepare: async () => {
      store.getState().adoptSwitchedAgent("ora-1", modelOptions("smart"));
      return { availableCommands: [] };
    },
  });

  assert.deepEqual(store.getState().conversations["ora-1"]?.modelChanges, []);
});

test("keeps one marker per point in the thread while the model is cycled", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: () =>
      events<PromptSessionEvent>([
        { type: "completed", stopReason: "end_turn" },
      ]),
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  store.getState().setConfigOptions("ora-1", modelOptions("fast"));
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "hi" });
  store.getState().setConfigOptions("ora-1", modelOptions("smart"));
  store.getState().setConfigOptions("ora-1", modelOptions("fast"));

  assert.deepEqual(store.getState().conversations["ora-1"]?.modelChanges, [
    { id: "local-4", afterTurnCount: 1, modelName: "Fast", createdAt: 42 },
  ]);
});

/** Builds the backend's refusal of a session that currently holds no live route. */
function sessionStoppedError(): RemoteContractError {
  return new RemoteContractError(
    {
      code: "session_stopped",
      params: {},
      requestId: "00000000-0000-4000-8000-000000000000",
    },
    null,
  );
}

test("re-attaches and retries once when the route died before the prompt was accepted", async () => {
  let loads = 0;
  let prompts = 0;
  const client: ChatSessionClient = {
    load: () => {
      loads += 1;
      return events<LoadSessionEvent>([{ type: "completed" }]);
    },
    // The provider process restarted between turns, so the first send is refused
    // before a single event exists; the second runs against the rebuilt route.
    prompt: () => {
      prompts += 1;
      if (prompts === 1) throw sessionStoppedError();
      return events<PromptSessionEvent>([
        textEvent(
          "agent_message_chunk",
          "back online",
          "agent-1",
        ) as PromptSessionEvent,
        { type: "completed", stopReason: "end_turn" },
      ]);
    },
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "hello" });

  // One load opened the conversation and one rebuilt the route the send needed.
  assert.equal(loads, 2);
  assert.equal(prompts, 2);
  const conversation = store.getState().conversations["ora-1"]!;
  // The reconnect never reaches the user: the optimistic turn streams straight
  // through and the conversation carries no error.
  assert.equal(conversation.error, null);
  assert.equal(conversation.turns.length, 1);
  const [turn] = conversation.turns;
  assert.equal(turn?.status, "completed");
  assert.deepEqual(
    turn?.items.map((item) => item.kind === "message" && item.content),
    ["back online"],
  );
});

test("does not retry a stopped session once the prompt has streamed anything", async () => {
  let loads = 0;
  let prompts = 0;
  const client: ChatSessionClient = {
    load: () => {
      loads += 1;
      return events<LoadSessionEvent>([{ type: "completed" }]);
    },
    prompt: async function* () {
      prompts += 1;
      yield textEvent(
        "agent_message_chunk",
        "partial",
        "agent-1",
      ) as PromptSessionEvent;
      throw sessionStoppedError();
    },
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");
  await assert.rejects(
    store.getState().sendMessage({ oraSessionId: "ora-1", text: "hello" }),
  );

  // Half a turn is already on screen, so resending would duplicate it.
  assert.equal(loads, 1);
  assert.equal(prompts, 1);
  assert.equal(
    store.getState().conversations["ora-1"]?.turns[0]?.status,
    "failed",
  );
});

test("does not re-attach for a failure that is not a stopped session", async () => {
  let loads = 0;
  let prompts = 0;
  const client: ChatSessionClient = {
    load: () => {
      loads += 1;
      return events<LoadSessionEvent>([{ type: "completed" }]);
    },
    prompt: () => {
      prompts += 1;
      throw new Error("agent runtime is unavailable");
    },
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");
  await assert.rejects(
    store.getState().sendMessage({ oraSessionId: "ora-1", text: "hello" }),
  );

  assert.equal(loads, 1);
  assert.equal(prompts, 1);
});
