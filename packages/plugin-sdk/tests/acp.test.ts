import {
  AcpProcessBridge,
  decodeLines,
  encodeLine,
  isCommandNotFound,
  type SpawnedProcess,
  tryEachCandidate,
} from "../src/acp/mod.ts";
import { type JsonValue, PluginMethodError } from "../src/mod.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`Expected ${expectedJson}, received ${actualJson}`);
  }
}

/** An in-memory child process whose stdout/stderr the test feeds and whose stdin it reads. */
function fakeProcess(): {
  process: SpawnedProcess;
  stdout: WritableStreamDefaultWriter<Uint8Array>;
  stderr: WritableStreamDefaultWriter<Uint8Array>;
  stdinLines: () => Promise<string[]>;
  exit: () => void;
  killed: () => boolean;
} {
  const stdin = new TransformStream<Uint8Array>();
  const stdout = new TransformStream<Uint8Array>();
  const stderr = new TransformStream<Uint8Array>();
  let resolveExit!: () => void;
  const exited = new Promise<void>((resolve) => {
    resolveExit = resolve;
  });
  let killed = false;
  const collected: string[] = [];
  const stdinRead = (async () => {
    for await (const line of decodeLines(stdin.readable)) {
      collected.push(line);
    }
  })();
  return {
    process: {
      stdin: stdin.writable,
      stdout: stdout.readable,
      stderr: stderr.readable,
      pid: 42,
      kill: () => {
        killed = true;
        resolveExit();
      },
      exited,
    },
    stdout: stdout.writable.getWriter(),
    stderr: stderr.writable.getWriter(),
    stdinLines: async () => {
      await stdinRead;
      return collected;
    },
    exit: resolveExit,
    killed: () => killed,
  };
}

Deno.test("decodeLines splits CRLF and partial chunks into lines", async () => {
  const encoder = new TextEncoder();
  const readable = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode('{"a":1}\r\n{"b"'));
      controller.enqueue(encoder.encode(':2}\n\n{"c":3}'));
      controller.close();
    },
  });
  const lines: string[] = [];
  for await (const line of decodeLines(readable)) {
    lines.push(line);
  }
  assertEquals(lines, ['{"a":1}', '{"b":2}', '{"c":3}']);
  assertEquals(new TextDecoder().decode(encodeLine("x")), "x\n");
});

Deno.test("bridge forwards frames both ways and drops non-object lines", async () => {
  const fake = fakeProcess();
  const received: JsonValue[] = [];
  const bridge = new AcpProcessBridge({
    spawn: () => fake.process,
    onAcpFrame: (frame) => received.push(frame),
    logTag: "fake",
  });
  assertEquals(bridge.running, false);
  await bridge.start("/tmp");
  assertEquals(bridge.running, true);

  await bridge.writeAcp({ jsonrpc: "2.0", id: 1, method: "initialize" });
  const encoder = new TextEncoder();
  await fake.stdout.write(
    encoder.encode('not json\n[1,2]\n{"jsonrpc":"2.0","id":1,"result":{}}\n'),
  );
  await fake.stdout.close();
  // Let the stdout pump drain.
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(received, [{ jsonrpc: "2.0", id: 1, result: {} }]);

  await bridge.stop();
  assertEquals(bridge.running, false);
  assertEquals(fake.killed(), true);
  assertEquals(await fake.stdinLines(), [
    '{"jsonrpc":"2.0","id":1,"method":"initialize"}',
  ]);
});

Deno.test("bridge reports an unexpected exit but not an explicit stop", async () => {
  const unexpected = fakeProcess();
  let exitedCalls = 0;
  const bridge = new AcpProcessBridge({
    spawn: () => unexpected.process,
    onAcpFrame: () => {},
    onExited: () => exitedCalls++,
    logTag: "fake",
  });
  await bridge.start("/tmp");
  unexpected.exit();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(exitedCalls, 1);
  assertEquals(bridge.running, false);
  // forwardAcp while down drops instead of throwing.
  assertEquals(bridge.forwardAcp({ jsonrpc: "2.0", method: "x" }), undefined);

  const expected = fakeProcess();
  const quiet = new AcpProcessBridge({
    spawn: () => expected.process,
    onAcpFrame: () => {},
    onExited: () => exitedCalls++,
    logTag: "fake",
  });
  await quiet.start("/tmp");
  await quiet.stop();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(exitedCalls, 1);
});

Deno.test("tryEachCandidate distinguishes not-installed from real faults", async () => {
  const tried: string[] = [];
  const value = await tryEachCandidate(
    ["a", "b"],
    (command) => {
      tried.push(command);
      if (command === "a") {
        throw new Deno.errors.NotFound("a");
      }
      return `ran ${command}`;
    },
    (candidates) => `missing (${candidates.join(", ")})`,
  );
  assertEquals(value, "ran b");
  assertEquals(tried, ["a", "b"]);

  let notInstalled: unknown;
  try {
    await tryEachCandidate(
      ["a"],
      () => {
        throw new Error("command not found");
      },
      (candidates) => `missing (${candidates.join(", ")})`,
    );
  } catch (error) {
    notInstalled = error;
  }
  assertEquals(notInstalled instanceof PluginMethodError, true);
  assertEquals((notInstalled as PluginMethodError).code, -32001);
  assertEquals((notInstalled as Error).message, "missing (a)");

  let real: unknown;
  try {
    await tryEachCandidate(
      ["a", "b"],
      (command) => {
        if (command === "a") {
          throw new Deno.errors.NotFound("a");
        }
        throw new Error("exit code 2");
      },
      () => "unused",
    );
  } catch (error) {
    real = error;
  }
  assertEquals((real as Error).message, "exit code 2");
  assertEquals(isCommandNotFound(new Error("'x' is not recognized")), true);
  assertEquals(isCommandNotFound(new Error("permission denied")), false);
});
