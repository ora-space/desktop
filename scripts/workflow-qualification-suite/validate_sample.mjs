import path from "node:path";
import { pathToFileURL } from "node:url";
import { generateAllWorkflows } from "./generator.mjs";
import { connectCDP } from "./cdp.mjs";
import { analyzeWorkflows } from "./analysis.mjs";

/** Analyzes every generated graph without creating definitions, runs, or sessions. */
export async function main({
  connect = connectCDP,
  generate = generateAllWorkflows,
} = {}) {
  const session = await connect();
  try {
    const analyses = await analyzeWorkflows(session, generate());
    for (const analysis of analyses) {
      console.log(
        `${analysis.valid ? "PASS" : "FAIL"} ${analysis.index}: ${analysis.name}${analysis.error ? ` — ${analysis.error}` : ""}`,
      );
    }
    return {
      passed:
        analyses.length > 0 && analyses.every((analysis) => analysis.valid),
      analyses,
    };
  } finally {
    session.close();
  }
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  main()
    .then((result) => {
      if (!result.passed) process.exitCode = 1;
    })
    .catch((error) => {
      console.error(error.message);
      process.exitCode = 1;
    });
}
