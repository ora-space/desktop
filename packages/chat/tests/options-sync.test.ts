import type * as acp from "@agentclientprotocol/sdk";
import assert from "node:assert/strict";
import test from "node:test";
import type {
  GetSessionOptionsResponse,
  LoadSessionEvent,
  PromptSessionEvent,
} from "@ora/contracts";
import { createChatStore, type ChatSessionClient } from "../src/index.ts";

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
function optionsEvent(
  configOptions: acp.SessionConfigOption[],
): LoadSessionEvent {
  return {
    type: "session_update",
    update: { sessionUpdate: "config_option_update", configOptions },
  };
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
  for (let round = 0; round < 5; round += 1) await Promise.resolve();
}

/**
 * A session whose reads the test answers one by one, and whose loads stream what the test scripts.
 *
 * `answer(null)` says the session has no provider.
 */
function session() {
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
    prompt: async function* (): AsyncIterable<PromptSessionEvent> {},
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
  const store = createChatStore(client);
  store.getState().initializeSession("ora-1");
  const conversation = () => store.getState().conversations["ora-1"];
  /** Answers the oldest read still waiting. */
  const answer = async (configOptions: acp.SessionConfigOption[] | null) => {
    const read = reads.shift();
    assert.ok(read, "no read is waiting");
    read(configOptions);
    await flushed();
  };
  return {
    store,
    reads,
    answer,
    conversation,
    streams: (script: () => AsyncIterable<LoadSessionEvent>) => {
      load = script;
    },
  };
}

test("reads the options of a session without a stream when they are announced", async () => {
  const { store, reads, answer, conversation } = session();

  store.getState().optionsChanged("ora-1");
  assert.equal(reads.length, 1);
  await answer(onModel("smart"));
  assert.deepEqual(conversation()?.configOptions, onModel("smart"));
  assert.equal(conversation()?.providerLost, false);

  // Without a provider the last options stay, for what they showed.
  store.getState().optionsChanged("ora-1");
  await answer(null);
  assert.equal(conversation()?.providerLost, true);
  assert.deepEqual(conversation()?.configOptions, onModel("smart"));

  // Options from a read say the session has a provider again.
  store.getState().optionsChanged("ora-1");
  await answer([]);
  assert.equal(conversation()?.providerLost, false);
  assert.deepEqual(conversation()?.configOptions, []);
});

test("leaves sessions that are not on screen alone", () => {
  const { store, reads } = session();

  store.getState().optionsChanged("elsewhere");

  assert.equal(reads.length, 0);
});

test("reads once more when the options are announced during a read", async () => {
  const { store, reads, answer, conversation } = session();

  store.getState().optionsChanged("ora-1");
  store.getState().optionsChanged("ora-1");
  assert.equal(reads.length, 1, "one read at a time");
  await answer(onModel("fast"));
  assert.equal(reads.length, 1, "read again after the first");
  await answer(onModel("smart"));

  assert.deepEqual(conversation()?.configOptions, onModel("smart"));
  assert.equal(reads.length, 0);
});

test("drops a read that a stream started or brought options during", async () => {
  for (const streamBrings of [onModel("fast"), undefined]) {
    const { store, answer, conversation, streams } = session();
    const ending = gate();
    streams(async function* () {
      if (streamBrings) yield optionsEvent(streamBrings);
      await ending.promise;
      yield { type: "completed" } as const;
    });

    store.getState().optionsChanged("ora-1");
    const loading = store.getState().loadSession("ora-1");
    await flushed();
    await answer(onModel("smart"));
    assert.deepEqual(conversation()?.configOptions, streamBrings ?? []);
    ending.resolve();
    await loading;
  }
});

test("reads once a stream ends, when the options were announced during it", async () => {
  const { store, reads, answer, conversation, streams } = session();
  const ending = gate();
  streams(async function* () {
    yield optionsEvent(onModel("fast"));
    await ending.promise;
    yield { type: "completed" } as const;
  });

  const loading = store.getState().loadSession("ora-1");
  await flushed();
  store.getState().optionsChanged("ora-1");
  assert.equal(reads.length, 0, "the stream's options stand while it runs");
  ending.resolve();
  await loading;

  assert.equal(reads.length, 1);
  await answer(onModel("smart"));
  assert.deepEqual(conversation()?.configOptions, onModel("smart"));
});

test("reads after a stream that brought no options, and not after one that brought an empty list", async () => {
  for (const [brought, expectedReads] of [
    [undefined, 1],
    [[], 0],
  ] as const) {
    const fixture = session();
    fixture.streams(async function* () {
      if (brought) yield optionsEvent([...brought]);
      yield { type: "completed" } as const;
    });

    await fixture.store.getState().loadSession("ora-1");

    assert.equal(fixture.reads.length, expectedReads, `brought ${brought}`);
    if (expectedReads > 0) {
      // A load of a session without a provider learns so here.
      await fixture.answer(null);
      assert.equal(fixture.conversation()?.providerLost, true);
    }
  }
});

test("reads every conversation on screen again after announcements may have been lost", () => {
  const { store, reads } = session();
  store.getState().initializeSession("ora-2");

  store.getState().resyncOptions();

  assert.equal(reads.length, 2);
});
