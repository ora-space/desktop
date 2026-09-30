import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
import { generateAllWorkflows } from "./generator.mjs";
import { connectCDP } from "./cdp.mjs";
import { analyzeWorkflows } from "./analysis.mjs";
import { runScenario, runEnduranceStress } from "./execution.mjs";
import { buildQualificationReport, qualificationPassed } from "./report.mjs";

/** Runs an explicitly targeted disposable workspace and saves fresh, local evidence for this invocation. */
export async function main({
  env = process.env,
  connect = connectCDP,
  generate = generateAllWorkflows,
  scenario = runScenario,
  endurance = runEnduranceStress,
  outputRoot = fileURLToPath(new URL("./results/", import.meta.url)),
  signal,
} = {}) {
  const workspaceId = env.ORA_QUALIFICATION_WORKSPACE_ID?.trim();
  if (!workspaceId) {
    throw new Error(
      "Set ORA_QUALIFICATION_WORKSPACE_ID to an active, disposable isolated workspace. Agents may modify its files.",
    );
  }
  const workflows = generate();
  const context = {
    workspaceId,
    startedAt: new Date().toLocaleString("sv-SE"),
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    build: {
      label: env.ORA_QUALIFICATION_BUILD_ID ?? "UNKNOWN",
      source: "operator-provided, unverified",
    },
    graphDigest: createHash("sha256")
      .update(JSON.stringify(workflows))
      .digest("hex"),
  };
  fs.mkdirSync(outputRoot, { recursive: true });
  // Never resume prior results: a fresh directory binds evidence to this build label, workspace, and graphs.
  const outputDirectory = fs.mkdtempSync(path.join(outputRoot, "run-"));
  const resultsFile = path.join(outputDirectory, "qualification_results.json");
  const reportFile = path.join(outputDirectory, "qualification_report.md");
  const log = (message) => {
    const line = `[${new Date().toLocaleString("sv-SE")}] ${message}`;
    console.log(line);
    fs.appendFileSync(path.join(outputDirectory, "runner.log"), `${line}\n`);
  };
  let session;
  let analyses = [];
  const results = [];
  let fatalError = null;
  const save = () =>
    fs.writeFileSync(
      resultsFile,
      JSON.stringify({ context, analyses, results, fatalError }, null, 2),
    );
  try {
    if (signal?.aborted) throw new Error("Qualification interrupted.");
    session = await connect({
      port: env.CDP_PORT,
      targetId: env.ORA_QUALIFICATION_CDP_TARGET_ID,
    });
    const listed = await session.invoke("list_workspaces", {});
    const workspace = listed?.workspaces?.find(
      (entry) => entry.id === workspaceId,
    );
    if (
      !workspace ||
      workspace.lifecycle !== "active" ||
      workspace.kind !== "isolated"
    ) {
      throw new Error(
        `Workspace ${workspaceId} must be an active isolated disposable workspace.`,
      );
    }
    context.workspace = workspace;
    log(`Analyzing ${workflows.length} graphs before creating any workflows.`);
    analyses = await analyzeWorkflows(session, workflows, { signal });
    save();
    if (analyses.some((analysis) => !analysis.valid)) {
      throw new Error("Graph preflight failed; no workflows were executed.");
    }
    for (const workflow of workflows) {
      if (signal?.aborted) throw new Error("Qualification interrupted.");
      log(`Running ${workflow.index}: ${workflow.name}`);
      const execute = workflow.isStress ? endurance : scenario;
      const result = await execute(session, workflow, workspaceId, { signal });
      results.push(result);
      save();
      log(
        `${result.qualified ? "PASS" : "FAIL"}: ${workflow.name} (${result.status})`,
      );
      if (!result.cleanupComplete) {
        throw new Error(
          `Owned resources could not be settled and removed for ${workflow.name}; qualification stopped.`,
        );
      }
    }
  } catch (error) {
    fatalError = error instanceof Error ? error.message : String(error);
    log(`Qualification stopped: ${fatalError}`);
  } finally {
    session?.close();
    save();
    fs.writeFileSync(
      reportFile,
      buildQualificationReport({
        context,
        workflows,
        analyses,
        results,
        fatalError,
      }),
    );
  }
  const passed = qualificationPassed({
    workflows,
    analyses,
    results,
    fatalError,
  });
  log(
    `${passed ? "PASS" : "FAIL / INCOMPLETE"}: local report at ${reportFile}`,
  );
  return { passed, outputDirectory, reportFile, resultsFile };
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  const controller = new AbortController();
  const stop = () => controller.abort();
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
  main({ signal: controller.signal })
    .then((result) => {
      if (!result.passed) process.exitCode = 1;
    })
    .catch((error) => {
      console.error(error.message);
      process.exitCode = 1;
    })
    .finally(() => {
      process.removeListener("SIGINT", stop);
      process.removeListener("SIGTERM", stop);
    });
}
