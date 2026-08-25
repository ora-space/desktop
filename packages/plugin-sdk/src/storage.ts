import type { Plugin } from "./plugin.ts";
import type { JsonValue } from "./protocol.ts";

const STORAGE_LIST = "ora/storage/list";
const STORAGE_READ = "ora/storage/read";
const STORAGE_WRITE = "ora/storage/write";
const STORAGE_REMOVE = "ora/storage/remove";

/** One entry returned by `storage.list`. */
export interface StorageEntry {
  name: string;
  kind: "file" | "directory";
  sizeBytes: number;
}

/**
 * Reads and writes the plugin's private data directory through the host.
 *
 * Paths are logical, slash-separated, and relative to the data directory Ora resolved for this
 * plugin (`data/<namespace>/<name>/`); the host refuses absolute paths, `..`, symlinks, and the
 * host-owned `web-profile/` directory. `downloads/` is where Ora puts files downloaded from a
 * surface, using exactly the `path` carried by `onDownloadCompleted`.
 */
export interface PluginStorage {
  /** Lists the entries directly below `path` (`""` for the data directory itself). */
  list(path: string): Promise<StorageEntry[]>;
  /** Reads one whole file; files above 8 MiB are refused with `too_large`. */
  read(path: string): Promise<Uint8Array>;
  /** Atomically replaces one file, creating missing parent directories. */
  write(path: string, bytes: Uint8Array): Promise<void>;
  /** Removes one file or one directory tree. */
  remove(path: string): Promise<void>;
}

/** Builds the storage client on top of a plugin's host-request channel. */
export function createStorage(plugin: Plugin): PluginStorage {
  return {
    async list(path) {
      const result = await plugin.request(STORAGE_LIST, { path });
      return parseEntries(result);
    },
    async read(path) {
      const result = await plugin.request(STORAGE_READ, { path });
      if (!isRecord(result) || typeof result.bytes_base64 !== "string") {
        throw new Error(`${STORAGE_READ} returned an invalid result`);
      }
      return decodeBase64(result.bytes_base64);
    },
    async write(path, bytes) {
      await plugin.request(STORAGE_WRITE, {
        path,
        bytes_base64: encodeBase64(bytes),
      });
    },
    async remove(path) {
      await plugin.request(STORAGE_REMOVE, { path });
    },
  };
}

/** Validates the wire shape of a list result and maps it to camelCase entries. */
function parseEntries(result: JsonValue): StorageEntry[] {
  if (!isRecord(result) || !Array.isArray(result.entries)) {
    throw new Error(`${STORAGE_LIST} returned an invalid result`);
  }
  return result.entries.map((entry) => {
    if (
      !isRecord(entry) || typeof entry.name !== "string" ||
      (entry.kind !== "file" && entry.kind !== "directory") ||
      typeof entry.size_bytes !== "number"
    ) {
      throw new Error(`${STORAGE_LIST} returned an invalid entry`);
    }
    return { name: entry.name, kind: entry.kind, sizeBytes: entry.size_bytes };
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Encodes bytes as standard base64 without building one giant intermediate string. */
export function encodeBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunk) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
  }
  return btoa(binary);
}

/** Decodes standard base64 into bytes. */
export function decodeBase64(encoded: string): Uint8Array {
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}
