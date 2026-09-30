import { generateAllWorkflows } from "./generator.mjs";

const PORT = process.env.CDP_PORT ?? "9222";
const BASE = `http://127.0.0.1:${PORT}`;

async function main() {
  const list = await (await fetch(`${BASE}/json/list`)).json();
  const target = list.find((t) => t.type === "page" && t.webSocketDebuggerUrl);
  const socket = new globalThis.WebSocket(target.webSocketDebuggerUrl);
  let nextId = 1;
  const pending = new Map();

  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });

  socket.addEventListener("message", (event) => {
    const msg = JSON.parse(event.data);
    const p = pending.get(msg.id);
    if (p) {
      pending.delete(msg.id);
      if (msg.error) p.reject(msg.error);
      else p.resolve(msg.result);
    }
  });

  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });

  await send("Runtime.enable");

  const evaluate = async (expr) => {
    const res = await send("Runtime.evaluate", {
      expression: expr,
      returnByValue: true,
      awaitPromise: true,
    });
    return res.result ? res.result.value : res;
  };

  const workflows = generateAllWorkflows();
  console.log(`Generated ${workflows.length} workflows.`);

  // Validate the first 5 and stress workflow definition
  const testIndices = [1, 21, 41, 61, 81, 101];
  for (const idx of testIndices) {
    const wf = workflows.find((w) => w.index === idx);
    const code = `(async () => {
      try {
        const projects = await window.__TAURI_INTERNALS__.invoke("list_projects", { request: {} });
        const projectId = projects.projects[0].id;
        const workspaces = await window.__TAURI_INTERNALS__.invoke("list_workspaces", { request: { projectId } });
        const workspaceId = workspaces.workspaces[0].id;

        const res = await window.__TAURI_INTERNALS__.invoke("create_workflow", {
          request: { name: "val-" + ${idx} + "-" + Date.now(), graph: ${JSON.stringify(JSON.stringify(wf.graph))} }
        });
        const pub = await window.__TAURI_INTERNALS__.invoke("publish_workflow", {
          request: { workflowId: res.workflow.id, version: "v1.0.0" }
        });
        const run = await window.__TAURI_INTERNALS__.invoke("create_workflow_run", {
          request: { workspaceId, workflowId: res.workflow.id, locale: "zh-CN" }
        });
        return { success: true, workflowId: res.workflow.id, runId: run.run.id };
      } catch (e) {
        return { success: false, error: e };
      }
    })()`;
    const res = await evaluate(code);
    console.log(
      `Workflow ${idx} (${wf.name}):`,
      res.success
        ? `VALID RUN CREATED (runId: ${res.runId})`
        : `FAILED: ${JSON.stringify(res.error)}`,
    );
  }

  socket.close();
}

main().catch(console.error);
