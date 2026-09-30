import assert from "node:assert/strict";
import test from "node:test";
import { connectCDP } from "./cdp.mjs";

/** Emulates only the public WebSocket API used by the transport, never a live debugger. */
function fixture(respond = () => ({ result: { value: "ok" } }), pages) {
  let socket;
  class Socket extends EventTarget {
    closed = false;
    requests = [];
    constructor() {
      super();
      socket = this;
      queueMicrotask(() => this.dispatchEvent(new Event("open")));
    }
    send(message) {
      const request = JSON.parse(message);
      this.requests.push(request);
      const result =
        request.method === "Runtime.enable"
          ? {}
          : request.params.expression.startsWith("Boolean(")
            ? { result: { value: true } }
            : respond(request);
      if (result === undefined) return;
      queueMicrotask(() =>
        this.dispatchEvent(
          new MessageEvent("message", {
            data: JSON.stringify({ id: request.id, result }),
          }),
        ),
      );
    }
    close() {
      this.closed = true;
      this.dispatchEvent(new Event("close"));
    }
  }
  return {
    options: {
      port: "9222",
      commandTimeoutMs: 20,
      discoveryTimeoutMs: 20,
      WebSocketImpl: Socket,
      fetchImpl: () => ({
        ok: true,
        json: () =>
          pages ?? [
            {
              id: "ora",
              type: "page",
              webSocketDebuggerUrl: "ws://127.0.0.1/ora",
            },
          ],
      }),
    },
    get socket() {
      return socket;
    },
  };
}

test("CDP propagates JavaScript exceptions instead of accepting an empty success", async () => {
  const transport = fixture(() => ({
    exceptionDetails: { text: "TypeError: broken page" },
  }));
  const session = await connectCDP(transport.options);
  try {
    await assert.rejects(
      session.invoke("analyze_workflow", { graph: "{}" }),
      /broken page/,
    );
  } finally {
    session.close();
  }
  assert.equal(transport.socket.closed, true);
});

test("a stalled CDP command closes the transport and rejects every pending request", async () => {
  const transport = fixture(() => undefined);
  const session = await connectCDP(transport.options);
  const pending = await Promise.allSettled([
    session.invoke("get_workflow_run", { runId: "one" }),
    session.invoke("get_workflow_run", { runId: "two" }),
  ]);
  assert.deepEqual(
    pending.map((result) => result.status),
    ["rejected", "rejected"],
  );
  assert.ok(pending.every((result) => /timed out/.test(result.reason.message)));
  assert.equal(transport.socket.closed, true);
  await assert.rejects(
    session.invoke("create_workflow", {}),
    /closed|timed out/,
  );
});

test("socket close or error rejects pending requests immediately", async () => {
  for (const eventType of ["close", "error"]) {
    const transport = fixture(() => undefined);
    const session = await connectCDP(transport.options);
    const pending = session.invoke("get_workflow_run", {});
    transport.socket.dispatchEvent(new Event(eventType));
    await assert.rejects(pending, /closed|error/);
    assert.equal(transport.socket.closed, true);
  }
});

test("malformed CDP messages reject pending requests and close the transport", async () => {
  const transport = fixture(() => undefined);
  const session = await connectCDP(transport.options);
  const pending = session.invoke("get_workflow_run", {});
  transport.socket.dispatchEvent(
    new MessageEvent("message", { data: "bad-json" }),
  );
  await assert.rejects(pending, /Malformed/);
  assert.equal(transport.socket.closed, true);
});

test("multiple page targets require an explicit debugger target rather than the first page", async () => {
  const pages = ["one", "two"].map((id) => ({
    id,
    type: "page",
    webSocketDebuggerUrl: `ws://127.0.0.1/${id}`,
  }));
  const transport = fixture(undefined, pages);
  await assert.rejects(
    connectCDP(transport.options),
    /ORA_QUALIFICATION_CDP_TARGET_ID/,
  );
  assert.equal(transport.socket, undefined);
});

test("CDP discovery is bounded even when the HTTP endpoint stalls", async () => {
  const transport = fixture();
  const fetchImpl = (_url, { signal }) =>
    new Promise((_resolve, reject) => {
      signal.addEventListener(
        "abort",
        () => reject(new Error("Discovery aborted")),
        { once: true },
      );
    });
  await assert.rejects(
    connectCDP({ ...transport.options, fetchImpl }),
    /Discovery|discovery/,
  );
});
