/** Keeps untrusted node names and errors inside their Markdown table cells. */
function cell(value) {
  return String(value ?? "UNKNOWN")
    .replaceAll("|", "\\|")
    .replace(/[\r\n]+/g, " ");
}

/** Requires evidence for every generated case; partial execution cannot become a successful suite. */
export function qualificationPassed({
  workflows,
  analyses,
  results,
  fatalError,
}) {
  return (
    !fatalError &&
    workflows.length > 0 &&
    workflows.every(
      (workflow) =>
        analyses.some(
          (analysis) => analysis.index === workflow.index && analysis.valid,
        ) &&
        results.some(
          (result) =>
            result.index === workflow.index &&
            result.qualified &&
            result.cleanupComplete,
        ),
    )
  );
}

/** Reports measured engine behavior without claiming UI, resource, or release certification. */
export function buildQualificationReport(input) {
  const { context, workflows, analyses, results, fatalError } = input;
  const passed = qualificationPassed(input);
  const scenarios = workflows.filter((workflow) => !workflow.isStress);
  const successes = scenarios.filter((workflow) =>
    results.some(
      (result) => result.index === workflow.index && result.qualified,
    ),
  );
  const validGraphs = workflows.filter((workflow) =>
    analyses.some(
      (analysis) => analysis.index === workflow.index && analysis.valid,
    ),
  );
  const stress = workflows.find((workflow) => workflow.isStress);
  const stressResult = results.find((result) => result.index === stress?.index);
  const duration =
    stressResult && Number.isFinite(stressResult.executionDurationMs)
      ? `${(stressResult.executionDurationMs / 60_000).toFixed(2)} minutes (${stressResult.executionDurationSource})`
      : "UNKNOWN / 未知";
  const rows = workflows.map((workflow) => {
    const analysis = analyses.find((entry) => entry.index === workflow.index);
    const result = results.find((entry) => entry.index === workflow.index);
    const verdict = result ? (result.qualified ? "PASS" : "FAIL") : "NOT RUN";
    const error = [
      analysis?.error,
      result?.error,
      ...(result?.cleanupErrors ?? []),
    ]
      .filter(Boolean)
      .join("; ");
    return `| ${workflow.index} | ${cell(workflow.name)} | ${analysis ? (analysis.valid ? "PASS" : "FAIL") : "NOT RUN"} | ${cell(result?.status ?? "NOT RUN")} | ${verdict} | ${cell(error || "—")} |`;
  });
  return `# Ora workflow qualification evidence

Result: **${passed ? "PASS" : "FAIL / INCOMPLETE"}** for backend IPC checks.

Workspace: ${cell(context.workspaceId)} (explicit disposable target)
Build label (operator-provided, unverified): ${cell(context.build?.label)}
Build metadata: ${cell(JSON.stringify(context.build ?? null))}
Generated graph digest: ${cell(context.graphDigest)}
Started (local time, ${cell(context.timeZone)}): ${cell(context.startedAt)}

| Check | Observed evidence | Result |
| --- | --- | --- |
| Production graph analysis | ${validGraphs.length}/${workflows.length} accepted by analyze_workflow | ${validGraphs.length === workflows.length && workflows.length > 0 ? "PASS" : "FAIL / INCOMPLETE"} |
| Scenario execution | ${successes.length}/${scenarios.length} successful with required node kinds | ${scenarios.length > 0 && successes.length === scenarios.length ? "PASS" : "FAIL / INCOMPLETE"} |
| Endurance execution | ${duration}; ${stressResult?.definitionNodeCount ?? "UNKNOWN"} definition nodes | ${stressResult?.qualified ? "PASS" : "FAIL / INCOMPLETE"} |
| Owned run/workflow cleanup | ${results.filter((result) => result.cleanupComplete).length}/${results.length} settled and removed | ${results.length > 0 && results.every((result) => result.cleanupComplete) ? "PASS" : "FAIL / INCOMPLETE"} |
| Workflow page rendering / white screen | NOT MEASURED / 未测量 | UNKNOWN |
| Memory leaks, deadlocks, orphan processes, database lock rates | NOT MEASURED / 未测量 | UNKNOWN |

${fatalError ? `Runner stopped: ${cell(fatalError)}\n` : ""}
| Case | Workflow | Analysis | Run status | Execution | Error |
| --- | --- | --- | --- | --- | --- |
${rows.join("\n")}

Run duration is measured for one execution. Agents' audit text is not instrumentation.
This report does not certify UI behavior or release readiness. Reports remain local.
`;
}
