//! Host-owned redaction registry: the bounded, per-generation set of values each plugin has been
//! delivered, published as immutable matchers (ADR D2/D3).
//!
//! Delivery sites register a value before handing it to the plugin; the pipeline redacts with the
//! matcher in force. Registration is append-only within a generation, bounded, and stops accepting
//! new values once the budget is spent or a matcher build fails, so the delivery never blocks.

use ora_domain::PluginId;
use ora_logging::ora_warn;
use ora_plugin_runtime::{
    MAX_REDACTION_BYTES, MAX_REDACTION_VALUES, MIN_REDACTION_BYTES, REDACTION_PLACEHOLDER,
    Redactions,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::watch;

/// The live registry of redaction values, keyed by the plugin that receives them.
#[derive(Clone, Default)]
pub struct RedactionRegistry {
    inner: Arc<Mutex<BTreeMap<PluginId, Entry>>>,
}

/// One plugin's current generation: the values accepted so far and the matcher publisher.
struct Entry {
    values: Vec<String>,
    bytes: usize,
    /// Whether a bound or build failure already stopped this generation from accepting more.
    stopped: bool,
    /// Whether the single per-generation host warning has already been logged.
    warned: bool,
    publisher: Option<watch::Sender<Redactions>>,
}

impl RedactionRegistry {
    /// Starts a fresh generation for `plugin_id`: the previous generation's values are dropped and
    /// a receiver for the new, empty matcher is returned. Older receivers keep their own matcher,
    /// so a previous generation's reader finishes with the values it was launched with (ADR D2).
    pub fn begin_generation(&self, plugin_id: &PluginId) -> watch::Receiver<Redactions> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let (sender, receiver) = watch::channel(Redactions::default());
        inner.insert(
            plugin_id.clone(),
            Entry {
                values: Vec::new(),
                bytes: 0,
                stopped: false,
                warned: false,
                publisher: Some(sender),
            },
        );
        receiver
    }

    /// Registers each eligible value before delivery, appending it to the generation's matcher.
    ///
    /// Returns how many values were newly accepted. Rejected values (too short, containing the
    /// placeholder, duplicates) are skipped without counting as a failure; a bound or build failure
    /// stops the generation and is logged once (ADR D2/D3).
    pub fn register(
        &self,
        plugin_id: &PluginId,
        values: impl IntoIterator<Item = String>,
    ) -> usize {
        let values: Vec<String> = values.into_iter().collect();
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = inner.entry(plugin_id.clone()).or_insert_with(|| Entry {
            values: Vec::new(),
            bytes: 0,
            stopped: false,
            warned: false,
            publisher: None,
        });
        if entry.stopped {
            return 0;
        }
        let mut accepted = Vec::new();
        let mut bytes = entry.bytes;
        for value in values {
            if !eligible(&value) || entry.values.contains(&value) || accepted.contains(&value) {
                continue;
            }
            if entry.values.len() + accepted.len() >= MAX_REDACTION_VALUES
                || bytes + value.len() > MAX_REDACTION_BYTES
            {
                stop(entry, plugin_id);
                break;
            }
            bytes += value.len();
            accepted.push(value);
        }
        if accepted.is_empty() {
            return 0;
        }
        let mut candidate = entry.values.clone();
        candidate.extend(accepted.iter().cloned());
        match Redactions::build(candidate.iter().cloned()) {
            Ok(matcher) => {
                entry.values = candidate;
                entry.bytes = bytes;
                if let Some(publisher) = &entry.publisher {
                    publisher.send_replace(matcher);
                }
                accepted.len()
            }
            Err(_) => {
                stop(entry, plugin_id);
                0
            }
        }
    }
}

/// Whether a value may be registered: `String`, at least the minimum length, and not itself a
/// substring of the placeholder (ADR D3).
fn eligible(value: &str) -> bool {
    value.len() >= MIN_REDACTION_BYTES && !REDACTION_PLACEHOLDER.contains(value)
}

/// Stops a generation from accepting more values and logs the single host warning (ADR D3).
fn stop(entry: &mut Entry, plugin_id: &PluginId) {
    entry.stopped = true;
    if entry.warned {
        return;
    }
    entry.warned = true;
    ora_warn!(
        plugin_id = %plugin_id,
        "plugin log redaction registry is full or failed to build; later values are not redacted for this generation",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_domain::PluginId;

    fn id() -> PluginId {
        PluginId::parse("official/example").expect("plugin id")
    }

    fn eligible_values() -> Vec<String> {
        vec![
            "secret_value_one".to_string(),
            "secret_value_two".to_string(),
        ]
    }

    /// Accepted values publish a matcher; rejected values are skipped without stopping the generation.
    #[test]
    fn registers_only_eligible_values() {
        let registry = RedactionRegistry::default();
        let receiver = registry.begin_generation(&id());
        let accepted = registry.register(
            &id(),
            vec![
                "secret_value_one".to_string(),
                "short".to_string(),
                "REDACTED".to_string(),
                "secret_value_one".to_string(),
            ],
        );
        // Only the first value is eligible: `short` is too short, `REDACTED` is a placeholder
        // substring, and the second `secret_value_one` is a duplicate.
        assert_eq!(accepted, 1);
        assert!(!receiver.borrow().is_empty());
    }

    /// Once over budget, later values are rejected and the generation is stopped.
    #[test]
    fn stops_registering_once_over_budget() {
        let registry = RedactionRegistry::default();
        let _receiver = registry.begin_generation(&id());
        // One huge value over the byte budget stops the generation immediately.
        let huge = "x".repeat(MAX_REDACTION_BYTES + 1);
        let accepted = registry.register(&id(), std::iter::once(huge));
        assert_eq!(accepted, 0);
        assert_eq!(
            registry.register(&id(), eligible_values()),
            0,
            "a stopped generation accepts nothing",
        );
    }

    /// A fresh generation starts empty; the previous generation's values do not leak forward.
    #[test]
    fn begin_generation_resets_values() {
        let registry = RedactionRegistry::default();
        let _first = registry.begin_generation(&id());
        assert_eq!(registry.register(&id(), eligible_values()), 2);
        let second = registry.begin_generation(&id());
        assert!(second.borrow().is_empty());
    }
}
