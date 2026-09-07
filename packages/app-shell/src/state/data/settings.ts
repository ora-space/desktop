/** Cache identity owned by settings data; consumers never repeat its tuples. */
export const settingsKeys = {
  developerMode: ["developer-mode"] as const,
  runtimeLogLevel: ["runtime-log-level"] as const,
  proxySettings: ["proxy-settings"] as const,
};
