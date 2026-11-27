import {
  AGENT_DISCOVERY_CLEANUP_RESERVE_MS,
  type AgentCommandsWait,
  skillSessionSetup,
  waitForAgentCommands,
} from "../src/mod.ts";
import {
  AGENT_COMMANDS_WAIT_CAP_MS,
  AGENT_DISCOVERY_BUDGET_MS,
} from "../src/protocol/index.ts";

/** Far enough away to bound nothing. */
const FAR = { deadline: Date.now() + 1_000_000_000 };

/** Runs one wait that nothing answers, without waiting, and returns the time it allowed. */
async function allowedWait(
  timeLeft: number,
  wait: (now: number) => AgentCommandsWait = () => ({}),
): Promise<number> {
  const { setTimeout: schedule } = globalThis;
  const { now } = Date;
  const started = 1_000_000;
  let allowed = -1;
  Date.now = () => started;
  globalThis.setTimeout = ((fire: () => void, delay: number) => {
    allowed = delay;
    return schedule(fire, 0);
  }) as typeof setTimeout;
  try {
    const report = await waitForAgentCommands(
      { deadline: started + timeLeft },
      new Promise(() => {}),
      wait(started),
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
  const plenty = AGENT_DISCOVERY_BUDGET_MS;
  assertEquals(await allowedWait(plenty), cap);
  assertEquals(await allowedWait(plenty, () => ({ limitMs: cap * 10 })), cap);
  assertEquals(
    await allowedWait(plenty, () => ({ limitMs: cap / 2 })),
    cap / 2,
  );
});

Deno.test("leaves the cleanup reserve without being asked to", async () => {
  const reserve = AGENT_DISCOVERY_CLEANUP_RESERVE_MS;
  assertEquals(await allowedWait(reserve + 100), 100);
  assertEquals(await allowedWait(reserve), 0);
  assertEquals(await allowedWait(0), 0);
  assertEquals(await allowedWait(100, () => ({ cleanupReserveMs: 40 })), 60);
});

Deno.test("takes the earlier of the host's deadline and the plugin's own", async () => {
  const reserve = AGENT_DISCOVERY_CLEANUP_RESERVE_MS;
  const own = (now: number) => ({ deadline: now + reserve + 100 });
  assertEquals(await allowedWait(reserve + 500, own), 100);
  assertEquals(await allowedWait(reserve + 50, own), 50);
});

Deno.test("takes a report that already arrived even with no time left", async () => {
  assertEquals(
    await waitForAgentCommands(
      { deadline: Date.now() - 1 },
      Promise.resolve(["review"]),
    ),
    ["review"],
  );
});

Deno.test("keeps an empty report, and treats a failed one as unknown", async () => {
  assertEquals(await waitForAgentCommands(FAR, Promise.resolve([])), []);
  assertEquals(
    await waitForAgentCommands(FAR, Promise.reject(new Error("closed"))),
    undefined,
  );
});

Deno.test("sets a probe session up with the Skills a session would be given", () => {
  const skills = {
    readRoots: ["/data/skills", "/home/plugins"],
    directories: ["/data/skills/review"],
    revision: "7",
  };
  const meta = { _meta: { "ora/skillDirectories": ["/data/skills/review"] } };
  assertEquals(skillSessionSetup(skills, true), {
    additionalDirectories: ["/data/skills", "/home/plugins"],
    ...meta,
  });
  assertEquals(skillSessionSetup(skills, false), meta);
  assertEquals(skillSessionSetup({ ...skills, directories: [] }, false), {});
  assertEquals(skillSessionSetup({ ...skills, readRoots: [] }, true), meta);
  assertEquals(skillSessionSetup(undefined, true), {});
});

/** Compares JSON values by their encoded form. */
function assertEquals(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}
