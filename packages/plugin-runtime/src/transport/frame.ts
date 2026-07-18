export const FRAME_HEADER_BYTES = 5;
export const MAX_FRAME_BYTES = 8 * 1024 * 1024;

/** Identifies the payload encoding. Request/Response/Notification live in JSON-RPC. */
export const FrameType = {
  Json: 1,
} as const;

export type FrameType = (typeof FrameType)[keyof typeof FrameType];

export interface Frame {
  readonly type: FrameType;
  readonly payload: Uint8Array;
}

/** Encodes `[type i8][length i32 BE][payload]`. */
export function encodeFrame(
  type: FrameType,
  payload: Uint8Array,
  maximumPayloadBytes = MAX_FRAME_BYTES,
): Uint8Array {
  validateMaximum(maximumPayloadBytes);
  validatePayloadLength(payload.byteLength, maximumPayloadBytes);
  validateFrameType(type);

  const encoded = new Uint8Array(FRAME_HEADER_BYTES + payload.byteLength);
  const view = new DataView(encoded.buffer, encoded.byteOffset, encoded.byteLength);
  view.setInt8(0, type);
  view.setInt32(1, payload.byteLength, false);
  encoded.set(payload, FRAME_HEADER_BYTES);
  return encoded;
}

/** Encodes one JSON payload frame using the only currently supported payload kind. */
export function encodeJsonFrame(
  payload: Uint8Array,
  maximumPayloadBytes = MAX_FRAME_BYTES,
): Uint8Array {
  return encodeFrame(FrameType.Json, payload, maximumPayloadBytes);
}

/** Incrementally decodes arbitrary pipe chunks and never allocates before header validation. */
export class FrameDecoder {
  readonly #maximumPayloadBytes: number;
  #header = new Uint8Array(FRAME_HEADER_BYTES);
  #headerFilled = 0;
  #frameType: FrameType | undefined;
  #payload: Uint8Array | undefined;
  #payloadFilled = 0;

  constructor(maximumPayloadBytes = MAX_FRAME_BYTES) {
    validateMaximum(maximumPayloadBytes);
    this.#maximumPayloadBytes = maximumPayloadBytes;
  }

  decodeChunk(chunk: Uint8Array): Frame[] {
    const frames: Frame[] = [];
    let offset = 0;

    while (offset < chunk.byteLength) {
      if (this.#payload === undefined) {
        const copied = Math.min(FRAME_HEADER_BYTES - this.#headerFilled, chunk.byteLength - offset);
        this.#header.set(chunk.subarray(offset, offset + copied), this.#headerFilled);
        this.#headerFilled += copied;
        offset += copied;
        if (this.#headerFilled === FRAME_HEADER_BYTES) {
          const view = new DataView(
            this.#header.buffer,
            this.#header.byteOffset,
            this.#header.byteLength,
          );
          const type = view.getInt8(0);
          validateFrameType(type);
          const length = view.getInt32(1, false);
          validatePayloadLength(length, this.#maximumPayloadBytes);
          this.#frameType = type;
          this.#payload = new Uint8Array(length);
          this.#payloadFilled = 0;
        }
      } else {
        const copied = Math.min(
          this.#payload.byteLength - this.#payloadFilled,
          chunk.byteLength - offset,
        );
        this.#payload.set(chunk.subarray(offset, offset + copied), this.#payloadFilled);
        this.#payloadFilled += copied;
        offset += copied;
        if (this.#payloadFilled === this.#payload.byteLength) {
          frames.push({ type: this.#frameType!, payload: this.#payload });
          this.#header = new Uint8Array(FRAME_HEADER_BYTES);
          this.#headerFilled = 0;
          this.#frameType = undefined;
          this.#payload = undefined;
          this.#payloadFilled = 0;
        }
      }
    }

    return frames;
  }

  /** Rejects EOF unless it occurs exactly between complete frames. */
  finish(): void {
    if (this.#payload !== undefined) {
      throw new Error("partial frame payload at EOF");
    }
    if (this.#headerFilled !== 0) {
      throw new Error("partial frame header at EOF");
    }
  }
}

function validateMaximum(maximum: number): void {
  if (!Number.isInteger(maximum) || maximum < 1 || maximum > MAX_FRAME_BYTES) {
    throw new RangeError(`frame maximum must be in 1..=${MAX_FRAME_BYTES}`);
  }
}

function validatePayloadLength(length: number, maximum: number): void {
  if (!Number.isInteger(length) || length <= 0) {
    throw new RangeError(`frame payload length must be positive, got ${length}`);
  }
  if (length > maximum) {
    throw new RangeError(`frame payload length ${length} exceeds ${maximum}`);
  }
}

function validateFrameType(type: number): asserts type is FrameType {
  if (type !== FrameType.Json) {
    throw new RangeError(`unknown signed frame type ${type}`);
  }
}
