/**
 * The desktop build injects the workspace version it reads from Cargo.toml at compile time.
 * Other hosts and the app-shell test suite do not set the global, so they fall back to the
 * workspace default and the settings stamp still renders a value.
 */
declare const __ORA_APP_VERSION__: string | undefined;

export const appVersion =
  typeof __ORA_APP_VERSION__ === "string" && __ORA_APP_VERSION__ !== ""
    ? __ORA_APP_VERSION__
    : "0.0.0";
