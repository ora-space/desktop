/// Holds the stable environment contract used for automated Git invocations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitEnv {
    pub terminal_prompt: bool,
    pub lang: String,
    pub pager: String,
}

impl GitEnv {
    /// Returns conservative automation defaults so Git behaves predictably under an agent runtime.
    pub fn automation_defaults() -> Self {
        Self {
            terminal_prompt: false,
            lang: "C".to_string(),
            pager: "cat".to_string(),
        }
    }
}

impl Default for GitEnv {
    /// Uses automation-safe defaults because an AI-oriented runtime should be deterministic by default.
    fn default() -> Self {
        Self::automation_defaults()
    }
}
