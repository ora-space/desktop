import {
  MAX_PLUGIN_LOG_IDENTIFIER_CHARS,
  MAX_PLUGIN_LOG_RECORD_BYTES,
  PLUGIN_LOG_ENVELOPE_V1_PREFIX,
  type PluginLogLevel,
} from "./protocol/index.ts";

/** Optional fields a plugin may attach to one record. */
export interface PluginLogFields {
  /** Component or module the record belongs to; the host applies a stable default when absent. */
  target?: string;
  /** Plugin method or operation the record was produced within. */
  method?: string;
  /** Free-form structured context; the host overwrites `plugin_id` and `generation` with its own. */
  context?: Record<string, unknown>;
  /** An `Error` (or anything thrown) rendered into a bounded description. */
  error?: unknown;
}

/** Defaults a child logger merges into every record it writes. */
export interface PluginLoggerDefaults {
  target?: string;
  context?: Record<string, unknown>;
}

/**
 * Structured logger whose records the host persists into this plugin's own log file.
 *
 * Every method returns without throwing whatever it is given: logging must never turn a plugin
 * business method into a failure, and it never touches stdout, which carries the protocol.
 */
export interface PluginLogger {
  trace(message: string, fields?: PluginLogFields): void;
  debug(message: string, fields?: PluginLogFields): void;
  info(message: string, fields?: PluginLogFields): void;
  warn(message: string, fields?: PluginLogFields): void;
  error(message: string, fields?: PluginLogFields): void;
  /** Derives a logger that applies `defaults` under whatever each call supplies. */
  child(defaults: PluginLoggerDefaults): PluginLogger;
}

/** Destination of encoded records; production writes to stderr, tests capture lines. */
export interface PluginLogSink {
  write(line: string): void;
}

/** Longest `message` kept before truncation; the record limit bounds the rest. */
const MAX_MESSAGE_CHARS = 16 * 1024;
/** Deepest `cause` chain rendered from an error. */
const MAX_ERROR_CAUSE_DEPTH = 4;
/** Deepest object nesting rendered from context before values collapse to a placeholder. */
const MAX_CONTEXT_DEPTH = 8;
const TRUNCATED_SUFFIX = "…[truncated]";

/** Creates a logger that encodes v1 envelopes into `sink`. */
export function createLogger(
  sink: PluginLogSink,
  defaults: PluginLoggerDefaults = {},
): PluginLogger {
  const write = (
    level: PluginLogLevel,
    message: string,
    fields: PluginLogFields = {},
  ) => {
    try {
      sink.write(encodeRecord(level, message, fields, defaults));
    } catch {
      // A sink that cannot be written (closed stderr, EAGAIN) is a diagnostics loss, never a
      // plugin failure; the host accounts for what it did not receive.
    }
  };
  return {
    trace: (message, fields) => write("TRACE", message, fields),
    debug: (message, fields) => write("DEBUG", message, fields),
    info: (message, fields) => write("INFO", message, fields),
    warn: (message, fields) => write("WARN", message, fields),
    error: (message, fields) => write("ERROR", message, fields),
    child: (childDefaults) =>
      createLogger(sink, {
        target: childDefaults.target ?? defaults.target,
        context: { ...defaults.context, ...childDefaults.context },
      }),
  };
}

/** The synchronous byte writer a stderr sink needs; `Deno.stderr` in production. */
export interface SyncByteWriter {
  writeSync(bytes: Uint8Array): number;
}

/**
 * Writes each record synchronously to `writer` (stderr by default) so ordering matches the
 * plugin's own timeline and two records from this process never interleave: the write finishes
 * before the call returns, and a short write is continued until the whole envelope is out.
 */
export function createStderrLogSink(
  writer: SyncByteWriter = Deno.stderr,
): PluginLogSink {
  const encoder = new TextEncoder();
  return {
    write(line) {
      let bytes = encoder.encode(line);
      // `writeSync` may write fewer bytes than offered; looping keeps one record contiguous.
      while (bytes.byteLength > 0) {
        const written = writer.writeSync(bytes);
        if (written <= 0) {
          throw new Error("stderr accepted no bytes");
        }
        bytes = bytes.subarray(written);
      }
    },
  };
}

/** Encodes one record as a single stderr line, degrading hostile values to bounded text. */
export function encodeRecord(
  level: PluginLogLevel,
  message: string,
  fields: PluginLogFields,
  defaults: PluginLoggerDefaults,
): string {
  const payload: Record<string, unknown> = {
    level,
    message: truncate(stringify(message), MAX_MESSAGE_CHARS),
  };
  const target = fields.target ?? defaults.target;
  if (typeof target === "string" && target.length > 0) {
    payload.target = target.slice(0, MAX_PLUGIN_LOG_IDENTIFIER_CHARS);
  }
  if (typeof fields.method === "string" && fields.method.length > 0) {
    payload.method = fields.method.slice(0, MAX_PLUGIN_LOG_IDENTIFIER_CHARS);
  }
  const context = { ...defaults.context, ...fields.context };
  if (Object.keys(context).length > 0) {
    payload.context = renderValue(context, MAX_CONTEXT_DEPTH, new WeakSet());
  }
  if (fields.error !== undefined) {
    payload.error = renderError(fields.error, MAX_ERROR_CAUSE_DEPTH);
  }
  let line = `${PLUGIN_LOG_ENVELOPE_V1_PREFIX}${JSON.stringify(payload)}\n`;
  if (byteLength(line) > MAX_PLUGIN_LOG_RECORD_BYTES) {
    // Context is the only unbounded part left; dropping it keeps the record structured
    // instead of letting the host shred it into raw fragments.
    payload.context = { truncated: true };
    line = `${PLUGIN_LOG_ENVELOPE_V1_PREFIX}${JSON.stringify(payload)}\n`;
  }
  return line;
}

/** Renders any thrown value as a bounded, JSON-safe object with a bounded cause chain. */
function renderError(error: unknown, depth: number): Record<string, unknown> {
  if (error instanceof Error) {
    const rendered: Record<string, unknown> = {
      name: stringify(error.name),
      message: truncate(stringify(error.message), MAX_MESSAGE_CHARS),
    };
    if (typeof error.stack === "string") {
      rendered.stack = truncate(error.stack, MAX_MESSAGE_CHARS);
    }
    if ("cause" in error && error.cause !== undefined) {
      rendered.cause = depth > 0
        ? renderError(error.cause, depth - 1)
        : { truncated: true };
    }
    return rendered;
  }
  return {
    name: "NonError",
    message: truncate(describe(error), MAX_MESSAGE_CHARS),
  };
}

/**
 * Converts an arbitrary value into JSON-serializable data without throwing.
 *
 * Cycles become a marker, `BigInt`/functions/symbols become their description, and a getter or
 * `toJSON` that throws is recorded as unserializable rather than propagated.
 */
function renderValue(
  value: unknown,
  depth: number,
  seen: WeakSet<object>,
): unknown {
  if (value === null || value === undefined) {
    return null;
  }
  switch (typeof value) {
    case "string":
      return truncate(value, MAX_MESSAGE_CHARS);
    case "number":
      return Number.isFinite(value) ? value : String(value);
    case "boolean":
      return value;
    case "bigint":
    case "function":
    case "symbol":
      return describe(value);
    case "object":
      break;
    default:
      return describe(value);
  }
  const object = value as object;
  if (seen.has(object)) {
    return "[circular]";
  }
  if (depth <= 0) {
    return "[depth exceeded]";
  }
  if (object instanceof Error) {
    return renderError(object, MAX_ERROR_CAUSE_DEPTH);
  }
  seen.add(object);
  try {
    if (Array.isArray(object)) {
      return object.map((item) => renderValue(item, depth - 1, seen));
    }
    const rendered: Record<string, unknown> = {};
    for (const key of Object.keys(object)) {
      let entry: unknown;
      try {
        entry = (object as Record<string, unknown>)[key];
      } catch (error) {
        entry = `[unserializable: ${describe(error)}]`;
      }
      rendered[key] = renderValue(entry, depth - 1, seen);
    }
    return rendered;
  } finally {
    seen.delete(object);
  }
}

/** Describes a value as text without invoking user-defined serialization. */
function describe(value: unknown): string {
  try {
    if (value instanceof Error) {
      return `${value.name}: ${value.message}`;
    }
    if (typeof value === "bigint") {
      return `${value}n`;
    }
    if (typeof value === "symbol" || typeof value === "function") {
      return String(value);
    }
    return typeof value === "string"
      ? value
      : Deno.inspect(value, { depth: 2 });
  } catch {
    return "[unrenderable]";
  }
}

/** Coerces a message-like value to a string without letting a hostile `toString` throw. */
function stringify(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  return describe(value);
}

/** Bounds text by character count, marking the cut. */
function truncate(text: string, maxChars: number): string {
  if (text.length <= maxChars) {
    return text;
  }
  return `${text.slice(0, maxChars)}${TRUNCATED_SUFFIX}`;
}

/** Byte length of a string in UTF-8, the unit the host's record limit is expressed in. */
function byteLength(text: string): number {
  return new TextEncoder().encode(text).byteLength;
}

/** The five console methods the SDK takes over; every other member of `console` is untouched. */
export type PluginConsole = Pick<
  Console,
  "debug" | "info" | "log" | "warn" | "error"
>;

const redirectedConsoles = new WeakSet<object>();

/**
 * Routes the five standard console methods of `target` through `logger` so multi-line output
 * stays one record and nothing reaches stdout.
 *
 * Installed once per console object, at the plugin run entrypoint, before any protocol frame is
 * written; it is never uninstalled, so it outlives initialization failures, `stop`, and the end
 * of every business callback. Later callers of the *global* methods — third-party dependencies
 * included — go through the logger too; a method cached before the takeover, another worker or
 * process, and other console methods are outside its reach. A console whose methods cannot be
 * replaced makes this throw, and the caller must then stay out of protocol operation.
 */
export function redirectConsoleToLogger(
  logger: PluginLogger,
  target: PluginConsole = console,
): void {
  if (redirectedConsoles.has(target)) {
    return;
  }
  const render = (values: unknown[]) =>
    values
      .map((value) => (typeof value === "string" ? value : describe(value)))
      .join(" ");
  const consoleTarget = "console";
  target.debug = (...values: unknown[]) =>
    logger.debug(render(values), { target: consoleTarget });
  target.info = (...values: unknown[]) =>
    logger.info(render(values), { target: consoleTarget });
  target.log = (...values: unknown[]) =>
    logger.info(render(values), { target: consoleTarget });
  target.warn = (...values: unknown[]) =>
    logger.warn(render(values), { target: consoleTarget });
  target.error = (...values: unknown[]) =>
    logger.error(render(values), { target: consoleTarget });
  // Marked only once every method is in place, so a partially failed install is retried, not
  // mistaken for a complete one.
  redirectedConsoles.add(target);
}
