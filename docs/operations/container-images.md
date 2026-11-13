# Container Images

English | [中文](container-images.zh.md)

`docker/Dockerfile` builds the Node and Controller images from one release build of the workspace, so
building both compiles once. The local stack in `ora-space/cluster` builds and runs both.

```sh
docker build -f docker/Dockerfile --target node -t ora-node:local .
docker build -f docker/Dockerfile --target controller -t ora-controller:local .
```

Both use a Debian bookworm runtime; the build stage uses the compiler from `rust-toolchain.toml`.

## Node image

One container per Workspace sandbox, started by the Sandbox Server (never by Compose).

| Input | Rule |
|---|---|
| `ORA_NODE_CONFIG` | The complete [Node configuration](../node/node.md) as JSON; empty refuses to start |
| `ORA_NODE_CERT`, `ORA_NODE_KEY`, `ORA_NODE_CA`, `ORA_CONTROLLER_CERT` | Base64 PEM for the Node's mutual-TLS control listener, see [Control session](../node/control-session.md) |
| `ORA_MODEL_CLIENT_CERT`, `ORA_MODEL_CLIENT_KEY`, `ORA_MODEL_CA` | Required when the configuration has `agent.model_proxy` |
| `ORA_GIT_CLIENT_CERT`, `ORA_GIT_CLIENT_KEY`, `ORA_GIT_CA` | Required when the configuration has `git_access` |
| `/var/lib/ora` | Mount point of the Workspace volume; every state path in the configuration lives below it |
| `/etc/ora/clone.gitconfig` | Clone Git configuration; the image ships an empty, root-owned file without credentials |

The management process (the Node) runs as root and owns the protected configuration and journals.
Git workloads run as UID/GID 1000 (`node`), an identity each such process takes between fork and
exec, and so do agent plugins and their processes when the configuration names an agent workload
directory; the Node's network control listener requires that separate identity.

The entrypoint runs under `tini` as PID 1, so processes reparented to it (what outlived a killed
Node) are reaped. It:

1. sets the volume root to `root:root` with mode `0711` (never its contents), creates the clone root and,
   when configured, the agent workload directory (`0711`), which the Node requires and never creates;
2. writes the configuration to `/run/ora/node.json` and each certificate and key to `/run/ora/*.pem`
   (mode `0600`, in a root-only directory) and unsets the variables;
3. starts `ora-node`, the only process it supervises: the Node runs every Git command itself and
   starts its own `ora-reaper`, see [Node](../node/node.md#execution-and-recovery).

`TERM` and `INT` are forwarded to the Node, which ends the Git it runs and waits for its stop
evidence. The Sandbox Server's stop timeout must therefore exceed the Node's
`shutdown_grace_ms + cleanup_timeout_ms`. The container exits with the Node's status.

Binaries live under the root-owned `/opt/ora/bin`; `ora-reaper` sits next to `ora-node`, which starts
it from its own directory. The image also ships `/opt/ora/bin/deno`, the runtime every plugin runs
on: its version is `.deno-version`, and the build verifies the archive and executable against
`scripts/sidecar-checksums.json` (Linux x64 and arm64), failing on an architecture without a pin.
Agent sessions use it through `agent.deno_path`.

`ora-git` is installed at `/usr/local/bin/ora-git`, on the workload `PATH`, so the Agent's processes
find it. It only sends fixed requests to the current session's `ORA_GIT_SOCKET`, whose directory is
`agent.ora_git_directory`; Git Server credentials stay with the Node. See [ora-git](../node/ora-git.md).

## Controller image

Runs `ora-controller` as the unprivileged `controller` user (UID 10001) with `/var/lib/ora-controller`
as a private home. The deployment mounts the configuration and passes `--config`. In the cloud form the
process serves no listener, so the image exposes no port. See [Controller](../controller/controller.md).
