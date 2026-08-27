# Expose Ora-managed Hook Plugin executables through the agent PATH

Hook Plugins package their executable payloads instead of requiring a separate system installation. Ora exposes those executables through a controlled `PATH` for agent processes without modifying the user's system `PATH`, because tools such as RTK rewrite commands to invoke themselves by bare command name while Agent Plugins remain responsible for agent-specific configuration.

## Considered Options

- Requiring a globally installed executable would make installation from the Ora marketplace incomplete.
- Writing absolute executable paths into agent hook configuration would not cover commands that the hook later rewrites to a bare executable name.

## Consequences

- Ora must detect command-name conflicts and construct deterministic per-agent process environments.
- Uninstalling or disabling a Hook Plugin must remove it from future agent environments without mutating the operating system environment.
