import type * as acp from "@agentclientprotocol/sdk";
import assert from "node:assert/strict";
import test from "node:test";
import {
  type GetSessionOptionsResponse,
  type LoadSessionEvent,
  type PromptSessionEvent,
  RemoteContractError,
} from "@ora/contracts";
import {
  createChatStore,
  type ChatSessionClient,
  type SessionConversation,
} from "../src/index.ts";

/** Names the model a session is on. */
function onModel(current: string): acp.SessionConfigOption[] {
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

/** Frames options a stream carries. */
function optionsEvent(configOptions: acp.SessionConfigOption[]) {
  return {
    type: "session_update",
    update: { sessionUpdate: "config_option_update", configOptions },
  } as const;
}

/** Builds a promise its test resolves at one point of a scripted stream. */
function gate() {
  let resolve: () => void = () => {};
  const promise = new Promise<void>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

/** Lets every settled promise run its callbacks. */
async function flushed() {
  for (let round = 0; round < 8; round += 1) await Promise.resolve();
}

/** What the session says when a change reaches it after it lost its provider. */
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

/**
 * A session the test drives: the agent answers changes with `answer`, reads wait for the test,
 * and loads stream what the test scripts. It starts on `fast`, after one turn.
 */
async function session(answer: () => Promise<object> = async () => ({})) {
  const reads: ((configOptions: acp.SessionConfigOption[] | null) => void)[] =
    [];
  let load: () => AsyncIterable<LoadSessionEvent> = async function* () {
    yield { type: "completed" } as const;
  };
  const client: ChatSessionClient = {
    getOptions: () =>
      new Promise<GetSessionOptionsResponse>((resolve) => {
        reads.push((configOptions) => resolve({ configOptions }));
      }),
    load: () => load(),
    prompt: async function* (): AsyncIterable<PromptSessionEvent> {
      yield optionsEvent(onModel("fast"));
      yield { type: "completed", stopReason: "end_turn" } as const;
    },
    respondToPermission: async () => ({}),
    setConfig: () => answer() as Promise<Record<string, never>>,
  };
  const store = createChatStore(client);
  store.getState().initializeSession("ora-1");
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "hi" });
  const conversation = () => store.getState().conversations["ora-1"]!;
  return {
    store,
    reads,
    conversation,
    /** Answers the oldest read still waiting. */
    read: async (configOptions: acp.SessionConfigOption[] | null) => {
      const waiting = reads.shift();
      assert.ok(waiting, "no read is waiting");
      waiting(configOptions);
      await flushed();
    },
    streams: (script: () => AsyncIterable<LoadSessionEvent>) => {
      load = script;
    },
  };
}

/** Tells whether a promise has settled, without waiting for it. */
async function settled(promise: Promise<unknown>): Promise<boolean> {
  let done = false;
  void promise.then(
    () => (done = true),
    () => (done = true),
  );
  await flushed();
  return done;
}

test("a change on an idle session is in progress until a read after it answers", async () => {
  // The agent may also keep the model it was on.
  for (const kept of ["smart", "fast"]) {
    const { store, reads, read, conversation } = await session();

    const choosing = store
      .getState()
      .setSessionConfig("ora-1", "model", "smart");
    await flushed();
    assert.equal(reads.length, 1, "read once the agent made the change");
    assert.equal(await settled(choosing), false);
    // The answer says nothing about the options; the read does.
    assert.deepEqual(conversation().configOptions, onModel("fast"));
    await read(onModel(kept));

    assert.equal(await choosing, "made");
    assert.deepEqual(conversation().configOptions, onModel(kept));
    // Marked once, and only when the model moved.
    assert.equal(conversation().modelChanges.length, kept === "smart" ? 1 : 0);
  }
});

test("a change during a stream ends on the options the stream brings, also unchanged ones", async () => {
  for (const brought of ["smart", "fast"]) {
    const { store, reads, conversation, streams } = await session();
    const answered = gate();
    const ending = gate();
    streams(async function* () {
      await answered.promise;
      yield optionsEvent(onModel(brought));
      await ending.promise;
      yield { type: "completed" } as const;
    });
    const loading = store.getState().loadSession("ora-1");
    await flushed();

    const choosing = store
      .getState()
      .setSessionConfig("ora-1", "model", "smart");
    await flushed();
    assert.equal(
      await settled(choosing),
      false,
      `${brought}: before the stream says`,
    );
    answered.resolve();

    assert.equal(await choosing, "made", brought);
    assert.equal(
      reads.length,
      0,
      `${brought}: the stream told, nothing was read`,
    );
    assert.deepEqual(conversation().configOptions, onModel(brought));
    ending.resolve();
    await loading;
  }
});

test("a change made just before its stream ends is told by the read after the stream", async () => {
  const { store, reads, read, conversation, streams } = await session();
  const ending = gate();
  streams(async function* () {
    yield optionsEvent(onModel("fast"));
    await ending.promise;
    yield { type: "completed" } as const;
  });
  const loading = store.getState().loadSession("ora-1");
  await flushed();

  const choosing = store.getState().setSessionConfig("ora-1", "model", "smart");
  await flushed();
  // The announcement of the change is lost; the stream just ends.
  ending.resolve();
  await loading;
  assert.equal(reads.length, 1, "read once the stream ended");
  await read(onModel("smart"));

  assert.equal(await choosing, "made");
  assert.deepEqual(conversation().configOptions, onModel("smart"));
});

test("a change settled by a stop ends once the session is known to have no provider", async () => {
  const { store, read, conversation } = await session();

  const choosing = store.getState().setSessionConfig("ora-1", "model", "smart");
  await flushed();
  await read(null);

  assert.equal(await choosing, "made");
  assert.equal(conversation().providerLost, true);
  assert.equal(conversation().modelChanges.length, 0, "the model did not move");
});

test("a change that finds the session without a provider is an intent, not an error", async () => {
  const { store, conversation } = await session(async () => {
    throw sessionStoppedError();
  });

  assert.equal(
    await store.getState().setSessionConfig("ora-1", "model", "smart"),
    "noProvider",
  );
  assert.equal(conversation().providerLost, true);
  assert.equal(conversation().error, null);
});

test("a model the agent changes on its own, with no change in flight, is not marked", async () => {
  const { store, conversation, streams } = await session();
  streams(async function* () {
    yield optionsEvent(onModel("smart"));
    yield { type: "completed" } as const;
  });

  await store.getState().loadSession("ora-1");

  assert.deepEqual(conversation().configOptions, onModel("smart"));
  assert.equal(conversation().modelChanges.length, 0);
});

test("options a start or a switch answered with give way to newer ones a stream brought", async () => {
  const { store, conversation, streams } = await session();
  store.setState((state) => ({
    conversations: {
      ...state.conversations,
      "ora-1": {
        ...state.conversations["ora-1"]!,
        diagnostics: {} as NonNullable<SessionConversation["diagnostics"]>,
      },
    },
  }));
  const fetchedFrom = store.getState().optionsFetchedFrom("ora-1");
  streams(async function* () {
    yield optionsEvent(onModel("smart"));
    yield { type: "completed" } as const;
  });
  await store.getState().loadSession("ora-1");

  store.getState().adoptSwitchedAgent("ora-1", onModel("fast"), fetchedFrom);
  store.getState().setConfigOptions("ora-1", onModel("fast"), fetchedFrom);

  assert.deepEqual(conversation().configOptions, onModel("smart"));
  // The move still happened: usage starts over whatever options are newer.
  assert.deepEqual(conversation().usage, {
    context: { status: "awaiting_report" },
    lastTurnTokens: { status: "awaiting_completion" },
  });
  assert.equal(
    conversation().diagnostics,
    undefined,
    "diagnostics start over too",
  );
  // With nothing newer meanwhile, the answer is taken.
  store
    .getState()
    .setConfigOptions(
      "ora-1",
      onModel("fast"),
      store.getState().optionsFetchedFrom("ora-1"),
    );
  assert.deepEqual(conversation().configOptions, onModel("fast"));
});

test("options a stream brought while the change was on its way end it as soon as it is made", async () => {
  const answered = gate();
  const { store, reads, conversation, streams } = await session(async () => {
    await answered.promise;
    return {};
  });
  const ending = gate();
  const brought = gate();
  streams(async function* () {
    await brought.promise;
    yield optionsEvent(onModel("smart"));
    await ending.promise;
    yield { type: "completed" } as const;
  });
  const loading = store.getState().loadSession("ora-1");
  await flushed();

  const choosing = store.getState().setSessionConfig("ora-1", "model", "smart");
  brought.resolve();
  await flushed();
  answered.resolve();

  assert.equal(await settled(choosing), true, "nothing more to wait for");
  assert.equal(reads.length, 0);
  assert.deepEqual(conversation().configOptions, onModel("smart"));
  ending.resolve();
  await loading;
});

test("a read that started before the change was made does not end it", async () => {
  const { store, reads, read, conversation } = await session();
  // An announcement from before the change starts a read that is still out when it is made.
  store.getState().optionsChanged("ora-1");
  assert.equal(reads.length, 1);

  const choosing = store.getState().setSessionConfig("ora-1", "model", "smart");
  await flushed();
  await read(onModel("fast"));
  assert.equal(await settled(choosing), false, "told by a read from before it");
  assert.equal(reads.length, 1, "read again for the change");
  await read(onModel("smart"));

  assert.equal(await choosing, "made");
  assert.deepEqual(conversation().configOptions, onModel("smart"));
});
