# Ready MCPs propagate to every Workspace

Every installed MCP Package is Default-enabled, but it enters the MCP Desired Set of all existing and future Agent Workspaces only while it is Ready. An MCP that Needs Configuration remains installed without affecting Agent conversations; becoming Ready publishes it automatically, while losing readiness removes its effective Desired entries until it becomes Ready again. This preserves the automatic propagation semantics used for installed Skills without treating installation alone as successful materialization.
