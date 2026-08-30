/**
 * Encodes one Setting value for safe OpenCode environment-variable substitution.
 *
 * OpenCode substitutes `{env:VAR}` references by splicing the raw value of `VAR` into the JSONC
 * text before parsing it. A value containing quotes, backslashes, or control characters would
 * break the surrounding JSON string, and a value containing braces would be misread by OpenCode's
 * pre-parse `{file:...}` scan as a file reference. Per the OpenCode adapter spec (§5.8), the adapter
 * therefore serializes the value as a JSON string, takes only the string content (the inner bytes,
 * still JSON-escaped), and replaces every `{` / `}` with the JSON unicode escapes `\\u007b` /
 * `\\u007d`. After OpenCode substitutes this encoded form and parses the JSONC, the original value
 * is restored exactly, because `\\u007b` / `\\u007d` are valid JSON escapes that parse back to the
 * original brace.
 *
 * This is the value the OpenCode agent plugin places on the `ORA_MCP_...` environment variable of
 * the OpenCode CLI subprocess — never a plaintext value written to a file.
 */
export function encodeOpenCodeEnvValue(value: string): string {
  // JSON.stringify wraps the value in quotes and escapes quotes, backslashes, and control
  // characters, yielding a string whose content is safe to splice into a JSON string position.
  // Strip the outer quotes to keep only that escaped content.
  const content = JSON.stringify(value).slice(1, -1);
  // Replace braces with JSON unicode escapes so OpenCode's pre-parse `{file:...}` scan never matches
  // a value that merely contains braces.
  return content.replaceAll("{", "\\u007b").replaceAll("}", "\\u007d");
}
