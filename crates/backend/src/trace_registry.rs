//! Registers each agent's trace declaration and resolves it for one session.
//!
//! A trace declaration arrives either from a plugin manifest (`[agent.trace]`) at
//! install/upgrade time or from the built-in default table that serves the bundled agents until
//! they are plugin-ized. The registry itself is data plumbing only: substitution happens in
//! `ora_plugin_manifest` and the resolved locator is consumed by the read service — nothing here
//! touches the filesystem.

use ora_domain::AgentRef;
use ora_plugin_manifest::{PluginAgentTrace, TraceLocator, TraceResolveContext};
use std::collections::HashMap;
use std::sync::RwLock;

/// One session's resolved trace locator: `format` is a passthrough identifier the dashboard
/// selects its parser with, and `locator` tells the read service where the bytes live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTrace {
    pub format: String,
    pub locator: TraceLocator,
}

/// The built-in default table: one entry per bundled CLI, mirroring the legacy dashboard
/// resolver until the CLIs are plugin-ized (their plugin manifests will then carry the same
/// declarations and these entries become dead data).
///
/// `data_dir`/`home` placeholders are left for `TraceService` to substitute per session.
pub fn builtin_defaults() -> Vec<(AgentRef, PluginAgentTrace)> {
    let mut defaults = Vec::new();
    // Claude-Code-compatible forks all emit transcript JSONL under a projects root that is
    // fork-specific; the search form matches the session file by id across project directories.
    for (agent_ref, root) in [
        ("ora-space.claude", "{home}/.claude/projects"),
        ("ora-space.codex", "{home}/.claude/projects"),
        ("ora-space.codeagentcli", "{home}/.cac/projects"),
    ] {
        if let (Ok(agent_ref), Ok(declaration)) = (
            AgentRef::parse(agent_ref),
            PluginAgentTrace::search("claude_code", root, "**/{agent_session_id}.jsonl"),
        ) {
            defaults.push((agent_ref, declaration));
        }
    }
    // opencode and its Nga variant write one NDJSON per session through the deployed collector.
    for agent_ref in ["ora-space.opencode", "ora-space.nga"] {
        if let (Ok(agent_ref), Ok(declaration)) = (
            AgentRef::parse(agent_ref),
            PluginAgentTrace::file(
                "opencode",
                "{data_dir}/opencode/trace/{agent_session_id}.ndjson",
            ),
        ) {
            defaults.push((agent_ref, declaration));
        }
    }
    defaults
}

/// Maps one agent to its installed trace declaration.
///
/// Reads are cheap (`RwLock` + clone of an immutable declaration) so the per-read resolve path
/// never blocks on mutation; register/unregister happen at plugin install/upgrade/uninstall time.
pub struct TraceRegistry {
    entries: RwLock<HashMap<AgentRef, PluginAgentTrace>>,
}

impl TraceRegistry {
    /// Builds a registry pre-populated with the host's built-in default table.
    pub fn new(defaults: impl IntoIterator<Item = (AgentRef, PluginAgentTrace)>) -> Self {
        Self {
            entries: RwLock::new(defaults.into_iter().collect()),
        }
    }

    /// Registers or replaces one plugin agent's declaration.
    pub fn register_plugin(&self, agent_ref: AgentRef, declaration: PluginAgentTrace) {
        self.entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(agent_ref, declaration);
    }

    /// Removes one plugin agent's declaration, leaving built-in entries untouched.
    pub fn unregister_plugin(&self, agent_ref: &AgentRef) {
        self.entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(agent_ref);
    }

    /// Returns the raw declaration for one agent, for listing scans that need the unsubstituted
    /// template.
    pub fn declaration(&self, agent_ref: &AgentRef) -> Option<PluginAgentTrace> {
        self.entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(agent_ref)
            .cloned()
    }

    /// Resolves one session's trace locator through the registered declaration, or `None` when
    /// the agent is not installed or declares no trace.
    pub fn resolve(
        &self,
        agent_ref: &AgentRef,
        context: &TraceResolveContext<'_>,
    ) -> Option<ResolvedTrace> {
        let entries = self
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let declaration = entries.get(agent_ref)?;
        let locator = declaration.resolve(context).ok()?;
        Some(ResolvedTrace {
            format: declaration.format().to_owned(),
            locator,
        })
    }

    /// Returns every agent ref with a trace declaration, for `trace_list` scanning.
    pub fn agents(&self) -> Vec<AgentRef> {
        let mut agents: Vec<AgentRef> = self
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        agents.sort();
        agents
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Extracts an expected successful result without using `expect` in tests.
    fn must<T, E: std::fmt::Debug>(result: Result<T, E>, label: &str) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("expected {label} to succeed, got {error:?}"),
        }
    }

    /// Extracts an expected resolved value without using `expect` in tests.
    fn must_some<T>(value: Option<T>, label: &str) -> T {
        match value {
            Some(value) => value,
            None => panic!("expected {label} to resolve"),
        }
    }

    /// Builds the claude-style search declaration used by several tests.
    fn claude_declaration() -> PluginAgentTrace {
        must(
            PluginAgentTrace::search(
                "claude_code",
                "{home}/.claude/projects",
                "**/{agent_session_id}.jsonl",
            ),
            "search declaration",
        )
    }

    /// Builds the opencode-style file declaration used by several tests.
    fn opencode_declaration() -> PluginAgentTrace {
        must(
            PluginAgentTrace::file(
                "opencode",
                "{data_dir}/opencode/trace/{agent_session_id}.ndjson",
            ),
            "file declaration",
        )
    }

    fn context(session_id: &str) -> TraceResolveContext<'_> {
        TraceResolveContext {
            home: Path::new("/home/user"),
            data_dir: Path::new("/home/user/.local/share"),
            agent_session_id: session_id,
        }
    }

    /// A registered declaration resolves; an unknown agent and an unsafe session id do not.
    #[test]
    fn resolves_registered_declarations_only() {
        let registry = TraceRegistry::new(Vec::new());
        let claude = must(AgentRef::parse("ora-space.claude"), "agent ref");
        registry.register_plugin(claude.clone(), claude_declaration());

        let resolved = must_some(
            registry.resolve(&claude, &context("abc-123")),
            "registered agent",
        );
        assert_eq!(resolved.format, "claude_code");
        assert_eq!(
            resolved.locator,
            TraceLocator::Search {
                root: "/home/user/.claude/projects".into(),
                pattern: "**/abc-123.jsonl".to_owned(),
            }
        );

        let unknown = must(AgentRef::parse("ora-space.missing"), "agent ref");
        assert!(registry.resolve(&unknown, &context("abc-123")).is_none());

        // An unsafe session id never survives resolution, even for a registered agent.
        assert!(registry.resolve(&claude, &context("../etc")).is_none());
    }

    /// Registration replaces, unregistration removes, and `agents` stays sorted.
    #[test]
    fn register_replace_and_unregister() {
        let registry = TraceRegistry::new(Vec::new());
        let claude = must(AgentRef::parse("ora-space.claude"), "agent ref");
        registry.register_plugin(claude.clone(), claude_declaration());
        // An upgrade replaces the declaration for the same agent.
        registry.register_plugin(claude.clone(), opencode_declaration());

        let resolved = must_some(
            registry.resolve(&claude, &context("ses_1")),
            "replaced declaration",
        );
        assert_eq!(resolved.format, "opencode");

        registry.unregister_plugin(&claude);
        assert!(registry.resolve(&claude, &context("ses_1")).is_none());
        assert!(registry.agents().is_empty());
    }

    /// The default table is plain data: entries resolve before any plugin registers.
    #[test]
    fn builtin_defaults_are_plain_data() {
        let claude = must(AgentRef::parse("ora-space.claude"), "agent ref");
        let registry = TraceRegistry::new([(claude.clone(), claude_declaration())]);

        let resolved = must_some(
            registry.resolve(&claude, &context("abc-123")),
            "built-in entry",
        );
        assert_eq!(resolved.format, "claude_code");
        assert_eq!(registry.agents(), vec![claude]);
    }
}
