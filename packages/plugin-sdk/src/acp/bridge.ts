import type { JsonValue } from "../protocol.ts";
import { decodeLines, encodeLine } from "./ndjson.ts";

/** The subset of a spawned child process an ACP bridge depends on, so tests can substitute one. */
export interface SpawnedProcess {
  stdin: WritableStream<Uint8Array>;
  stdout: ReadableStream<Uint8Array>;
  stderr: ReadableStream<Uint8Array>;
  readonly pid: number;
  kill(): void;
  readonly exited: Promise<void>;
}

export interface AcpProcessBridgeOptions {
  /**
   * Launches the external ACP program in `cwd`.
   *
   * The command, its arguments, and how candidates are tried are the plugin's business; the
   * bridge only needs a live process with all three stdio pipes.
   */
  spawn: (cwd: string) => SpawnedProcess | Promise<SpawnedProcess>;
  /** Receives every ACP frame emitted by the child, in output order. */
  onAcpFrame: (frame: JsonValue) => void;
  /** Invoked after the child exits on its own, never after an explicit stop. */
  onExited?: () => void;
  /** Prefix for the child's stderr lines when republished on this plugin's stderr. */
  logTag: string;
}

interface RunningProcess {
  process: SpawnedProcess;
  stdinWriter: WritableStreamDefaultWriter<Uint8Array>;
}

/**
 * Owns one ACP child process and bridges ACP frames between its stdio and Ora.
 *
 * The plugin owns the child's whole lifetime: spawn on `agent/start`, kill on `agent/stop`. Ora
 * never sees the child's stdio, which is what lets the external program use ACP methods the host
 * has never heard of. Nothing here parses ACP; frames are re-framed between Ora's binary envelope
 * and the child's NDJSON and otherwise passed through verbatim.
 */
export class AcpProcessBridge {
  readonly #spawn: AcpProcessBridgeOptions["spawn"];
  readonly #onAcpFrame: (frame: JsonValue) => void;
  readonly #onExited: () => void;
  readonly #logTag: string;
  #running: RunningProcess | undefined;
  #expectedExit = false;

  constructor(options: AcpProcessBridgeOptions) {
    this.#spawn = options.spawn;
    this.#onAcpFrame = options.onAcpFrame;
    this.#onExited = options.onExited ?? (() => {});
    this.#logTag = options.logTag;
  }

  get running(): boolean {
    return this.#running !== undefined;
  }

  /**
   * Spawns the child in the given working directory and starts bridging its stdio.
   *
   * Any previous child is stopped first so a restart cannot leave two children writing frames
   * into the same host connection.
   */
  async start(cwd: string): Promise<void> {
    await this.stop();
    this.#expectedExit = false;
    const process = await this.#spawn(cwd);
    this.#running = { process, stdinWriter: process.stdin.getWriter() };
    this.#attach(process);
  }

  /**
   * Forwards one host ACP frame into the child's stdin as NDJSON.
   *
   * Awaiting the write is what lets the child's backpressure reach the host instead of growing an
   * unbounded queue inside this process.
   */
  async writeAcp(frame: JsonValue): Promise<void> {
    const running = this.#running;
    if (running === undefined) {
      throw new Error(`the ${this.#logTag} agent is not running`);
    }
    await running.stdinWriter.write(encodeLine(JSON.stringify(frame)));
  }

  /**
   * Forwards one host frame when the child is up, otherwise drops it with a warning.
   *
   * Notifications have no response channel, so throwing here would never reach the host and
   * could not recover the frame either; a warning is the most useful outcome.
   */
  forwardAcp(frame: JsonValue): Promise<void> | void {
    if (!this.running) {
      console.warn(
        `dropping ACP frame: the ${this.#logTag} agent is not running`,
      );
      return;
    }
    return this.writeAcp(frame);
  }

  /** Kills the child and releases every pipe; idempotent when already stopped. */
  async stop(): Promise<void> {
    const running = this.#running;
    this.#running = undefined;
    this.#expectedExit = true;
    if (running === undefined) {
      return;
    }
    try {
      await running.stdinWriter.close();
    } catch {
      // The child already exited and closed its stdin; nothing is left to flush.
    }
    try {
      running.process.kill();
    } catch {
      // Already dead.
    }
  }

  /** Wires stdout, stderr, and exit bookkeeping for one live child. */
  #attach(process: SpawnedProcess): void {
    void this.#pumpStdout(process);
    void this.#pumpStderr(process);
    void process.exited.then(() => {
      if (this.#running?.process === process) {
        this.#running = undefined;
      }
      if (!this.#expectedExit) {
        console.warn(`${this.#logTag} ACP process exited unexpectedly`);
        this.#onExited();
      }
    });
  }

  /**
   * Forwards every NDJSON line the child prints as one ACP frame.
   *
   * A line that is not a JSON object is dropped with a warning rather than failing the bridge:
   * Ora rejects non-object frames anyway, and one stray diagnostic line must not end every live
   * session on this agent.
   */
  async #pumpStdout(process: SpawnedProcess): Promise<void> {
    try {
      for await (const line of decodeLines(process.stdout)) {
        let frame: JsonValue;
        try {
          frame = JSON.parse(line) as JsonValue;
        } catch {
          console.warn(`dropping non-JSON stdout line: ${line}`);
          continue;
        }
        if (
          frame === null || typeof frame !== "object" || Array.isArray(frame)
        ) {
          console.warn(`dropping non-object ACP frame from ${this.#logTag}`);
          continue;
        }
        this.#onAcpFrame(frame);
      }
    } catch (error) {
      console.warn(`${this.#logTag} stdout read failed: ${error}`);
    }
  }

  /** Republishes the child's diagnostics on this plugin's stderr, which Ora logs. */
  async #pumpStderr(process: SpawnedProcess): Promise<void> {
    try {
      for await (const line of decodeLines(process.stderr)) {
        if (line.length > 0) {
          console.error(`[${this.#logTag}] ${line}`);
        }
      }
    } catch (error) {
      console.warn(`${this.#logTag} stderr read failed: ${error}`);
    }
  }
}

/** Spawns a command with all three stdio pipes exposed for streaming. */
export function spawnPipedProcess(
  command: string,
  args: readonly string[],
  cwd: string,
): SpawnedProcess {
  const child = new Deno.Command(command, {
    args: [...args],
    cwd,
    stdin: "piped",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  return {
    stdin: child.stdin,
    stdout: child.stdout,
    stderr: child.stderr,
    pid: child.pid,
    kill: () => child.kill(),
    exited: child.status.then(() => undefined),
  };
}
