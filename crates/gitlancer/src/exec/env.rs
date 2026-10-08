use std::collections::BTreeMap;

/// Variable Git reads to learn how many command-scoped config entries the environment carries.
const GIT_CONFIG_COUNT: &str = "GIT_CONFIG_COUNT";

/// Holds the stable environment contract used for automated Git invocations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitEnv {
    pub terminal_prompt: bool,
    pub lang: String,
    pub pager: String,
    pub variables: BTreeMap<String, String>,
}

impl GitEnv {
    /// Returns conservative automation defaults so Git behaves predictably under an agent runtime.
    pub fn automation_defaults() -> Self {
        Self {
            terminal_prompt: false,
            lang: "C".to_string(),
            pager: "cat".to_string(),
            variables: BTreeMap::new(),
        }
    }

    /// Adds one command-scoped environment variable without weakening automation defaults.
    pub fn with_variable(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.variables.insert(name.into(), value.into());
        self
    }

    /// Adds one command-scoped Git config entry that outranks every config file.
    ///
    /// The entry travels through `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n` rather
    /// than `-c` so values such as credential-bearing proxy URLs stay out of the logged argv.
    pub fn with_config(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let index = self
            .variables
            .get(GIT_CONFIG_COUNT)
            .and_then(|count| count.parse::<usize>().ok())
            .unwrap_or(0);
        self.with_variable(format!("GIT_CONFIG_KEY_{index}"), key)
            .with_variable(format!("GIT_CONFIG_VALUE_{index}"), value)
            .with_variable(GIT_CONFIG_COUNT, (index + 1).to_string())
    }
}

impl Default for GitEnv {
    /// Uses automation-safe defaults because an AI-oriented runtime should be deterministic by default.
    fn default() -> Self {
        Self::automation_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Verifies successive config entries take consecutive indices alongside plain variables.
    #[test]
    fn config_entries_are_numbered_in_insertion_order() {
        let env = GitEnv::default()
            .with_variable("GIT_TERMINAL_PROMPT", "0")
            .with_config("http.proxy", "")
            .with_config("http.https://example.com.proxy", "http://proxy:8080");

        assert_eq!(
            env.variables,
            BTreeMap::from(
                [
                    ("GIT_TERMINAL_PROMPT", "0"),
                    ("GIT_CONFIG_COUNT", "2"),
                    ("GIT_CONFIG_KEY_0", "http.proxy"),
                    ("GIT_CONFIG_VALUE_0", ""),
                    ("GIT_CONFIG_KEY_1", "http.https://example.com.proxy"),
                    ("GIT_CONFIG_VALUE_1", "http://proxy:8080"),
                ]
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
            )
        );
    }
}
