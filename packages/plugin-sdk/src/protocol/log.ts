/**
 * Prefix of every structured log record the SDK writes to stderr, trailing space included.
 *
 * The host recognizes the format by this prefix and treats everything else on stderr as raw
 * `INFO` text. The prefix identifies a format, not an authority: the host still overwrites the
 * trusted identity fields itself.
 */
export const PLUGIN_LOG_ENVELOPE_V1_PREFIX = "@ora/plugin-log/v1 ";

/** Levels a plugin log record may carry, least to most severe. */
export const PLUGIN_LOG_LEVELS = [
  "TRACE",
  "DEBUG",
  "INFO",
  "WARN",
  "ERROR",
] as const;

/** One of the closed set of plugin log levels. */
export type PluginLogLevel = (typeof PLUGIN_LOG_LEVELS)[number];

/** Upper bound on `target` and `method`; longer values are rejected by the host. */
export const MAX_PLUGIN_LOG_IDENTIFIER_CHARS = 256;

/**
 * Upper bound on one serialized record; the host splits anything longer into raw fragments,
 * which would lose the structure this SDK went to the trouble of encoding.
 */
export const MAX_PLUGIN_LOG_RECORD_BYTES = 64 * 1024;
