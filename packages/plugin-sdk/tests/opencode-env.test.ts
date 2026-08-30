import { encodeOpenCodeEnvValue } from "../src/opencode-env.ts";

/** Compares JSON-compatible values without a Node compatibility dependency. */
function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`Expected ${expectedJson}, received ${actualJson}`);
  }
}

/**
 * Values whose encoded form is a trivial, hand-derivable literal: the input carries no quotes,
 * backslashes, control characters, or braces, so JSON.stringify adds no escaping and the encoded
 * form equals the input verbatim. These anchor the happy path against a worked example, independent
 * of any JSON.parse oracle.
 */
const TRIVIAL_CASES: readonly (readonly [string, string])[] = [
  ["abc", "abc"],
  ["", ""],
  ["Bearer tvly-abc123", "Bearer tvly-abc123"],
];

Deno.test("encodeOpenCodeEnvValue leaves brace-free ASCII values verbatim", () => {
  for (const [value, expected] of TRIVIAL_CASES) {
    assertEquals(encodeOpenCodeEnvValue(value), expected);
  }
});

/**
 * A value containing `{` becomes the JSON unicode escape `\\u007b` (one backslash then `u007b`) and
 * `}` becomes `\\u007d`, per the OpenCode adapter spec §5.8, so OpenCode's pre-parse `{file:...}`
 * scan cannot misread a value that merely contains braces. In the assertions below, `\\u007b` is a
 * TypeScript source literal for the six-character runtime string backslash + `u007b`.
 */
Deno.test("encodeOpenCodeEnvValue escapes every brace to the JSON unicode form", () => {
  assertEquals(encodeOpenCodeEnvValue("{"), "\\u007b");
  assertEquals(encodeOpenCodeEnvValue("}"), "\\u007d");
  assertEquals(encodeOpenCodeEnvValue("{file:x}"), "\\u007bfile:x\\u007d");
  assertEquals(encodeOpenCodeEnvValue("a{b}c"), "a\\u007bb\\u007dc");
});

/**
 * The §5.8 invariant: after OpenCode substitutes the encoded value into a JSONC string position and
 * parses, the original value is restored exactly. `JSON.parse` is V8's independent JSON parser, so a
 * round-trip through it cannot pass by construction the way re-running the encoder could. The brace
 * escapes survive because `\\u007b` / `\\u007d` are valid JSON unicode escapes that parse back to
 * the original brace. The encoded form must also never carry a literal `{` or `}` — that is the
 * whole point of the brace escape, since a literal brace is what OpenCode's pre-parse scan matches.
 */
Deno.test("the encoded value round-trips through a JSON string parse to the original", () => {
  const values: readonly string[] = [
    'a"b',
    "a\\b",
    "a\nb",
    "a\tb",
    "{file:x}",
    "Bearer {env:X} {file:y}",
    "\\u007b",
    "plain",
    'quote "and" brace {x}',
    'path\\with\\backslashes {and} "quotes"',
  ];
  for (const value of values) {
    const encoded = encodeOpenCodeEnvValue(value);
    // The encoded form is the content of a JSON string; wrapping it in quotes must parse back.
    const restored = JSON.parse(`"${encoded}"`);
    assertEquals(restored, value);
    assertEquals(encoded.includes("{"), false);
    assertEquals(encoded.includes("}"), false);
  }
});
