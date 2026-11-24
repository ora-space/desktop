import {
  AGENT_DISCOVERY_CLEANUP_RESERVE_MS,
  type AgentCommandsWait,
  waitForAgentCommands,
} from "../src/mod.ts";
import { AGENT_COMMANDS_WAIT_CAP_MS } from "../src/protocol/index.ts";

/** Runs one wait that nothing answers, without waiting, and returns the time it allowed. */
async function allowedWait(
  wait: (now: number) => AgentCommandsWait,
): Promise<number> {
  const { setTimeout: schedule } = globalThis;
  const { now } = Date;
  let allowed = -1;
  Date.now = () => 1_000_000;
  globalThis.setTimeout = ((fire: () => void, delay: number) => {
    allowed = delay;
    return schedule(fire, 0);
  }) as typeof setTimeout;
  try {
    const report = await waitForAgentCommands(
      new Promise(() => {}),
      wait(1_000_000),
    );
    assertEquals(report, undefined);
  } finally {
    globalThis.setTimeout = schedule;
    Date.now = now;
  }
  return allowed;
}

Deno.test("waits no longer than the cap, which a plugin can only lower", async () => {
  const cap = AGENT_COMMANDS_WAIT_CAP_MS;
  assertEquals(await allowedWait(() => ({})), cap);
  assertEquals(await allowedWait(() => ({ limitMs: cap * 10 })), cap);
  assertEquals(await allowedWait(() => ({ limitMs: cap / 2 })), cap / 2);
});

Deno.test("stops where the cleanup reserve begins", async () => {
  const reserve = AGENT_DISCOVERY_CLEANUP_RESERVE_MS;
  assertEquals(
    await allowedWait((now) => ({ deadline: now + reserve + 100 })),
    100,
  );
  assertEquals(
    await allowedWait((now) => ({
      deadline: now + 100,
      cleanupReserveMs: 40,
    })),
    60,
  );
  assertEquals(await allowedWait((now) => ({ deadline: now + reserve })), 0);
  assertEquals(await allowedWait((now) => ({ deadline: now })), 0);
});

Deno.test("takes a report that already arrived even with no time left", async () => {
  assertEquals(
    await waitForAgentCommands(Promise.resolve(["review"]), {
      deadline: Date.now() - 1,
    }),
    ["review"],
  );
});

Deno.test("keeps an empty report, and treats a failed one as unknown", async () => {
  assertEquals(await waitForAgentCommands(Promise.resolve([])), []);
  assertEquals(
    await waitForAgentCommands(Promise.reject(new Error("closed"))),
    undefined,
  );
});

/** Compares JSON values by their encoded form. */
function assertEquals(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}
