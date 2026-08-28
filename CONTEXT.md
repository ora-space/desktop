# Ora Agent Effects

This context describes how Ora makes installed capabilities available to an Agent within a Workspace while preserving target-specific Agent behavior.

## Language

**Agent Workspace**:
The authoritative working directory shared by an Agent instance and its Sessions. It is either a project's main checkout or a Task's isolated worktree.
_Avoid_: Agent working directory, plugin data directory, Agent process directory

**MCP Package**:
An installed, immutable package that describes one MCP server and its configuration requirements without running an Ora plugin process.
_Avoid_: MCP process, MCP Agent plugin

**Resolved MCP**:
The target-independent, use-time MCP description produced for one exact installed package version and one Agent Workspace after required configuration has been resolved.
_Avoid_: Raw MCP config, Agent config

**Default-enabled MCP**:
An installed MCP Package that is intended to join every Agent Workspace automatically whenever it is Ready. The intent remains while the package Needs Configuration, so it can join again without another user selection.
_Avoid_: Selected MCP, active MCP

**Workspace MCP Desired Set**:
The complete set of Ready MCP Packages that Ora intends to make available in an Agent Workspace. Every Default-enabled MCP joins this set for all existing and future Agent Workspaces when it becomes Ready.
_Avoid_: Installed MCP catalog, Session MCP selection

**MCP Source**:
The stable Effect identity of one MCP Package across package and configuration versions.
_Avoid_: MCP package version, resolved configuration

**MCP Source Revision**:
An immutable snapshot binding an MCP Source to one exact package version, descriptor content, configuration-store revision, and resolved-state digest.
_Avoid_: MCP version, mutable settings

**Agent Target**:
The pairing of one Agent Workspace and one Agent Plugin that forms an independent configuration convergence boundary.
_Avoid_: Agent instance, Session

**Agent Target Ready Generation**:
The latest Workspace generation for which every required Skill and MCP operation has been processed and the Agent Target can accept new turns. Non-blocking target-specific issues may remain visible at this generation.
_Avoid_: Desired generation, applied file generation

**MCP Configuration Capability**:
A versioned, negotiated Agent Plugin declaration that it can materialize specified MCP transports for its target Agent and coordinate configuration changes with active Sessions. Its absence means the Agent does not support MCP materialization without invalidating the Agent's baseline conversation capability.
_Avoid_: MCP support flag, MCP filesystem surface

**Agent Capability Revision**:
The identity of an Agent Plugin version and its declared configuration capabilities whose change requires affected Agent Targets to be reconciled again.
_Avoid_: Plugin ID, process generation

**MCP Configuration Snapshot**:
The complete Resolved MCP set for one Agent Target and one Workspace generation. It is the idempotent input to MCP Materialization rather than an incremental add or remove event.
_Avoid_: MCP update event, config fragment

**MCP Materialization**:
The Agent Plugin-owned transformation of a complete Resolved MCP set into the target Agent's native configuration while preserving resources not managed by Ora.
_Avoid_: MCP installation, host rendering

**Managed MCP Entry**:
An MCP entry in a target Agent configuration whose ownership is proven by Ora's state for the current Agent Target.
_Avoid_: Installed MCP, matching MCP name

**Managed Agent Configuration Document**:
A target-native configuration document exclusively generated for one Agent Target and owned as a whole through a durable locator and fingerprint.
_Avoid_: User configuration file, config fragment

**MCP Materialization Conflict**:
A state in which the observed target configuration no longer matches the fingerprint previously applied by Ora, so ownership cannot justify another automatic write.
_Avoid_: Configuration update, overwrite permission

**Ready with Issues**:
An Agent Target state in which the complete MCP Configuration Snapshot has been processed and supported MCPs are available, while one or more target-specific, non-blocking MCP issues remain visible.
_Avoid_: Degraded, partially reconciled

**Needs Configuration**:
An installed MCP state in which required configuration is incomplete or unavailable, so the MCP remains Default-enabled but does not belong to any Workspace MCP Desired Set.
_Avoid_: Installation failure, Unsupported by Agent
