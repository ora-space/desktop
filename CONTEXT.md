# Ora Plugin Context

This context defines the language used to describe installed plugins and their host-managed configuration.

## Language

**Plugin Configuration**:
The complete set of user-provided values associated with one installed plugin.
_Avoid_: Ora preference, Agent option, plugin settings

**Installation Validity**:
Whether an installed package and its immutable declarations are acceptable to Ora.
_Avoid_: Enabled state, Configuration Readiness, runtime status

**Durable Eligibility**:
Whether the user has enabled an installed plugin to participate in Ora across application restarts.
_Avoid_: Installation Validity, Configuration Readiness, running state

**Setting Declaration**:
A plugin-authored description of one configurable value, identified by a stable Setting ID and distributed with the plugin.
_Avoid_: Stored setting, preference

**Stored Setting Value**:
A user-provided, non-secret value associated with a Setting Declaration independently of the installed plugin version.
_Avoid_: Setting Declaration, default

**Secret**:
A sensitive user-provided value whose plaintext must not be represented as a Stored Setting Value.
_Avoid_: Password field, secret setting value

**Configuration Completeness**:
Whether every required Setting Declaration has an available, type-correct effective value.
_Avoid_: Configuration Readiness, plugin health, runtime status

**Configuration Summary**:
The host-visible configuration result for an installed plugin: not declared, available with Configuration Completeness, or unavailable because configuration data cannot be read.
_Avoid_: Configuration Readiness, Plugin State

**Configuration Readiness**:
Whether a plugin capability can use a resolved Plugin Configuration in its runtime environment.
_Avoid_: Configuration Completeness, enabled state, runtime status

**Configuration Snapshot**:
The immutable, resolved Plugin Configuration bound to one Agent instance when that instance starts.
_Avoid_: Live configuration, Stored Setting Value

**Configuration Revision**:
The identity of one successfully saved version of a Plugin Configuration.
_Avoid_: Plugin version, schema version

**Needs Configuration**:
A user-facing indication that Configuration Completeness is incomplete because required values are absent or invalid.
_Avoid_: Broken plugin, invalid installation

**Invalid Declaration**:
An installation validity outcome indicating that a plugin's Setting Declarations cannot be accepted.
_Avoid_: Needs Configuration, invalid value

**Agent Plugin**:
An installed plugin that contributes an Agent capability to Ora.
_Avoid_: Agent process, Agent instance

**Managed Agent Process**:
An Agent process started and controlled by Ora on behalf of an Agent Plugin.
_Avoid_: Agent Plugin, plugin process

**Runtime Status**:
The current process-scoped lifecycle state of an enabled plugin.
_Avoid_: Durable Eligibility, Configuration Readiness, Installation Validity
