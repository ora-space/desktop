# Ora Desktop Documentation

English | [中文](README.zh.md)

These documents describe Ora Desktop and the Cloud runtime executables (Controller, Node) across
module boundaries. Implementation details inside a crate or package live in that directory's
`README.md`; the authoritative source for decisions is the ADRs in
[ora-space/specs](https://github.com/ora-space/specs/tree/main/decisions).

## Where to start

| I want to…                  | Read first                                                                                                       |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Understand the whole system | [Architecture](architecture.md)                                                                                  |
| Run it on my machine        | [Local development](development/local-development.md)                                                            |
| Add or remove a feature     | [Changing the code](development/changing-the-code.md), then the document for the domain                          |
| Run the tests               | [Testing](development/testing.md)                                                                                |
| Deploy the Cloud runtime    | [Container images](operations/container-images.md), [Controller](controller/controller.md), [Node](node/node.md) |

## All documents

**Core**

- [Application boundary and domain models](core/application-boundary.md): crate ownership, entities, contract shapes, public errors, handlers
- [Frontend contracts](core/frontend-contracts.md): the endpoint catalog, generation, the typed client, transports, error localization
- [Frontend architecture](core/frontend-architecture.md): translations, query ownership, public feature interfaces, the typed test transport
- [Persistence](core/persistence.md): the migration catalog, startup and development reconciliation, connections, repositories
- [Logging](core/logging.md): configuration, the JSON event contract, request completion, Git command logging
- [Streams and cancellation](core/streams.md): the stream registry and what a cancellation confirms

**Desktop**

- [Desktop host](desktop/desktop-host.md): startup, the command layer, native commands, persistent paths, updates
- [Settings](desktop/settings.md): developer mode, log level, network proxy, worktree root

**Workspaces**

- [Tasks and worktrees](workspaces/tasks-and-worktrees.md): Workspaces, task creation, checkout resolution, durable Git cleanup
- [Workspace files](workspaces/workspace-files.md): browsing, search, file operations, watching, the viewer
- [Workspace review](workspaces/workspace-review.md): diffs, staging, commit and push, review refresh
- [Gitlancer](workspaces/gitlancer.md): the typed Git CLI runtime

**Agents**

- [Agent runtime](agents/agent-runtime.md): agents from plugins, supervision, sessions, flow control, limits, host previews
- [Session history](agents/session-history.md): recording, switching agents, degraded history, deletion
- [Session MCP](agents/session-mcp.md): selection, delivery, live refresh, Host MCP health
- [Agent definitions and Skills](agents/agents-and-skills.md): roles, the Skill catalog, packages and imports
- [Skill plugin configuration](agents/skill-configuration.md): public and secret settings, startup environment snapshots, restart behavior and security boundaries
- [Effects](agents/effects.md): Workspace Effect convergence and Skill materialization

**Plugins**

- [Plugin system](plugins/plugin-system.md): kinds, layout, lifecycle, protocol and SDK, configuration, Hooks
- [Agent configuration](plugins/agent-configuration.md): static declarations, model capabilities, release association and install checks
- [Marketplace](plugins/marketplace.md): sources, sync and the index, installation, packs
- [Surfaces](plugins/surfaces.md): workbench and webview plugin UIs
- [Plugin logging](plugins/plugin-logging.md): host-owned per-plugin logs
- [Workflow packages](plugins/workflow-packages.md): importing workflows from `.orax` packages

**Workflows**

- [Workflows](workflows/workflows.md): definitions, versions, graph storage, node kinds, excluded nodes
- [Workflow runs](workflows/workflow-runs.md): runs, Agent nodes, variables, failures, retries, resume and rollback
- [Loops and iterations](workflows/loops-and-iterations.md): container nodes, rounds and execution scopes

**Scheduled tasks**

- [Scheduled tasks](scheduled-tasks/scheduled-tasks.md): definitions, approval, runs and missed occurrences

**Controller**

- [Controller](controller/controller.md): the coordination store, preview capability selection, the executable, the cloud form and Workspace sandboxes
- [Agent relay](controller/agent-relay.md): sessions, Thread events, commands, Revision delivery and grants
- [Cloud contract](controller/cloud-contract.md): services, upload checksums, generating and upgrading the contract
- [Runtime control](controller/runtime-control.md): identities, authenticated channels, closure and force stop

**Node**

- [Node](node/node.md): deployment, execution and recovery, storage
- [Control session](node/control-session.md): transports, ownership, close codes, admission, liveness
- [Controller–Node protocol](node/protocol.md): framing, capabilities, message contracts, the codec
- [Repository clone](node/repository-clone.md): the Git Server snapshot input, the transport repository, failures and records
- [Worktree execution](node/worktree-execution.md)
- [Node agent sessions](node/agent-sessions.md): per-execution runtimes, workload users, live previews, assistant-text projections, the ledger, model access
- [Revisions](node/revisions.md): delivery over Git, the history upload and restoring a prior Revision
- [ora-git](node/ora-git.md): the Agent's Git command, per-session sockets, verbs, pushes and exit codes
- [Node plugins](node/plugins.md)

**Operations**

- [Container images](operations/container-images.md)
- [Sandbox routing](operations/sandbox-routing.md)
- [Sidecars](operations/sidecars.md)

**Development**

- [Local development](development/local-development.md)
- [Testing](development/testing.md)
- [Changing the code](development/changing-the-code.md)

## Maintenance rules

- `docs/` holds only documents maintained together with the code. Plans, progress, implementation logs,
  research notes, incident write-ups and acceptance records do not belong here; they live in the issue
  that tracks the work.
- Every document has an English version (`x.md`) and a Chinese version (`x.zh.md`), linked to each other
  below the title (`English | [中文](x.zh.md)` and `[English](x.md) | 中文`). Change both in the same
  change, and link English documents to English documents.
- Describe behavior the current code can verify. Link the governing specs ADRs instead of retelling them,
  and never describe a `proposed` decision, or an `approved` one that is not implemented, as implemented.
- Keep one topic in one place. Cross-module behavior goes here; a crate's internals go in its `README.md`.
- A behavior change updates the matching document in the same change. A new topic gets a document in the
  fitting directory and a line in this map.
