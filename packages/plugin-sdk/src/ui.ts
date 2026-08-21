import { createPlugin, type Plugin, PluginMethodError } from "./plugin.ts";
import type { JsonValue } from "./protocol.ts";
import { createStorage, type PluginStorage } from "./storage.ts";

const UI_SURFACE_OPENED = "ora/ui/surface_opened";
const UI_SURFACE_CLOSED = "ora/ui/surface_closed";
const UI_DOWNLOAD_COMPLETED = "ora/ui/download_completed";
const UI_REQUEST = "ora/ui/request";
const UI_PUSH = "ora/ui/push";

/** Identifies one open surface instance on one plugin process generation. */
export interface SurfaceSession {
  surfaceId: string;
  surfaceInstanceId: number;
  /** Generation of the plugin process Ora addressed; echoed back on `push`. */
  pluginGeneration: number;
}

/** A file Ora finished writing into the plugin's `downloads/` directory. */
export interface CompletedDownload {
  id: number;
  pageUrl: string | null;
  sourceUrl: string;
  fileName: string;
  /** Logical storage path (`downloads/<fileName>`), readable through `storage.read`. */
  path: string;
  sizeBytes: number;
  /** Local time, RFC 3339. */
  completedAt: string;
}

/** The `ora/ui/download_completed` event with its session. */
export interface DownloadCompletedEvent {
  session: SurfaceSession;
  download: CompletedDownload;
}

/** One `ora/ui/request` from a panel page, with the page's opaque payload. */
export interface SurfaceRequest {
  session: SurfaceSession;
  payload: JsonValue;
}

/**
 * Implements the ui contract. Every handler is optional, but Ora checks the registration
 * against the manifest at the handshake: a plugin with a `remote_site` surface must provide
 * `onDownloadCompleted`, and one with a `panel` surface must provide `onRequest`.
 */
export interface UiPluginDefinition {
  onSurfaceOpened?(session: SurfaceSession): void | Promise<void>;
  onSurfaceClosed?(session: SurfaceSession): void | Promise<void>;
  onDownloadCompleted?(event: DownloadCompletedEvent): void | Promise<void>;
  /** Answers one page request; the returned value is handed back to the page as its payload. */
  onRequest?(request: SurfaceRequest): JsonValue | Promise<JsonValue>;
}

/** A ui plugin ready to run, with the host channels it may use while running. */
export interface UiPlugin {
  readonly plugin: Plugin;
  readonly storage: PluginStorage;
  /** Pushes one payload to the panel page of `session`; best-effort, never acknowledged. */
  push(session: SurfaceSession, payload: JsonValue): Promise<void>;
  /** Announces the registration and serves Ora until shutdown; see `Plugin.run`. */
  run(...args: Parameters<Plugin["run"]>): Promise<void>;
}

/**
 * Builds a plugin that serves Ora's ui contract with the `ora/ui/*` wire names and snake_case
 * params translated to the camelCase objects above, so plugin code never spells a method name.
 *
 * `ora/ui/push` is always declared: Ora requires the declaration from every panel plugin and it
 * is harmless for a remote-site one. Request-shaped methods are registered only when a handler
 * exists, which is what lets Ora reject an incomplete plugin at the handshake instead of later.
 */
export function defineUiPlugin(definition: UiPluginDefinition): UiPlugin {
  const plugin = createPlugin();
  plugin.declareEmit(UI_PUSH);
  plugin.onNotification(UI_SURFACE_OPENED, (params) => {
    return definition.onSurfaceOpened?.(
      parseSession(params, UI_SURFACE_OPENED),
    );
  });
  plugin.onNotification(UI_SURFACE_CLOSED, (params) => {
    return definition.onSurfaceClosed?.(
      parseSession(params, UI_SURFACE_CLOSED),
    );
  });
  if (definition.onDownloadCompleted !== undefined) {
    const onDownloadCompleted = definition.onDownloadCompleted;
    plugin.registerMethod(UI_DOWNLOAD_COMPLETED, async (params) => {
      await onDownloadCompleted({
        session: parseSession(params, UI_DOWNLOAD_COMPLETED),
        download: parseDownload(params),
      });
      return {};
    });
  }
  if (definition.onRequest !== undefined) {
    const onRequest = definition.onRequest;
    plugin.registerMethod(UI_REQUEST, async (params) => {
      const session = parseSession(params, UI_REQUEST);
      const payload = isRecord(params) ? params.payload ?? null : null;
      return { payload: await onRequest({ session, payload }) };
    });
  }

  return {
    plugin,
    storage: createStorage(plugin),
    push: (session, payload) =>
      plugin.notify(UI_PUSH, {
        surface_id: session.surfaceId,
        surface_instance_id: session.surfaceInstanceId,
        plugin_generation: session.pluginGeneration,
        payload,
      }),
    run: (...args) => plugin.run(...args),
  };
}

/** Reads the three session fields every `ora/ui/*` message carries. */
function parseSession(params: JsonValue, method: string): SurfaceSession {
  if (
    !isRecord(params) || typeof params.surface_id !== "string" ||
    typeof params.surface_instance_id !== "number" ||
    typeof params.plugin_generation !== "number"
  ) {
    throw new PluginMethodError(
      -32602,
      `${method} requires surface_id, surface_instance_id, and plugin_generation`,
    );
  }
  return {
    surfaceId: params.surface_id,
    surfaceInstanceId: params.surface_instance_id,
    pluginGeneration: params.plugin_generation,
  };
}

/** Reads the `download` object of `ora/ui/download_completed`. */
function parseDownload(params: JsonValue): CompletedDownload {
  const download = isRecord(params) ? params.download : undefined;
  if (
    !isRecord(download) || typeof download.id !== "number" ||
    (download.page_url !== null && typeof download.page_url !== "string") ||
    typeof download.source_url !== "string" ||
    typeof download.file_name !== "string" ||
    typeof download.path !== "string" ||
    typeof download.size_bytes !== "number" ||
    typeof download.completed_at !== "string"
  ) {
    throw new PluginMethodError(
      -32602,
      `${UI_DOWNLOAD_COMPLETED} requires a complete download object`,
    );
  }
  return {
    id: download.id,
    pageUrl: download.page_url ?? null,
    sourceUrl: download.source_url,
    fileName: download.file_name,
    path: download.path,
    sizeBytes: download.size_bytes,
    completedAt: download.completed_at,
  };
}

function isRecord(value: unknown): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
