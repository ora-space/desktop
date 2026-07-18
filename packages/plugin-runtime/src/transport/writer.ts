import type { Writable } from "node:stream";

import { encodeJsonFrame } from "./frame.js";

export type WriterLane = "ordinary" | "control" | "safety";

export interface ProtocolWriterLimits {
  readonly maximumFrames: number;
  readonly maximumBytes: number;
  readonly reservedControlFrames: number;
  readonly reservedControlBytes: number;
  readonly reservedSafetyFrames: number;
  readonly reservedSafetyBytes: number;
}

/** Serializes complete JSON frames through a captured stdout writer with non-borrowable reserves. */
export class ProtocolWriter {
  readonly #write: Writable["write"];
  readonly #limits: ProtocolWriterLimits;
  #tail: Promise<void> = Promise.resolve();
  #queuedFrames = 0;
  #queuedBytes = 0;
  #ordinaryFrames = 0;
  #ordinaryBytes = 0;
  #safetyFrames = 0;
  #safetyBytes = 0;
  #closed = false;

  constructor(stdout: Writable, limits: ProtocolWriterLimits) {
    this.#write = stdout.write.bind(stdout);
    this.#limits = limits;
    validateWriterLimits(limits);
  }

  /** Reserves queue capacity synchronously and resolves only after every frame byte is written. */
  enqueue(payload: Uint8Array, lane: WriterLane): Promise<void> {
    if (this.#closed) {
      return Promise.reject(new Error("protocol writer is closed"));
    }
    const frame = encodeJsonFrame(payload);
    this.#reserve(frame.byteLength, lane);
    const write = this.#tail.then(() => this.#writeFrame(frame));
    this.#tail = write
      .catch(() => undefined)
      .then(() => this.#release(frame.byteLength, lane));
    return write;
  }

  response(payload: Uint8Array, lane: WriterLane = "ordinary"): Promise<void> {
    return this.enqueue(payload, lane);
  }

  notification(payload: Uint8Array, lane: WriterLane = "ordinary"): Promise<void> {
    return this.enqueue(payload, lane);
  }

  /** Prevents new commands and waits for the already accepted FIFO to drain. */
  async close(): Promise<void> {
    this.#closed = true;
    await this.#tail;
  }

  #reserve(bytes: number, lane: WriterLane): void {
    if (this.#queuedFrames + 1 > this.#limits.maximumFrames || this.#queuedBytes + bytes > this.#limits.maximumBytes) {
      throw new RangeError("protocol writer total queue budget exhausted");
    }
    const ordinaryFrameLimit =
      this.#limits.maximumFrames - this.#limits.reservedControlFrames - this.#limits.reservedSafetyFrames;
    const ordinaryByteLimit =
      this.#limits.maximumBytes - this.#limits.reservedControlBytes - this.#limits.reservedSafetyBytes;
    if (lane === "ordinary" && (this.#ordinaryFrames + 1 > ordinaryFrameLimit || this.#ordinaryBytes + bytes > ordinaryByteLimit)) {
      throw new RangeError("protocol writer ordinary queue budget exhausted");
    }
    if (lane === "safety" && (this.#safetyFrames + 1 > this.#limits.reservedSafetyFrames || this.#safetyBytes + bytes > this.#limits.reservedSafetyBytes)) {
      throw new RangeError("protocol writer safety reserve exhausted");
    }
    this.#queuedFrames += 1;
    this.#queuedBytes += bytes;
    if (lane === "ordinary") {
      this.#ordinaryFrames += 1;
      this.#ordinaryBytes += bytes;
    } else if (lane === "safety") {
      this.#safetyFrames += 1;
      this.#safetyBytes += bytes;
    }
  }

  #release(bytes: number, lane: WriterLane): void {
    this.#queuedFrames -= 1;
    this.#queuedBytes -= bytes;
    if (lane === "ordinary") {
      this.#ordinaryFrames -= 1;
      this.#ordinaryBytes -= bytes;
    } else if (lane === "safety") {
      this.#safetyFrames -= 1;
      this.#safetyBytes -= bytes;
    }
  }

  async #writeFrame(frame: Uint8Array): Promise<void> {
    await new Promise<void>((resolve, reject) => {
      this.#write(frame, (error?: Error | null) => {
        if (error === undefined || error === null) {
          resolve();
        } else {
          reject(error);
        }
      });
    });
  }
}

function validateWriterLimits(limits: ProtocolWriterLimits): void {
  const values = Object.values(limits);
  if (values.some((value) => !Number.isSafeInteger(value) || value < 1)) {
    throw new RangeError("protocol writer limits must be positive safe integers");
  }
  if (limits.reservedControlFrames + limits.reservedSafetyFrames >= limits.maximumFrames) {
    throw new RangeError("writer frame reserves leave no ordinary capacity");
  }
  if (limits.reservedControlBytes + limits.reservedSafetyBytes >= limits.maximumBytes) {
    throw new RangeError("writer byte reserves leave no ordinary capacity");
  }
}
