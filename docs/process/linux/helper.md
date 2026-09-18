# Linux process helper deployment and management

English | [中文](helper.zh.md)

The independent privileged helper direction is approved in the
[Linux follow-up ADR](../../../specs/decisions/node/process/containment/linux/20260917-independent-privileged-helper.md).
The executable provides deployment preflight and an **authenticated inspection-only listener**.
It has no launch API, service installer or guardian integration, and does not advertise strong containment.

This route is paused at the current checkpoint while development prioritizes the
[fully rootless best-effort adapter](rootless.md). Existing helper code is retained;
no privileged deployment is required by that adapter.

## Build and configuration

Build with `cargo build -p ora-process-helper`. A deployment administrator can explicitly run the
binary as a separate root process using `ora-process-helper --check /etc/ora/process-helper.json`.
Do not install it setuid. Neither `--check` nor `--serve` modifies cgroup membership.
Implementation does not install binaries, create accounts or start privileged services automatically.

Configuration version 1 uses this shape (the numeric identities are examples, not defaults):

```json
{
  "version": 1,
  "manager_uid": 1000,
  "workload_uid": 2000,
  "workload_gid": 2000,
  "cgroup_root": "/sys/fs/cgroup/ora-workloads"
}
```

The manager and workload UIDs must be different and nonzero; the workload GID must be nonzero.
All three reject `4294967295`, which credential syscalls can interpret as "leave unchanged".
Unknown fields and versions are rejected. Configuration is limited to 16 KiB and must be a regular
root-controlled file. Every ancestor must also be root-controlled, not group/other writable, and
free of symlinks. The shared `ora-utils::path::open_trusted_path` pins checked directory/file
descriptors instead of validating a path and following a replacement link afterward.

The cgroup root must already exist, be root-controlled, use cgroup v2, have `domain` type, contain
no directly attached processes and expose `cgroup.events` plus a writable `cgroup.kill`.
Preflight never writes to these files. The helper itself must remain outside the workload root.
Checking succeeds only for these prerequisites: it does not prove all descendants are gone,
freeze future permissions, verify account provisioning or grant permission to launch.

## Inspection service

An administrator can explicitly run `ora-process-helper --serve /etc/ora/process-helper.json /run/ora-helper/control.sock`.
The parent directory must already exist and satisfy the same root-controlled, non-writable-by-others,
no-symlink checks. The endpoint is owned by `manager_uid`, mode `0600`; its parent must allow that
manager to traverse it. Existing files or sockets are never replaced. SIGINT/SIGTERM closes the
listener and cancels exchanges, then removes only the socket inode created by this service.
Crash leftovers require administrator inspection/removal before restart; automatic recovery is pending.

Each connection is authenticated using the kernel's connecting peer UID before any payload is read.
The manager must not pass authenticated sockets to workloads: this is connection-time identity,
not per-message reauthentication. The manager can query availability, never choose a command, UID,
PID or cgroup target. Wire declarations belong to `ora-process-protocol`.

Each connection carries one request and one reply: a four-byte big-endian length followed by UTF-8 JSON.
Request: `{"version":1,"operation":"inspect"}`. Reply: `{"version":1,"status":"launch_unavailable"}`.
Other statuses are `unauthorized`, `invalid_request` and `unsupported_version`. Unknown fields are
rejected. Requests are limited to 16 KiB before allocation; the entire accepted exchange has a
five-second deadline, including reply writes. Truncated frames and timeouts close the connection.
At most 16 exchanges run concurrently; remaining connections stay in the bounded OS listen queue.
These transport limits are not workload shutdown policy. Inspection does not revalidate cgroup state.

## Low-level launch gate (not exposed over IPC)

`ora_process_runtime::spawn_linux_helper_workload` is a building block for trusted helper code,
not a production Run API. It reloads root-owned configuration and accepts only an existing
`cgroup_root/scope/run` target. It creates no directories and owns neither admission decisions nor
a journal: the future lifecycle owner must enforce exclusive admission and retain cleanup responsibility.
In particular, this function itself does **not** deduplicate calls or make a Run durable.

The gate checks root-controlled migration ancestors, empty direct membership, an empty/unfrozen
Run subtree, and a writable cgroup v2 membership file. Before exec, the forked child writes `0` to
the pinned membership descriptor, starts a separate session, marks non-stdio descriptors close-on-exec,
sets `no_new_privs`, clears supplementary groups, sets all real/effective/saved GIDs and UIDs, and
clears permitted/effective/inheritable capabilities. It then changes cwd under the workload identity.
Any failed step aborts exec. Missing kernel support fails closed. The membership descriptor is
duplicated above fd 2 so stdio setup cannot overwrite it even when the helper starts with closed stdio.
The mechanism follows [Rust's pre-exec constraints](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#tymethod.pre_exec)
and [cgroup v2 process migration](https://docs.kernel.org/admin-guide/cgroup-v2.html#organizing-processes-and-threads).

Executable and cwd must be absolute. The helper's environment and stdio are not inherited: only
the supplied environment is used and the returned `Child` owns three pipes. The caller must drain
output, close input, reap the direct child and enforce the descendant policy separately. Returning
an error is not a cleanup/retirement certificate. Trusted administrators must not concurrently mutate
the launch boundary; freezing it during synchronous spawn can block startup. Crash reconciliation,
startup cancellation and stable handoff remain prerequisites to exposing launch over IPC.

### Privileged acceptance test

Normal tests exercise sentinel rejection and failure-before-execution without elevation.
`helper_launch_privileged` contains an explicitly ignored test for a **disposable, administrator-provisioned**
Linux environment. It requires root, cgroup v2 with `cgroup.kill`, and a dedicated trusted configuration
whose workload identity differs from root and the manager. Set `ORA_PROCESS_HELPER_TEST_CONFIG` to that
configuration and explicitly run `cargo test -p ora-process-runtime --test helper_launch_privileged -- --ignored`.
This is an administrator action, not part of the ordinary three-platform CI matrix.

The test creates only fresh Scope/Run directories beneath that configured test root. It probes
credentials, capabilities, no-new-privileges, actual membership, rejected sibling/ancestor migration,
descriptor/environment isolation and post-drop cwd access. Cleanup kills/removes only those fresh
directories; it never deletes the configured root. Failed cleanup may leave directories for inspection.
The test has not been run in the ordinary user environment; compilation is not positive acceptance evidence.

## Remaining boundary

The launch gate still needs positive privileged verification, Run/Scope resource ownership,
descriptor handoff, workspace access provisioning, guardian survival and helper recovery. The root helper must never become a
general-purpose arbitrary-root-command or arbitrary-PID migration interface.

Current tests exercise configuration rejection, non-cgroup filesystem rejection, CLI failure and
trusted path handling without privilege elevation. Real Unix-socket tests additionally cover peer
authentication, strict framing, timeout and listener shutdown on an unprivileged Linux runner.
Positive root/cgroup deployment and endpoint ownership/cleanup tests still require an
explicitly provisioned environment. The crates CI job now runs on Linux, macOS and Windows;
that matrix alone is not containment evidence.
