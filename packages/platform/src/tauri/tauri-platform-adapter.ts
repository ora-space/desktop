import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import {
  PathSelectionInProgressError,
  type LocationActionsCapability,
  type LocationTarget,
  type PlatformAdapter,
  type SelectPathOptions,
  type WindowControlsCapability,
  type WindowManagerOs,
} from "../types";

/**
 * Reads the host OS from the webview user agent.
 *
 * Tauri paints the same webview on every platform, so the user agent is the one
 * synchronous signal available when the adapter is constructed - no async plugin
 * round-trip, which keeps `windowControls` a plain field the shell can read on
 * first render.
 */
function detectWindowManagerOs(): WindowManagerOs | null {
  if (typeof navigator === "undefined") return null;
  const ua = navigator.userAgent;
  if (/Mac|iPhone|iPad|iPod/i.test(ua)) return "macos";
  if (/Windows|Win32|Win64|WOW64/i.test(ua)) return "windows";
  if (/Linux|X11|CrOS/i.test(ua)) return "linux";
  return null;
}

/**
 * Decides whether the frameless desktop window paints its own controls.
 *
 * macOS keeps its native traffic lights, so it - like the Web host - reports
 * `none`. Windows and Linux ship without any native chrome (the window is
 * created with `decorations: false`), so they get the imperative commands the
 * custom title bar drives.
 */
function createTauriWindowControls(): WindowControlsCapability {
  // Guards the test/SSR paths: the adapter is instantiated in jsdom, where the
  // Tauri IPC bridge is absent and `getCurrentWindow()` would throw reaching for
  // it. Outside a real Tauri window there is nothing to control.
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    return { kind: "none" };
  }

  const os = detectWindowManagerOs();
  if (os === null || os === "macos") {
    return { kind: "none" };
  }

  const appWindow = getCurrentWindow();
  return {
    kind: "overlay",
    os,
    minimize: () => appWindow.minimize(),
    toggleMaximize: () => appWindow.toggleMaximize(),
    close: () => appWindow.close(),
    isMaximized: () => appWindow.isMaximized(),
    subscribeMaximized: (listener) => {
      let active = true;
      // Tauri has no dedicated maximize event, so every resize re-reads the flag.
      // The `active` guard stops a late async read from firing after unsubscribe.
      const unlisten = appWindow.onResized(() => {
        void appWindow.isMaximized().then((maximized) => {
          if (active) listener(maximized);
        });
      });
      return () => {
        active = false;
        void unlisten.then((stop) => stop());
      };
    },
  };
}

/**
 * Wires the location handoff commands, but only inside a real Tauri window.
 *
 * Unlike the window controls, this stays `supported` on macOS too - macOS keeps its
 * native traffic lights (so it paints no window controls) yet still launches Finder,
 * Terminal, and VS Code. The jsdom/SSR guard mirrors the window-control path.
 */
function createTauriLocationActions(): LocationActionsCapability {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    return { kind: "unsupported" };
  }

  return {
    kind: "supported",
    resolveTaskCwd: (taskId) =>
      invoke<{ path: string }>("resolve_task_cwd", { request: { taskId } }).then(
        (response) => response.path,
      ),
    open: (target: LocationTarget, path: string) =>
      invoke("open_location", { request: { target, path } }),
  };
}

/** Delegates path selection to the desktop operating system's native open dialog. */
export class TauriPlatformAdapter implements PlatformAdapter {
  private selectionInProgress = false;

  readonly windowControls: WindowControlsCapability = createTauriWindowControls();

  readonly locationActions: LocationActionsCapability = createTauriLocationActions();

  readonly worktreeStorage = {
    kind: "configurable" as const,
    getRoot: async (): Promise<string> => {
      const config = await invoke<{ worktreeRoot: string }>("get_desktop_config", {
        request: {},
      });
      return config.worktreeRoot;
    },
    setRoot: async (path: string): Promise<void> => {
      await invoke("set_worktree_root", {
        request: { worktreeRoot: path },
      });
    },
  };

  /** Opens one native single-selection dialog configured for a file or directory. */
  async selectPath(options: SelectPathOptions): Promise<string | null> {
    if (this.selectionInProgress) {
      throw new PathSelectionInProgressError();
    }

    this.selectionInProgress = true;
    try {
      return await open({
        directory: options.kind === "directory",
        multiple: false,
        defaultPath: options.initialPath,
      });
    } finally {
      this.selectionInProgress = false;
    }
  }
}

/** Creates the desktop adapter without runtime platform auto-detection. */
export function createTauriPlatformAdapter(): TauriPlatformAdapter {
  return new TauriPlatformAdapter();
}
