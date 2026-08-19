/**
 * The SDK release this module tree was published as.
 *
 * Reported to the host in `ora/register` so a mismatch between a plugin's SDK and the host's
 * expectations is visible at the handshake instead of at the first failing call. The release
 * workflow rewrites this constant from the tag; it must match `deno.json` and `package.json`.
 */
export const SDK_VERSION = "1.0.0";

/** The plugin protocol version this SDK speaks (frame format, `ora/register`, `ora/shutdown`). */
export const PLUGIN_API_VERSION = 1;
