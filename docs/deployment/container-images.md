# Node and Controller container images

English | [中文](container-images.zh.md)

`docker/Dockerfile` builds two images from one release build of the workspace, so building both
compiles once:

```sh
docker build -f docker/Dockerfile --target node -t ora-node:local .
docker build -f docker/Dockerfile --target controller -t ora-controller:local .
```

The local stack in `ora-space/cluster` (`task up`) builds both. Both images use a Debian bookworm
runtime; the build stage uses the same compiler as `rust-toolchain.toml`.

## Node image

One container per sandbox, started by the Sandbox Server (never by Compose). Its contract with the
Sandbox Server:

| Input                      | Rule                                                                                                                                       |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `ORA_NODE_CONFIG`          | The complete Node service configuration as JSON; empty refuses to start.                                                                   |
| `/var/lib/ora`             | Mount point of the Workspace volume. Every state path in the configuration (Node home, process host directory, clone root) lives below it. |
| `/etc/ora/clone.gitconfig` | Clone Git configuration; the image ships an empty, root-owned file without credentials.                                                    |
| User                       | Services run as `node` (UID 1000), matching `process.expected_uid`.                                                                        |

The entrypoint runs under `tini` as PID 1. As root it only sets the volume root to `node` with mode
`0700` (never its contents), creates the clone root the Node requires to exist, and writes the
configuration to `/run/ora/node.json`; it then drops to `node`. It starts the process host with
`create` when the host directory is missing and `recover` otherwise, so the same Workspace volume
keeps its host journal across containers, and a failed recovery never falls back to creating a new
host. It waits until the host socket accepts connections (a leftover socket file does not count)
before starting `ora-node`.

A stop signal goes to the Node first, which closes its managed scopes through the live host; only
after the Node exits is the host stopped. Guardians are not stopped: their journals stay on the
volume and the next container recovers them. The Sandbox Server's stop timeout must therefore exceed
the Node's `shutdown_grace_ms + cleanup_timeout_ms`. If the Node exits on its own, the host is
stopped too and the container exits with the Node's status.

Binaries live under the root-owned `/opt/ora/bin`, which the release host requires for the guardian.

## Controller image

Runs `ora-controller` as the unprivileged `controller` user with `/var/lib/ora-controller` as a
private home directory. The deployment mounts the configuration and passes `--config`; in cloud
persistence the process serves no listener, so the image exposes no port. See
[local Controller runtime](../controller/local-runtime.md) for the configuration.
