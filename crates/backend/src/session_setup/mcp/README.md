# Session MCP setup

This module resolves the MCP servers sent at ACP `session/new` and `session/load`, and tracks the
secret-free revision used by live sessions.

## Responsibilities

- Read installed MCP packages and their configuration completeness.
- Apply the Session-owned selection before resolving configuration values.
- Produce one ordered ACP server snapshot and a secret-free revision from the same inputs.
- Reject missing explicit selections, incomplete explicit configuration, unsupported transports,
  and revision races without returning a partial server list.

## Selection invariant

Installation adds an MCP plugin to the global catalog; it does not grant a workflow node access.
Ordinary Sessions use `Automatic` discovery. Workflow Sessions persist `Explicit`, including an
empty set, directly on the Session row. Recovery, Agent switching, and provider rebuilding read
that stored selection and never infer authority from workflow-run relationships.

## Non-responsibilities

This module does not install plugins, materialize Skills, expose Setting values in logs, or decide
which MCP bindings an editor should offer. The workflow graph parser validates canonical plugin
IDs when a run starts, while this runtime verifies current installation and configuration only
when a Session is set up.
