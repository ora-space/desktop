# Ora Plugin-to-Agent Configuration

This context describes how Ora makes an installed, configuration-only MCP capability usable by an Agent while keeping package meaning, Agent-native representation, and Ora ownership distinct.

## Language

**Agent Plugin**:
An installed Ora plugin that adapts one Agent provider to Ora's Agent protocol.
_Avoid_: Agent package, Agent implementation

**MCP Plugin**:
An installed, processless Ora plugin that describes one MCP Server and its user-configurable inputs. It does not itself run the server or an Agent.
_Avoid_: MCP Agent plugin, MCP runtime plugin

**Installed MCP**:
An MCP Plugin whose package and static declaration passed installation validation. Installation does not imply that required inputs exist or that an Agent can use it.
_Avoid_: Enabled MCP, Ready MCP

**Configuration-Ready MCP**:
An Installed MCP whose required Settings and current installed version can produce a Resolved MCP. This is configuration completeness, not proof that an Agent has applied it.
_Avoid_: Ready, Installed MCP, Reachable MCP

**Resolved MCP**:
The Agent-independent use-time description produced from a validated MCP declaration and one exact plugin-configuration revision. It contains the information an Agent adapter needs without exposing Ora's package or Settings storage.
_Avoid_: Raw MCP config, Agent MCP config

**Effective MCP Set**:
The complete set of Configuration-Ready MCPs that currently applies under Automatic MCP Enablement.
_Avoid_: Installed MCP list, Marketplace MCP list

**MCP-Capable Agent Plugin**:
An Agent Plugin that declares an MCP materialization surface at runtime and can translate a complete Effective MCP Set into its own native representation.
_Avoid_: MCP Plugin, Agent compatibility by name

**Agent MCP Surface**:
The independently converged relationship between one Workspace and one MCP-Capable Agent Plugin. Its readiness does not imply atomic readiness with Skill surfaces.
_Avoid_: Agent Target, Workspace-root filesystem surface

**MCP Materialization**:
The conversion of a complete Effective MCP Set into an Agent's native configuration under provable Ora ownership.
_Avoid_: MCP installation, MCP copy

**Ora-Owned Agent Configuration**:
An Agent-native configuration asset that Ora created as one ownership unit and may therefore replace or remove after revalidating both durable and colocated ownership evidence. A pre-existing user asset is never Ora-owned merely because its name or contents match.
_Avoid_: User config, Adopted config, Managed entry

**MCP Environment Binding**:
A stable environment reference used in Agent-native configuration so the corresponding Setting value need not be persisted in a Workspace.
_Avoid_: SecretRef, Workspace credential

**Automatic MCP Enablement**:
The first-release policy that applies every Configuration-Ready MCP to every supported local Workspace without per-Workspace selection or exclusion.
_Avoid_: MCP installation, Workspace opt-in

**MCP Application State**:
The user-visible state of an MCP after considering configuration completeness, compatible Agent availability, surface convergence, and Agent activation: `NeedsConfiguration`, `WaitingForAgent`, `Applying`, `Ready`, or `Failed`.
_Avoid_: Settings completeness, Install state

**Agent Activation**:
The runtime transition after materialization that makes the shared Agent process consume the newly applied native configuration.
_Avoid_: File write, Session creation

**MCP Ownership Conflict**:
A fail-closed condition in which Ora cannot prove that an observed Agent configuration is the same ownership unit it previously created. Ora does not adopt or overwrite the observed asset.
_Avoid_: Recoverable drift, Name collision

**Functional MCP Loop**:
The user-visible state in which an MCP Plugin has been installed and configured, materialized and activated for an Agent, and successfully invoked as a tool during an Agent conversation.
_Avoid_: Configured MCP, Installed MCP, Successful file write
