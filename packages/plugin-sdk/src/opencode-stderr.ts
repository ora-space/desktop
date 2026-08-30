import { encodeOpenCodeEnvValue } from "./opencode-env.ts";

/** The placeholder substituted for every redacted secret form. */
const REDACTED = "[REDACTED]";

/**
 * Redacts every known plaintext value (and each form it can take in OpenCode CLI stderr) from a
 * stderr line, returning a safe-to-log string.
 *
 * The OpenCode agent plugin pipes the OpenCode CLI's stderr verbatim to its own stderr, which the
 * Ora Plugin Runtime records as structured logs. A CLI error that echoes an Authorization header
 * would otherwise persist the key. Per the OpenCode adapter spec (§5.8), before forwarding, the
 * adapter must precisely replace the known plaintext value wherever it appears — and the value can
 * appear in three forms:
 *
 * - the raw plaintext (a header value echoed back),
 * - its JSON-string-content form (quotes and backslashes escaped — how it looks inside a JSON
 *   serialization a CLI might print),
 * - its OpenCode-encoded form (braces additionally escaped — what the `ORA_MCP_...` process env
 *   and the pre-parse JSONC text actually carry).
 *
 * All three are derivable from the plaintext alone, so a caller hands only the plaintext secrets.
 * Replacement is literal (never regex-pattern) and longest-form-first, so a secret that is a
 * prefix of another does not leave a fragment behind. An empty secret is skipped because it would
 * match every position.
 */
export function redactOpenCodeStderr(
  stderr: string,
  secrets: readonly string[],
): string {
  const forms = new Set<string>();
  for (const secret of secrets) {
    if (secret === "") {
      continue;
    }
    forms.add(secret);
    forms.add(JSON.stringify(secret).slice(1, -1));
    forms.add(encodeOpenCodeEnvValue(secret));
  }
  // Longest first so a longer secret is matched before a shorter one that is its prefix; otherwise
  // redacting the prefix would leave the suffix of the longer secret visible.
  const ordered = Array.from(forms).sort((a, b) => b.length - a.length);
  if (ordered.length === 0) {
    return stderr;
  }
  const alternation = ordered
    .map((form) => form.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("|");
  return stderr.replace(new RegExp(alternation, "g"), REDACTED);
}
