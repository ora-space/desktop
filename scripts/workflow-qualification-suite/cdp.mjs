/** Connects to one explicitly selected Ora page with bounded HTTP, handshake, and protocol requests. */
export async function connectCDP({
  port = process.env.CDP_PORT ?? "9222",
  targetId = process.env.ORA_QUALIFICATION_CDP_TARGET_ID,
  commandTimeoutMs = 30_000,
  discoveryTimeoutMs = 10_000,
  fetchImpl = globalThis.fetch,
  WebSocketImpl = globalThis.WebSocket,
} = {}) {
  if (
    !/^\d+$/.test(String(port)) ||
    Number(port) < 1 ||
    Number(port) > 65_535
  ) {
    throw new Error("CDP_PORT must be a valid local TCP port.");
  }
  for (const timeout of [commandTimeoutMs, discoveryTimeoutMs]) {
    if (!Number.isFinite(timeout) || timeout <= 0)
      throw new Error("CDP timeouts must be positive.");
  }
  const controller = new AbortController();
  let discoveryTimer;
  let list;
  try {
    list = await Promise.race([
      (async () => {
        const response = await fetchImpl(`http://127.0.0.1:${port}/json/list`, {
          signal: controller.signal,
        });
        if (!response.ok)
          throw new Error(`CDP discovery HTTP ${response.status}`);
        return response.json();
      })(),
      new Promise((_resolve, reject) => {
        discoveryTimer = setTimeout(() => {
          reject(
            new Error(
              `CDP discovery timed out after ${discoveryTimeoutMs} ms.`,
            ),
          );
          controller.abort();
        }, discoveryTimeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(discoveryTimer);
  }
  if (!Array.isArray(list))
    throw new Error("CDP discovery did not return page targets.");
  const pages = list.filter(
    (page) =>
      page.type === "page" && typeof page.webSocketDebuggerUrl === "string",
  );
  const target = targetId
    ? pages.find((page) => page.id === targetId)
    : pages.length === 1
      ? pages[0]
      : null;
  if (!target) {
    throw new Error(
      "Select exactly one Ora debugger page with ORA_QUALIFICATION_CDP_TARGET_ID.",
    );
  }

  const socket = new WebSocketImpl(target.webSocketDebuggerUrl);
  const pending = new Map();
  let nextId = 1;
  let closedError = null;
  let rejectOpening;
  let openingTimer;

  const fail = (error) => {
    if (closedError) return;
    closedError = error;
    clearTimeout(openingTimer);
    rejectOpening?.(error);
    rejectOpening = undefined;
    for (const request of pending.values()) {
      clearTimeout(request.timer);
      request.reject(error);
    }
    pending.clear();
    socket.removeEventListener("message", onMessage);
    socket.removeEventListener("error", onError);
    socket.removeEventListener("close", onClose);
    socket.removeEventListener("open", onOpen);
    try {
      socket.close();
    } catch {
      /* Closing an unopened or broken socket is best effort. */
    }
  };
  const onMessage = (event) => {
    let message;
    try {
      message = JSON.parse(event.data);
      if (message === null || typeof message !== "object")
        throw new Error("Expected a protocol object.");
    } catch {
      fail(new Error("Malformed CDP message."));
      return;
    }
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error)
      request.reject(
        new Error(
          `CDP ${request.method}: ${message.error.message ?? JSON.stringify(message.error)}`,
        ),
      );
    else request.resolve(message.result);
  };
  const onError = () => fail(new Error("CDP socket error; connection closed."));
  const onClose = () => fail(new Error("CDP socket closed."));
  let resolveOpening;
  const onOpen = () => {
    clearTimeout(openingTimer);
    rejectOpening = undefined;
    resolveOpening();
  };
  const opened = new Promise((resolve, reject) => {
    resolveOpening = resolve;
    rejectOpening = reject;
    openingTimer = setTimeout(
      () => fail(new Error("CDP connection timed out.")),
      commandTimeoutMs,
    );
  });
  socket.addEventListener("open", onOpen, { once: true });
  socket.addEventListener("message", onMessage);
  socket.addEventListener("error", onError);
  socket.addEventListener("close", onClose);

  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      if (closedError) {
        reject(closedError);
        return;
      }
      const id = nextId++;
      const timer = setTimeout(
        () =>
          fail(
            new Error(
              `CDP ${method} timed out after ${commandTimeoutMs} ms; connection closed.`,
            ),
          ),
        commandTimeoutMs,
      );
      pending.set(id, { method, resolve, reject, timer });
      try {
        socket.send(JSON.stringify({ id, method, params }));
      } catch (error) {
        fail(error instanceof Error ? error : new Error(String(error)));
      }
    });
  const evaluate = async (expression) => {
    const response = await send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
      userGesture: true,
    });
    if (response?.exceptionDetails)
      throw new Error(
        `CDP evaluation failed: ${JSON.stringify(response.exceptionDetails)}`,
      );
    if (!response?.result || response.result.subtype === "error") {
      throw new Error(
        `CDP evaluation returned no value: ${JSON.stringify(response)}`,
      );
    }
    return response.result.value;
  };
  try {
    await opened;
    await send("Runtime.enable");
    if (
      (await evaluate("Boolean(window.__TAURI_INTERNALS__?.invoke)")) !== true
    ) {
      throw new Error("Selected debugger page is not an Ora Tauri page.");
    }
  } catch (error) {
    fail(error instanceof Error ? error : new Error(String(error)));
    throw error;
  }
  return {
    send,
    evaluate,
    invoke: (command, request = {}) =>
      evaluate(
        `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, {request: ${JSON.stringify(request)}})`,
      ),
    close: () => fail(new Error("CDP session closed by the runner.")),
  };
}
