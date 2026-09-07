/** Pretty-prints structured control-node output while preserving ordinary text verbatim. */
export function formatWorkflowNodeOutput(output: string): string {
  try {
    const value: unknown = JSON.parse(output);
    return typeof value === "string" ? value : JSON.stringify(value, null, 2);
  } catch {
    return output;
  }
}
