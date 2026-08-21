// Injected into every panel webview before the page runs. Exposes the narrow bridge a plugin
// page uses to reach its own process, on top of Tauri's IPC through the single
// `surface_request` command.
(function () {
  "use strict";
  if (window.acquireOraSurfaceApi !== undefined) {
    return;
  }
  // Pushes that arrive before the page registered a listener are replayed on registration so a
  // slow-loading page cannot miss the first message; the buffer is bounded and drops the oldest.
  const PENDING_LIMIT = 64;
  const listeners = new Set();
  const pending = [];

  function invoke(command, args) {
    const internals = window.__TAURI_INTERNALS__;
    if (internals === undefined || typeof internals.invoke !== "function") {
      return Promise.reject({ kind: "host", code: "INTERNAL" });
    }
    return internals.invoke(command, args);
  }

  const api = Object.freeze({
    version: 1,
    request(payload) {
      return invoke("surface_request", {
        payload: payload === undefined ? null : payload,
      });
    },
    onPush(listener) {
      if (typeof listener !== "function") {
        throw new TypeError("onPush expects a function");
      }
      listeners.add(listener);
      const replay = pending.splice(0, pending.length);
      for (const envelope of replay) {
        listener(envelope);
      }
      return () => {
        listeners.delete(listener);
      };
    },
  });

  Object.defineProperty(window, "__ORA_SURFACE_PUSH__", {
    value(envelope) {
      if (listeners.size === 0) {
        if (pending.length >= PENDING_LIMIT) {
          pending.shift();
        }
        pending.push(envelope);
        return;
      }
      for (const listener of listeners) {
        try {
          listener(envelope);
        } catch (error) {
          console.error("ora surface push listener failed", error);
        }
      }
    },
    writable: false,
    configurable: false,
  });
  Object.defineProperty(window, "acquireOraSurfaceApi", {
    value() {
      return api;
    },
    writable: false,
    configurable: false,
  });
})();
