use super::variable_pool::{VariableSelector, WorkflowVariablePool, WorkflowVariablePoolError};
use thiserror::Error;

/// Failure raised while rendering a `{{#node.variable.path#}}` template against the variable pool.
///
/// The renderer resolves every placeholder eagerly so a node that references a missing or unset
/// variable fails before it starts, instead of handing the agent a partially-expanded prompt.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VariableTemplateError {
    #[error("template has an unterminated variable reference")]
    Unterminated,
    #[error("template selector {selector} is not a node id and root variable")]
    InvalidSelector { selector: String },
    #[error("template references an undeclared variable: {selector}")]
    Undeclared { selector: String },
    #[error("template variable {selector} has no object path segment {segment}")]
    InvalidPath { selector: String, segment: String },
    #[error("template variable {selector} is not an object, so its path cannot be resolved")]
    PathNotObject { selector: String },
    #[error("template variable {selector} has not been assigned yet")]
    UnsetVariable { selector: String },
}

/// Renders Dify-style `{{#node.variable.path#}}` placeholders from the run's variable pool.
///
/// Strings render verbatim; every other value renders as JSON. An unknown, unassigned, or
/// mistyped selector is an error, never a silent blank.
pub fn render_variable_template(
    template: &str,
    pool: &WorkflowVariablePool,
) -> Result<String, VariableTemplateError> {
    let mut rendered = String::new();
    let mut rest = template;
    while let Some(open) = rest.find("{{#") {
        rendered.push_str(&rest[..open]);
        let after_open = &rest[open + 3..];
        let close = after_open
            .find("#}}")
            .ok_or(VariableTemplateError::Unterminated)?;
        let selector_text = &after_open[..close];
        let parts: Vec<String> = selector_text.split('.').map(str::to_string).collect();
        let selector = VariableSelector::try_from_parts(&parts).ok_or_else(|| {
            VariableTemplateError::InvalidSelector {
                selector: selector_text.to_string(),
            }
        })?;
        let value = pool.resolve(&selector).map_err(|error| match error {
            WorkflowVariablePoolError::Undeclared { selector } => {
                VariableTemplateError::Undeclared { selector }
            }
            WorkflowVariablePoolError::PathMissing { selector, segment } => {
                VariableTemplateError::InvalidPath { selector, segment }
            }
            WorkflowVariablePoolError::PathNotObject { selector, path: _ } => {
                VariableTemplateError::PathNotObject { selector }
            }
            WorkflowVariablePoolError::InvalidWriter { .. } => {
                unreachable!("resolving never validates a writer")
            }
            WorkflowVariablePoolError::TypeMismatch { .. } => {
                unreachable!("resolving never validates an assigned value's declared type")
            }
            WorkflowVariablePoolError::LengthExceeded { .. } => {
                unreachable!("resolving never validates an assigned value's length")
            }
        })?;
        let Some(value) = value else {
            return Err(VariableTemplateError::UnsetVariable {
                selector: selector.qualified(),
            });
        };
        rendered.push_str(&render_value(value));
        rest = &after_open[close + 3..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}

/// Renders one resolved value as template text: strings verbatim, everything else as JSON.
fn render_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn pool_with_review() -> WorkflowVariablePool {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("review.text", "string", "review");
        pool.declare("review.structured_output", "object", "review");
        pool.declare("start.topic", "string", "start");
        pool.declare("start.optional", "string", "start");
        pool.set("review.text", "review", json!("审查完成"))
            .unwrap();
        pool.set(
            "review.structured_output",
            "review",
            json!({ "approved": true, "score": 92 }),
        )
        .unwrap();
        pool.set("start.topic", "start", json!("检查支付模块"))
            .unwrap();
        pool
    }

    /// Resolves every placeholder, rendering strings verbatim and objects as JSON.
    #[test]
    fn renders_string_and_nested_object_placeholders() {
        let pool = pool_with_review();
        let rendered = render_variable_template(
            "批准状态：{{#review.structured_output.approved#}}，主题：{{#start.topic#}}",
            &pool,
        )
        .unwrap();
        assert_eq!(rendered, "批准状态：true，主题：检查支付模块");
    }

    /// A string selector renders verbatim without quotes.
    #[test]
    fn string_placeholder_renders_without_quotes() {
        let pool = pool_with_review();
        let rendered = render_variable_template("结果：{{#review.text#}}", &pool).unwrap();
        assert_eq!(rendered, "结果：审查完成");
    }

    /// Unset and undeclared selectors fail rather than rendering blank.
    #[test]
    fn rejects_unset_and_undeclared_selectors() {
        let pool = pool_with_review();
        assert_eq!(
            render_variable_template("{{#start.optional#}}", &pool).unwrap_err(),
            VariableTemplateError::UnsetVariable {
                selector: "start.optional".to_string()
            }
        );
        assert_eq!(
            render_variable_template("{{#other-node.text#}}", &pool).unwrap_err(),
            VariableTemplateError::Undeclared {
                selector: "other-node.text".to_string()
            }
        );
    }

    /// Plain text with no placeholders passes through unchanged.
    #[test]
    fn plain_text_without_placeholders_is_unchanged() {
        let pool = pool_with_review();
        assert_eq!(
            render_variable_template("请根据审查结果继续工作。", &pool).unwrap(),
            "请根据审查结果继续工作。"
        );
    }

    /// An unterminated or malformed placeholder is a hard error.
    #[test]
    fn rejects_unterminated_and_malformed_placeholders() {
        let pool = pool_with_review();
        assert_eq!(
            render_variable_template("{{#review.text", &pool).unwrap_err(),
            VariableTemplateError::Unterminated
        );
        assert_eq!(
            render_variable_template("{{#only-node#}}", &pool).unwrap_err(),
            VariableTemplateError::InvalidSelector {
                selector: "only-node".to_string()
            }
        );
    }
}
