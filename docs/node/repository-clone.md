# Managed snapshot clone and Workspace Git

English | [中文](repository-clone.zh.md)

Cloud freezes a Binding snapshot before handing out a Workspace's clone step. The Controller
forwards `repository_url`, `snapshot_ref`, `base_commit` and `workspace_ref` verbatim. The Node
accepts only HTTPS repository URLs without credentials, queries or fragments; snapshot refs name
a canonical Binding UUID and the exact base commit; Workspace refs name a canonical Workspace UUID
and a concrete branch. Old repository/branch inputs and SSH clone inputs are rejected.

## Deployment and execution

Configure `clone` with an existing absolute `repository_root`, a trusted `git_config` file and
`search_path` (there is no SSH setting: clones come only from the Git Server over HTTPS), and
configure `git_access` with `ca_cert`, `client_cert` and `client_key`. Cloud mode
requires git-access. The Node advertises clone only when both clone and git-access are configured.
The root must not overlap Node/process state or Worktree checkouts. Native device, inode and birth
time evidence protects reservations; unknown or replaced evidence never authorizes a mutation.

The Node fetches only the frozen snapshot into a private bare transport repository under
`<repository_root>/.ora-transport/`. Its Git uses the git-access client certificate and the clone
execution's `Ora-Execution-Id` only as command arguments, with helpers, redirects, hooks and ambient
configuration disabled. The fetched commit must equal `base_commit`.

The transport is handed to the workload user without credentials. Host-managed workload commands
initialize the checkout, fetch the local transport's objects into the Workspace's branch and reset
the working tree. File transport is enabled only for that local fetch. The resulting checkout has
full history, independent objects, no remote, no credential configuration and no tracking refs.
Verification checks its directory identity, branch, clean tracked files and exact base commit.
Transports are removed after use and cleared on startup, except that of a clone whose workload
sequence has not yet finished the local fetch: that step is never preceded by another snapshot
fetch, and its Run may still be reading the transport under its guardian.

## Recovery and failures

Each workload command is durably associated before dispatch. A completed command is verified only
after its managed scope has closed and cleanup is confirmed. Confirmed signal termination reports
`interrupted`; missing process or directory evidence stays Unknown and preserves the directory.
A known failure retains the reservation; business retries use new execution identities and targets.
Terminal replay returns the original result without inspecting or fetching again.

Snapshot fetch may restart because it changes only the private transport. Its pass count is durable
and bounded at three across restarts. Each pass retries transient network/5xx failures three times
with 5/30-second delays. Missing or refused snapshots report `snapshot_unavailable`; a fetched
commit mismatch reports `base_commit_mismatch`; exhausted network failures report
`source_unavailable`. Local commands use `command_timeout_ms`, network work uses
`network_timeout_ms` (default 600000, allowed 1..1200000). Cleanup uncertainty never permits redispatch.

## Agent collaboration

Set `agent.ora_git_directory` to a Node-owned socket directory, normally `/run/ora-git`. The Node
creates a random socket per session and exposes only `ORA_GIT_SOCKET` to its workload. Requests
belong to that socket's session and require the workload UID. They fail as `no_session` once the
Node stops, an `EndSession` is accepted (before the session acts on it), or the session has ended.
One request per session runs at a time; concurrent requests receive `busy`. When the session leaves
the runtime the socket refuses new requests at once but waits for a request whose Git is running,
so nothing it asked for still writes the checkout or the Git Server once the session is recorded as
ended and delivery may start. A client whose connection closes without an answer reports
`no_session`.

The image installs `ora-git` on the workload PATH:

- `info` prints the Workspace identity, branch and baseline without network access.
- `list` lists upstream/Workspace refs with the frozen session directory's Issue and run metadata.
- `fetch upstream` and `fetch workspace <uuid> [branch]` update only `refs/remotes/ora/**`.
- `push [branch]` writes only this Workspace's matching branch, after a local fast-forward check.

Pushes declare the old value from the Node ledger using `--force-with-lease`. Agent-editable tracking
refs never supply that value. A stale lease is `conflict`, unless the remote already holds the
requested commit (successful replay). Non-fast-forward changes are rejected before sending a push.
Fetch preserves local branches, HEAD, index, files and configuration. Git Server authorization uses
the live session execution and remains authoritative. The CLI exposes no URL, refspec or force flag.

See [durable records](persistence/repository-acquisition.md), [Revision delivery](revision-delivery.md)
and [runtime ownership](runtime.md).
