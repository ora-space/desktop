# Trusted-local process host

English | [中文](service.zh.md)

The Linux `ora-process-host` app owns the [host journal and coordinator](storage.md).
It accepts durable Run/Stop/Close intent over local IPC; each original [guardian](../guardian.md)
continues independently of the host or requesting Node connection. No root, helper, token or key is required.
This is a cooperative same-user deployment, not authentication against malicious local programs.

## Deployment and connection

Build both binaries with `cargo build -p ora-process-host -p ora-process-guardian`. Deploy the guardian
executable under a trusted absolute path; its ancestors cannot be group/other writable or symlinks.
Choose an explicit, private local state parent that satisfies the host journal's filesystem and socket
path-length requirements. The app neither reads HOME to select state nor creates missing parents.

```text
ora-process-host create /absolute/private/process-state /absolute/private/bin/ora-process-guardian
ora-process-host recover /absolute/private/process-state /absolute/private/bin/ora-process-guardian
```

`create` requires an absent state directory; `recover` requires the original complete journal and lock.
Failure never falls back to creation. The program runs in the foreground as an independent service;
Node-side installation/bootstrap and Desktop packaging are not implemented. SIGTERM/SIGINT stop host
coordination, not workloads. Explicitly close Scopes and observe closure before terminating work.
Service-manager/container group termination is not equivalent to killing only the host.

`ora_process_client::ProcessHost::new(state_dir, expected_uid)` connects to that explicit directory.
`execute(HostOperation)` supports Inspect, CreateScope, Start, QueryRun, Stop, Close, QueryScope and
bounded Output. A new socket is used per exchange; callers retain original IDs through reconnects.
Start acceptance is not execution success, Stop acceptance is not completed cleanup, and dropping a
client or cancelling a wait does not cancel the accepted operation. Conflicting Run parameters fail.

## Protocol and recovery boundary

Host wire version 2 uses the existing bounded MessagePack frame codec (16 KiB, depth 16). Control and
output use separate `host.sock` / `host-io.sock` endpoints, each with 16 connection slots and a five-second
exchange deadline. Output chunks are at most 4096 bytes; the guardian retains its bounded volatile
capture policy. Slow output clients cannot occupy control slots. Both peers check the kernel UID;
host binding comes from the host's journal, never from a Node-supplied epoch.

Queries expose durable `last_observed` facts and a separate `coordination` status. Historical Running
does not prove a process is still live. Guardian unavailability neither permits relaunch nor erases a
previously recorded result. Failed bootstrap, storage and transport keep original identities and files.

Recovery acquires the original `host.lock` before checking endpoint inodes. Only recognized private,
single-link sockets with a refused connection may be replaced. A live listener, regular file, symlink
or incompatible journal is preserved and rejected. Shutdown does not unlink endpoints. Older binaries
that do not recognize these host endpoint names reject the directory instead of repurposing it.

## Verification and remaining work

After building both apps, run `cargo test -p ora-process-host --test service`. The test deploys real
private binaries and exercises host SIGKILL/reconnect, original guardian/Run continuity, stop and close,
lost Start replies, exactly-once side effects, stalled output peers, host contention and foreign-file
preservation. Test HOME differs from the injected state path. Workspace tests build both apps.

[Standalone Node](../../node/runtime.md) now composes managed Git and durable resource handoff.
Controller IPC and Backend writer cutover remain separate. This app does not yet provide
retirement/garbage collection, stdin, output streaming, privileged Strong containment, non-Linux
adapters, Node authentication or Controller leases. Existing Backend process consumers are unchanged.
