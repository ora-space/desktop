//! Maps what an agent offers outside a session onto the public contract.

use ora_contracts::SHOWN_AGENT_COMMAND_LIMIT;
use ora_domain::AgentRef;
use ora_logging::ora_warn;
use ora_plugin_protocol::AgentDiscoverResult;

/// Whether a client shows only part of one report.
///
/// Clients apply the limit, to discovered and session-reported commands alike; what is delivered
/// is never cut. It is checked here only so that hiding some leaves a trace in Desktop's log.
pub(crate) fn exceeds_shown_commands(count: usize) -> bool {
    count > SHOWN_AGENT_COMMAND_LIMIT
}

/// Records one report beyond the limit, without recording what it says.
pub(crate) fn warn_when_commands_exceed_limit(agent_ref: &AgentRef, count: usize) {
    if exceeds_shown_commands(count) {
        ora_warn!(
            agent = %agent_ref,
            count,
            limit = SHOWN_AGENT_COMMAND_LIMIT,
            "agent reported more commands than the limit shown",
        );
    }
}

/// Builds the public answer to one discovery.
pub(crate) fn response(
    agent_ref: &AgentRef,
    discovered: AgentDiscoverResult,
) -> ora_contracts::DiscoverAgentResponse {
    let commands = discovered.commands.map(|commands| {
        warn_when_commands_exceed_limit(agent_ref, commands.len());
        commands
            .into_iter()
            .map(|command| ora_contracts::AgentCommand {
                name: command.name,
                description: command.description,
                input_hint: command.input_hint,
            })
            .collect()
    });
    ora_contracts::DiscoverAgentResponse {
        models: discovered
            .models
            .into_iter()
            .map(|model| ora_contracts::AgentModel {
                id: model.id,
                display_name: model.display_name,
                default: model.default,
            })
            .collect(),
        commands,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{response, warn_when_commands_exceed_limit};
    use ora_domain::AgentRef;
    use ora_plugin_protocol::{AgentCommand, AgentDiscoverResult, AgentModel};
    use pretty_assertions::assert_eq;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::Context;

    /// Collects every recorded event as its fields.
    #[derive(Clone, Default)]
    pub(crate) struct Recorded(Arc<Mutex<Vec<Vec<(String, String)>>>>);

    impl<S: tracing::Subscriber> Layer<S> for Recorded {
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
            let mut fields = Fields(Vec::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
    }

    struct Fields(Vec<(String, String)>);

    impl Visit for Fields {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }

    /// Returns the size of every report recorded as beyond the limit, in order.
    pub(crate) fn warnings(recorded: &Recorded) -> Vec<String> {
        let events = recorded.0.lock().unwrap();
        events
            .iter()
            .filter(|fields| {
                fields.iter().any(|(name, value)| {
                    name == "message"
                        && value == "agent reported more commands than the limit shown"
                })
            })
            .flat_map(|fields| fields.iter().filter(|(name, _)| name == "count"))
            .map(|(_, count)| count.clone())
            .collect()
    }

    /// Names the agent the cases discover.
    fn agent() -> AgentRef {
        AgentRef::parse("official/test.agent").unwrap()
    }

    /// Unknown commands and an empty list reach the client as different facts, unchanged in order.
    #[test]
    fn the_public_answer_keeps_unknown_apart_from_empty() {
        let command = |name: &str| AgentCommand {
            name: name.to_string(),
            description: "Runs it".to_string(),
            input_hint: Some("scope".to_string()),
        };
        let public = |name: &str| ora_contracts::AgentCommand {
            name: name.to_string(),
            description: "Runs it".to_string(),
            input_hint: Some("scope".to_string()),
        };
        for (commands, expected) in [
            (None, None),
            (Some(Vec::new()), Some(Vec::new())),
            (
                Some(vec![command("b"), command("a")]),
                Some(vec![public("b"), public("a")]),
            ),
        ] {
            let discovered = AgentDiscoverResult {
                models: vec![AgentModel {
                    id: "m1".to_string(),
                    display_name: "Model 1".to_string(),
                    default: true,
                }],
                commands,
            };
            assert_eq!(
                response(&agent(), discovered),
                ora_contracts::DiscoverAgentResponse {
                    models: vec![ora_contracts::AgentModel {
                        id: "m1".to_string(),
                        display_name: "Model 1".to_string(),
                        default: true,
                    }],
                    commands: expected,
                }
            );
        }
    }

    /// Only a report beyond the limit is recorded, as its size and agent and nothing it says.
    #[test]
    fn a_report_beyond_the_limit_warns_with_counts_only() {
        let recorded = Recorded::default();
        ora_logging::with_recorded_trace_logging(recorded.clone(), || {
            warn_when_commands_exceed_limit(&agent(), 500);
            warn_when_commands_exceed_limit(&agent(), 501);
        });

        let events = recorded.0.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![vec![
                (
                    "message".to_string(),
                    "agent reported more commands than the limit shown".to_string()
                ),
                (
                    "method".to_string(),
                    "\"warn_when_commands_exceed_limit\"".to_string()
                ),
                ("agent".to_string(), "official/test.agent".to_string()),
                ("count".to_string(), "501".to_string()),
                ("limit".to_string(), "500".to_string()),
            ]]
        );
    }

    /// A discovery beyond the limit is recorded too, and still answers with every command.
    #[test]
    fn a_discovery_beyond_the_limit_is_recorded_and_not_cut() {
        let recorded = Recorded::default();
        let commands = (0..501)
            .map(|index| AgentCommand {
                name: format!("c{index}"),
                description: String::new(),
                input_hint: None,
            })
            .collect();
        let answer = ora_logging::with_recorded_trace_logging(recorded.clone(), || {
            let discovered = AgentDiscoverResult {
                models: Vec::new(),
                commands: Some(commands),
            };
            response(&agent(), discovered)
        });

        assert_eq!(answer.commands.map(|commands| commands.len()), Some(501));
        assert_eq!(warnings(&recorded), vec!["501"]);
    }
}
