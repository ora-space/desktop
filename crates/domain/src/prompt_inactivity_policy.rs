use serde::{Deserialize, Serialize};

/// Controls whether silence alone may cancel and retry one prompt turn.
///
/// Hosts choose this per turn so a workflow's external work can keep waiting without changing
/// ordinary conversations, cancellation, or provider failure handling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptInactivityPolicy {
    /// Cancel and retry when the runtime's meaningful-activity window expires.
    #[default]
    Timeout,
    /// Wait for the default workflow silence window without resending the prompt.
    Wait,
    /// Send once without imposing a silence deadline.
    Unbounded,
}

impl PromptInactivityPolicy {
    /// Returns a deadline budget for single-send turns. Ordinary prompts use a retry schedule.
    pub fn wait_limit(self) -> Option<std::time::Duration> {
        (self == Self::Wait).then_some(std::time::Duration::from_secs(30 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::PromptInactivityPolicy;
    use pretty_assertions::assert_eq;

    /// The authoring values stay stable and omitted policy retains the existing runtime behavior.
    #[test]
    fn wire_values_and_default_are_explicit() {
        assert_eq!(
            PromptInactivityPolicy::default(),
            PromptInactivityPolicy::Timeout
        );
        for (policy, wire) in [
            (PromptInactivityPolicy::Timeout, "\"timeout\""),
            (PromptInactivityPolicy::Wait, "\"wait\""),
            (PromptInactivityPolicy::Unbounded, "\"unbounded\""),
        ] {
            assert_eq!(serde_json::to_string(&policy).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<PromptInactivityPolicy>(wire).unwrap(),
                policy
            );
        }
        assert!(serde_json::from_str::<PromptInactivityPolicy>("\"unknown\"").is_err());
    }
}
