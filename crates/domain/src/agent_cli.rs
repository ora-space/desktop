use crate::AgentRef;

/// Names one agent CLI Ora launches and supervises itself.
///
/// This is a launch descriptor, not an identity: it says which executable to run and with which
/// arguments. The identity a session is bound to is an [`AgentRef`], which every built-in CLI
/// supplies through [`AgentCli::agent_ref`] so that built-in and plugin-provided agents are
/// indistinguishable everywhere above the launch step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentCli {
    OpenCode,
    Nga,
    CodeAgentCli,
    Claude,
    Codex,
}

impl AgentCli {
    pub const ALL: [Self; 5] = [
        Self::OpenCode,
        Self::Nga,
        Self::CodeAgentCli,
        Self::Claude,
        Self::Codex,
    ];

    /// Returns the namespaced identity this CLI provides, independent of enum declaration order.
    ///
    /// These are the same values persistence has always stored, so a built-in CLI keeps its
    /// identity when it is later repackaged as a plugin under the same id.
    pub fn agent_ref(self) -> AgentRef {
        let value = match self {
            Self::OpenCode => "ora-space.opencode",
            Self::Nga => "ora-space.nga",
            Self::CodeAgentCli => "ora-space.codeagentcli",
            Self::Claude => "ora-space.claude",
            Self::Codex => "ora-space.codex",
        };
        // The literals above are non-empty, so parsing cannot fail; constructing through `parse`
        // keeps one validation path rather than a second way to build the value object.
        AgentRef::parse(value).unwrap_or_else(|_| unreachable!("built-in agent ids are valid"))
    }

    /// Returns the executable basename used by the cross-platform PATH lookup.
    pub fn executable_name(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode",
            Self::Nga => "nga",
            Self::CodeAgentCli => "codeagentcli",
            Self::Claude => "claude-agent-acp",
            Self::Codex => "codex-acp",
        }
    }

    /// Returns the child process arguments used to start ACP over stdio.
    ///
    /// Ora's own CLIs (OpenCode, Nga, CodeAgentCli) expose ACP behind an `acp`
    /// subcommand. Claude Code and Codex are instead fronted by dedicated
    /// `claude-agent-acp`/`codex-acp` adapter binaries, which speak ACP directly
    /// with no subcommand.
    pub fn launch_arguments(self) -> &'static [&'static str] {
        match self {
            Self::OpenCode | Self::Nga | Self::CodeAgentCli => &["acp"],
            Self::Claude | Self::Codex => &[],
        }
    }
}
