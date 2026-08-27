# Plugin Packages

This context defines the language used for installable Ora plugin packages and the contribution role each package presents to the host.

## Language

**Hook Plugin**:
An Ora plugin package that contributes exactly one managed executable and one agent-neutral Hook Protocol descriptor. It has no plugin process and does not own agent-specific configuration.
_Avoid_: RTK plugin, hook capability

**Agent Plugin**:
An Ora plugin package that provides an agent and owns the agent-specific work needed to configure installed Hook Plugins for that agent.
_Avoid_: agent adapter, hook configurator

**Installed Hook Plugin**:
A globally available Hook Plugin that Agent Plugins may consume. Installation does not write eligibility state; every installed valid Hook is available. Command-alias uniqueness is deferred to a future consumer.
_Avoid_: enabled hook, project hook, configured hook

**Hook Protocol**:
A versioned, strongly typed contract that identifies how an Agent Plugin integrates a Hook Plugin. RTK uses an RTK-specific protocol within this common contract instead of RTK-specific manifest fields.
_Avoid_: arbitrary hook metadata, RTK manifest

**Hook Configuration**:
The immutable `assets/config.json` declaration that contains exactly one Hook Protocol descriptor and may also declare plugin-global Settings. Stored Setting Values and agent-specific configuration are not part of it.
_Avoid_: Hook manifest, Agent configuration, arbitrary metadata

**RTK Rewrite Protocol**:
The `rtk-rewrite-v1` Hook Protocol, which invokes `rtk rewrite` and preserves the command decision represented by RTK's exit status and output. Its descriptor reports the embedded RTK tool version independently from the Hook Plugin version.
_Avoid_: RTK CLI protocol, agent-specific RTK hook

**Hook Target**:
A target triple that identifies the operating system, processor architecture, and binary ABI of one installable Hook Plugin artifact.
_Avoid_: platform package, target plugin

**Universal Artifact**:
A plugin release artifact that can be installed on every host target supported by its Ora version requirement.
_Avoid_: generic package, targetless artifact

**Targeted Artifact**:
A plugin release artifact built for exactly one Hook Target and carrying that target identity inside the package.
_Avoid_: platform package, architecture build

**Runnable Hook Package**:
An installed Hook Plugin whose declared executable starts successfully on its Hook Target and satisfies its Hook Protocol contract. Runnability is established by isolated release and end-to-end tests, never by executing the payload during installation.
_Avoid_: valid archive, enabled hook

**Hook Command**:
The validated bare command name through which an Agent Plugin may expose a Hook Plugin executable to its agent.
_Avoid_: executable path, PATH entry

**Unsupported Hook**:
An Installed Hook Plugin whose Hook Protocol an Agent Plugin does not implement. It remains available globally but is not configured for that agent.
_Avoid_: broken hook, disabled hook
