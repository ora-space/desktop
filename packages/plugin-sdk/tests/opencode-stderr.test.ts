import { redactOpenCodeStderr } from "../src/opencode-stderr.ts";

/** Compares JSON-compatible values without a Node compatibility dependency. */
function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`Expected ${expectedJson}, received ${actualJson}`);
  }
}

Deno.test("redacts a raw plaintext API key from an error line", () => {
  assertEquals(
    redactOpenCodeStderr("auth failed: tvly-abc123 is invalid", [
      "tvly-abc123",
    ]),
    "auth failed: [REDACTED] is invalid",
  );
});

/**
 * The CLI may echo a value as it appears inside a JSON string — quotes and backslashes escaped —
 * not just the raw plaintext. The hand-derived JSON-string-content form of `a"b` is `a\"b` (the
 * quote escaped with a backslash), built here as a template literal where `\\` is one backslash.
 */
Deno.test("redacts the JSON-escaped form of a value containing a quote", () => {
  // JSON-string-content form: the quote is backslash-escaped.
  assertEquals(redactOpenCodeStderr(`echo a\\"b`, ['a"b']), "echo [REDACTED]");
  // The raw form is also redacted when it appears verbatim.
  assertEquals(redactOpenCodeStderr(`echo a"b`, ['a"b']), "echo [REDACTED]");
});

/**
 * The OpenCode-encoded form (braces escaped to `\\u007b` / `\\u007d`) is what the `ORA_MCP_...`
 * process env and the pre-parse JSONC text actually carry, so a CLI line echoing either would
 * contain it. The hand-derived encoded form of `{x}` is `\\u007bx\\u007d`, built with `\\` as one
 * backslash.
 */
Deno.test("redacts the OpenCode-encoded form of a value containing braces", () => {
  assertEquals(
    redactOpenCodeStderr(`config: \\u007bx\\u007d bad`, ["{x}"]),
    "config: [REDACTED] bad",
  );
  // The raw brace form is also redacted when it appears verbatim.
  assertEquals(
    redactOpenCodeStderr(`config: {x} bad`, ["{x}"]),
    "config: [REDACTED] bad",
  );
});

Deno.test("redacts every known secret in one pass", () => {
  assertEquals(
    redactOpenCodeStderr("alpha then beta then alpha", ["alpha", "beta"]),
    "[REDACTED] then [REDACTED] then [REDACTED]",
  );
});

/**
 * A shorter secret that is a prefix of a longer one must not be replaced first: redacting `abc`
 * inside `abcdef` would leave `def` behind, leaking a fragment of the longer secret. The longer
 * form must win.
 */
Deno.test("redacts the longer secret before a shorter substring so no fragment leaks", () => {
  assertEquals(
    redactOpenCodeStderr("abcdef", ["abc", "abcdef"]),
    "[REDACTED]",
  );
});

Deno.test("skips an empty secret so it cannot redact every position", () => {
  assertEquals(redactOpenCodeStderr("unchanged", [""]), "unchanged");
  // An empty secret alongside a real one does not disable redaction of the real one.
  assertEquals(
    redactOpenCodeStderr("real value real", ["", "real"]),
    "[REDACTED] value [REDACTED]",
  );
});

Deno.test("leaves stderr unchanged when no secrets are known", () => {
  assertEquals(redactOpenCodeStderr("nothing to hide", []), "nothing to hide");
});

/**
 * Secrets are matched literally, not as regular-expression patterns: a `.` in a secret must not
 * match an arbitrary character. Unescaped, `a.b` would also redact `axb`, which is wrong.
 */
Deno.test("matches a literal secret, not a regex pattern", () => {
  assertEquals(
    redactOpenCodeStderr("a.b matched, axb did not", ["a.b"]),
    "[REDACTED] matched, axb did not",
  );
});
