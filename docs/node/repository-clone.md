# Managed repository clone

English | [中文](repository-clone.zh.md)

Linux Node exposes `configure_clone`, `submit_clone` and `recover_clones` to its embedding caller.
The standalone executable accepts an optional `clone` deployment section and recovers already accepted
clones. It does not accept new commands through files, stdin or external IPC, and Backend is unchanged.

```json
{
  "clone": {
    "repository_root": "/home/node/repositories",
    "git_config": "/home/node/deployment/clone.gitconfig",
    "search_path": ["/usr/bin"],
    "ssh": { "kind": "disabled" }
  }
}
```

These are additions to the existing executable configuration, not a complete configuration file.
Provision the root and configuration before startup. All paths must be absolute, controlled by the
current user or root, with no group/other writes or symlink components. Node does not change deployment
permissions. The root cannot overlap Node/process state or configured Worktree checkouts. Unix filesystem
birth-time support is required: device, inode and birth time identify the root and newly created leaf.
Missing or replaced ownership evidence remains Unknown; a database reservation alone never owns a directory.
This is rootless, cooperative-user protection, not isolation from hostile code with the same UID.

The explicit Git configuration may select CA certificates and noninteractive deployment credential helpers.
Only configuration paths are copied into the Run environment; the Worktree environment and ambient HOME
are not inherited. Clone output is discarded so authentication diagnostics cannot become journal output.
For SSH, use `{"kind":"configured","program":"/usr/bin/ssh","config":"/home/node/deployment/ssh_config"}`.
That deployment file selects identities and known hosts; Node forces batch mode and strict host-key checks.
Unknown host keys must be provisioned by deployment, never accepted interactively by Node.

Each accepted request owns a new, exclusively created directory and at most one durable mutation Run.
Git performs a nonlocal, full-history, single-branch clone with no templates, recursive submodules or LFS
smudge/process filters; hooks and interactive prompts are disabled per command. A same-named tag cannot
satisfy the requested branch. Verification requires a normal, nonshallow checkout, independent objects,
the requested source/branch, matching HEAD and fetched branch commit, and clean tracked files.

Recovery first closes the original managed scope, then checks exit and native directory evidence before
reading repository facts. Missing process evidence cannot justify another Run. A reservation with no Run
can resume only if its persisted phase proves dispatch never happened. Known failure retains its directory;
a retry needs new operation/execution identities and receives a different directory. Terminal replay uses
the original durable result and event, even offline or after user edits, without inspecting or recloning.

Real Linux host/guardian tests cover TLS clone, deployment credentials, missing branches/tag-only sources,
authentication failure, checkout extension suppression, replay, pre-existing/replaced paths and Node kill
during acquisition, SSH known-host rejection/success and completion/outbox write-failure recovery.
Linux acceptance requires OpenSSH client/server installed and the system's `/run/sshd` directory provisioned;
the fixture daemon runs as the ordinary test user on an ephemeral port. CI provisions this test dependency,
not a production Node service. This is not Controller delivery/reconnect acceptance or a cross-platform Node runtime.

See [durable records](persistence/repository-acquisition.md) and [runtime ownership](runtime.md).
