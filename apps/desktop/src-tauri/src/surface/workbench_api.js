// Injected into every workbench webview before the page runs. Exposes the narrow bridge a page
// uses to reach its own plugin process through the single `plugin_webview_invoke` command. The
// page cannot reach any other Tauri command: the ACL grants this webview label nothing else.
(function () {
  "use strict";
  if (window.ora !== undefined) {
    return;
  }

  function invoke(method, params) {
    const internals = window.__TAURI_INTERNALS__;
    if (internals === undefined || typeof internals.invoke !== "function") {
      return Promise.reject({ kind: "host", code: "INTERNAL" });
    }
    // The host takes the caller identity from the webview label, so the request body carries
    // only the method and its params. A page cannot name a plugin, surface, or generation.
    // Tauri maps top-level invoke arguments onto the command's parameter names, so the body
    // must nest the payload under `request` to reach the command's `request` parameter.
    return internals.invoke("plugin_webview_invoke", {
      request: {
        method: method,
        params: params === undefined ? null : params,
      },
    });
  }

  Object.defineProperty(window, "ora", {
    value: Object.freeze({ invoke: invoke }),
    writable: false,
    configurable: false,
    enumerable: true,
  });
})();
