import assert from "node:assert/strict";
import { normalizeGeneratedText } from "./check-contract-schema.ts";

Deno.test("generated text comparison treats LF and CRLF as equivalent", () => {
  assert.equal(
    normalizeGeneratedText("first\r\nsecond\r\n"),
    normalizeGeneratedText("first\nsecond\n"),
  );
});

Deno.test("generated text comparison preserves non-newline differences", () => {
  assert.notEqual(
    normalizeGeneratedText("export type Value = string;\r\n"),
    normalizeGeneratedText("export type Value = number;\n"),
  );
});
