import type * as acp from "@agentclientprotocol/sdk";
import assert from "node:assert/strict";
import test from "node:test";
import type {
  GetSessionOptionsResponse,
  LoadSessionEvent,
  PromptSessionEvent,
} from "@ora/contracts";
import { createChatStore, type ChatSessionClient } from "../src/index.ts";

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
function optionsEvent(configOptions: acp.SessionConfigOption[]) {
  return {
    type: "session_update",
    update: { sessionUpdate: "config_option_update", configOptions },
  } as const;
}
async function flushed() {
  for (let round = 0; round < 8; round += 1) await Promise.resolve();
}
async function settled(promise: Promise<unknown>): Promise<boolean> {
  let done = false;
  void promise.then(
    () => (done = true),
    () => (done = true),
  );
  await flushed();
  return done;
}
function harness(
  prompt: () => AsyncIterable<PromptSessionEvent>,
  setConfig: () => Promise<Record<string, never>> = async () => ({}),
) {
  const reads: ((configOptions: acp.SessionConfigOption[] | null) => void)[] =
    [];
  const failures: (() => void)[] = [];
  const client: ChatSessionClient = {
    getOptions: () =>
      new Promise<GetSessionOptionsResponse>((resolve, reject) => {
        reads.push((configOptions) => resolve({ configOptions }));
        failures.push(() => reject(new Error("read lost")));
      }),
    load: async function* (): AsyncIterable<LoadSessionEvent> {
      yield { type: "completed" } as const;
    },
    prompt,
    respondToPermission: async () => ({}),
    setConfig,
  };
  const store = createChatStore(client);
  store.getState().initializeSession("ora-1");
  return {
    store,
    reads,
    conversation: () => store.getState().conversations["ora-1"]!,
    read: async (configOptions: acp.SessionConfigOption[] | null) => {
      const waiting = reads.shift();
      failures.shift();
      assert.ok(waiting, "no read is waiting");
      waiting(configOptions);
      await flushed();
    },
    /** Loses the oldest read still waiting. */
    lose: async () => {
      reads.shift();
      const failing = failures.shift();
      assert.ok(failing, "no read is waiting");
      failing();
      await flushed();
    },
  };
}

test("two quick changes that come back to the first model leave no stale marker", async () => {
  const { store, read, conversation } = harness(async function* () {
    yield optionsEvent(onModel("fast"));
    yield { type: "completed", stopReason: "end_turn" } as const;
  });
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "hi" });
  const first = store.getState().setSessionConfig("ora-1", "model", "smart");
  const second = store.getState().setSessionConfig("ora-1", "model", "fast");
  await flushed();
  await read(onModel("smart"));
  await read(onModel("fast"));
  await first;
  await second;
  assert.deepEqual(conversation().configOptions, onModel("fast"));
  assert.deepEqual(
    conversation().modelChanges.map((change) => change.modelName),
    [],
    "the session is back on Fast",
  );
});

test("a model the agent changes mid-turn, with no change in flight, is not marked", async () => {
  const { store, conversation } = harness(async function* () {
    yield optionsEvent(onModel("fast"));
    yield optionsEvent(onModel("smart"));
    yield { type: "completed", stopReason: "end_turn" } as const;
  });
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "hi" });
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "again" });
  assert.deepEqual(conversation().configOptions, onModel("smart"));
  assert.deepEqual(conversation().modelChanges, []);
});

test("a change whose read after it is lost ends anyway, on the options the view had", async () => {
  const { store, lose, conversation } = harness(async function* () {
    yield optionsEvent(onModel("fast"));
    yield { type: "completed", stopReason: "end_turn" } as const;
  });
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "hi" });
  const choosing = store.getState().setSessionConfig("ora-1", "model", "smart");
  await flushed();
  assert.equal(await settled(choosing), false);

  await lose();

  assert.equal(await choosing, "made");
  assert.deepEqual(conversation().configOptions, onModel("fast"));
});

test("overlapping changes are marked against the options before the first of them", async () => {
  const answers: (() => void)[] = [];
  const { store, read, conversation } = harness(
    async function* () {
      yield optionsEvent(onModel("fast"));
      yield { type: "completed", stopReason: "end_turn" } as const;
    },
    () =>
      new Promise((resolve) => {
        answers.push(() => resolve({}));
      }),
  );
  await store.getState().sendMessage({ oraSessionId: "ora-1", text: "hi" });
  const first = store.getState().setSessionConfig("ora-1", "model", "smart");
  // The session moves to Smart before the second change is asked for.
  store.getState().optionsChanged("ora-1");
  await read(onModel("smart"));
  const second = store.getState().setSessionConfig("ora-1", "model", "fast");
  await flushed();
  for (const answer of answers) answer();
  await flushed();
  // Each change is told by a read that started after it was made.
  await read(onModel("fast"));
  await read(onModel("fast"));
  await first;
  await second;

  assert.deepEqual(conversation().configOptions, onModel("fast"));
  assert.deepEqual(
    conversation().modelChanges,
    [],
    "back on the model they started from",
  );
});
