import assert from "node:assert/strict";
import test from "node:test";

import {
  FrameDecoder,
  FrameType,
  MAX_FRAME_BYTES,
  encodeJsonFrame,
} from "../../src/transport/frame.js";

const encoder = new TextEncoder();
const request = encoder.encode('{"jsonrpc":"2.0","id":"h:1","method":"ping","params":{}}');

test("encodes the canonical type-first five-byte header", () => {
  const encoded = encodeJsonFrame(request);
  assert.deepEqual([...encoded.subarray(0, 5)], [1, 0, 0, 0, 0x38]);
  assert.deepEqual(encoded.subarray(5), request);
});

test("decodes every split position and coalesced frames", () => {
  const encoded = encodeJsonFrame(request);
  for (let cut = 0; cut <= encoded.byteLength; cut += 1) {
    const decoder = new FrameDecoder();
    const frames = [
      ...decoder.decodeChunk(encoded.subarray(0, cut)),
      ...decoder.decodeChunk(encoded.subarray(cut)),
    ];
    decoder.finish();
    assert.deepEqual(frames, [{ type: FrameType.Json, payload: request }]);
  }

  const decoder = new FrameDecoder();
  assert.equal(decoder.decodeChunk(new Uint8Array([...encoded, ...encoded])).length, 2);
});

test("rejects invalid signed lengths, types, caps, and partial EOF", () => {
  assert.throws(() => new FrameDecoder(MAX_FRAME_BYTES + 1), RangeError);
  assert.throws(() => new FrameDecoder().decodeChunk(Uint8Array.of(1, 0, 0, 0, 0)), RangeError);
  assert.throws(
    () => new FrameDecoder().decodeChunk(Uint8Array.of(1, 0xff, 0xff, 0xff, 0xff)),
    RangeError,
  );
  assert.throws(() => new FrameDecoder().decodeChunk(Uint8Array.of(0x7f, 0, 0, 0, 2)), RangeError);

  const decoder = new FrameDecoder();
  decoder.decodeChunk(Uint8Array.of(0, 0));
  assert.throws(() => decoder.finish(), /partial frame header/u);
});
