//! Interprets the model selector in an authoritative provider configuration report.
//!
//! Starting a session checks whether the requested value is offered; rebuilding reads the current
//! value. Both must identify the same selector without making their different value checks equal.

use agent_client_protocol_schema::v1::{
    SessionConfigId, SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigSelect, SessionConfigSelectOptions,
};

/// Preserves the provider's first Model category, falling back only to a single select option.
fn model_selector(
    options: &[SessionConfigOption],
) -> Option<(&SessionConfigId, &SessionConfigSelect)> {
    let option = options
        .iter()
        .find(|option| matches!(option.category, Some(SessionConfigOptionCategory::Model)))
        .or_else(|| {
            let mut selects = options
                .iter()
                .filter(|option| matches!(option.kind, SessionConfigKind::Select(_)));
            let only = selects.next()?;
            selects.next().is_none().then_some(only)
        })?;
    let SessionConfigKind::Select(select) = &option.kind else {
        return None;
    };
    Some((&option.id, select))
}

/// Finds a configuration target only when this handshake explicitly offers the requested model.
pub(super) fn model_config_id(
    options: &[SessionConfigOption],
    model: &str,
) -> Option<SessionConfigId> {
    let (id, select) = model_selector(options)?;
    let offered = match &select.options {
        SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .any(|option| option.value.0.as_ref() == model),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter())
            .any(|option| option.value.0.as_ref() == model),
        _ => false,
    };
    offered.then(|| id.clone())
}

/// Reads the last reported model without mistaking the available choices for the current value.
pub(super) fn current_model(options: &[SessionConfigOption]) -> Option<String> {
    let (_, select) = model_selector(options)?;
    Some(select.current_value.0.to_string())
}

#[cfg(test)]
mod tests {
    use super::{current_model, model_config_id};
    use agent_client_protocol_schema::v1::{
        SessionConfigBoolean, SessionConfigGroupId, SessionConfigId, SessionConfigKind,
        SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelect,
        SessionConfigSelectGroup, SessionConfigSelectOption, SessionConfigSelectOptions,
        SessionConfigValueId,
    };
    use pretty_assertions::assert_eq;

    /// Builds a report whose current value and available choices can intentionally disagree.
    fn select(id: &str, current: &str, offered: &[&str]) -> SessionConfigOption {
        SessionConfigOption::new(
            SessionConfigId::new(id.to_string()),
            id,
            SessionConfigKind::Select(SessionConfigSelect::new(
                SessionConfigValueId::new(current.to_string()),
                SessionConfigSelectOptions::Ungrouped(
                    offered
                        .iter()
                        .map(|value| {
                            SessionConfigSelectOption::new(value.to_string(), value.to_string())
                        })
                        .collect(),
                ),
            )),
        )
    }

    /// A toggle is not a model selector, even when the report has no select options.
    fn toggle() -> SessionConfigOption {
        SessionConfigOption::new(
            SessionConfigId::new("toggle"),
            "Toggle",
            SessionConfigKind::Boolean(SessionConfigBoolean::new(/*current_value*/ false)),
        )
    }

    /// Start and rebuild share category precedence, ambiguity and empty-report behavior.
    #[test]
    fn start_and_rebuild_resolve_the_same_selector() {
        let cases = [
            ("empty", vec![], None, None),
            ("toggle only", vec![toggle()], None, None),
            (
                "sole select",
                vec![select("uncategorized", "small", &["small"])],
                Some("uncategorized"),
                Some("small"),
            ),
            (
                "sole categorized select retains fallback",
                vec![
                    select("mode", "small", &["small"]).category(SessionConfigOptionCategory::Mode),
                ],
                Some("mode"),
                Some("small"),
            ),
            (
                "ambiguous",
                vec![
                    select("mode", "small", &["small"]),
                    select("other", "other", &["other"]),
                ],
                None,
                None,
            ),
            (
                "declared model wins",
                vec![
                    select("mode", "other", &["small"]),
                    select("models", "small", &["small"])
                        .category(SessionConfigOptionCategory::Model),
                ],
                Some("models"),
                Some("small"),
            ),
            (
                "first declared model wins",
                vec![
                    select("first", "small", &["small"])
                        .category(SessionConfigOptionCategory::Model),
                    select("second", "other", &["small"])
                        .category(SessionConfigOptionCategory::Model),
                ],
                Some("first"),
                Some("small"),
            ),
            (
                "invalid declared model does not fall back",
                vec![
                    toggle().category(SessionConfigOptionCategory::Model),
                    select("other", "small", &["small"]),
                ],
                None,
                None,
            ),
        ];
        for (name, options, id, current) in cases {
            assert_eq!(
                (model_config_id(&options, "small"), current_model(&options)),
                (id.map(SessionConfigId::new), current.map(str::to_string)),
                "{name}",
            );
        }
    }

    /// A provider's current value can outlive a catalog choice; only a new intent needs an offer.
    #[test]
    fn current_value_and_offered_intent_keep_their_distinct_meaning() {
        let options = [
            select("models", "retired", &["small"]).category(SessionConfigOptionCategory::Model)
        ];
        assert_eq!(
            (
                current_model(&options),
                model_config_id(&options, "retired"),
                model_config_id(&options, "small")
            ),
            (
                Some("retired".to_string()),
                None,
                Some(SessionConfigId::new("models"))
            ),
        );
    }

    /// Grouping available values does not change either caller's selector or current model.
    #[test]
    fn grouped_choices_offer_models_to_start_and_rebuild() {
        let options = [SessionConfigOption::new(
            SessionConfigId::new("models"),
            "Models",
            SessionConfigKind::Select(SessionConfigSelect::new(
                SessionConfigValueId::new("small"),
                SessionConfigSelectOptions::Grouped(vec![SessionConfigSelectGroup::new(
                    SessionConfigGroupId::new("provider"),
                    "Provider",
                    vec![SessionConfigSelectOption::new("small", "Small")],
                )]),
            )),
        )
        .category(SessionConfigOptionCategory::Model)];
        assert_eq!(
            (current_model(&options), model_config_id(&options, "small")),
            (
                Some("small".to_string()),
                Some(SessionConfigId::new("models"))
            ),
        );
    }
}
