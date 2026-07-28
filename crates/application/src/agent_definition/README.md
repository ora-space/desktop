# Agent Definition Application Module

This module implements transport-independent CRUD use cases for configurable agent definitions.

## Responsibilities

- `CreateAgentDefinitionHandler` validates and constructs a domain `AgentDefinition` using injected identity and clock sources.
- Get and list handlers expose only visible repository records.
- Update preserves identity and audit history while applying mutable contract fields.
- Delete performs a soft delete and reports a stable not-found error when no visible record is affected.
- The mapper converts domain records into the shared `Agent` contract without leaking persistence details.

`AgentDefinitionRepository`, `AgentDefinitionIdGenerator`, and `Clock` form the infrastructure boundary. Repository failures are converted into `ApplicationError`; storage technology and transport error mapping remain outside this module.

This module manages configuration records only. It does not discover provider models, spawn agent CLIs, or own ACP session lifecycle; those operations belong to the backend agent runtime.

See the [ora-application overview](../../README.md).
