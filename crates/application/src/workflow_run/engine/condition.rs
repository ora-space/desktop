use super::variable_pool::{VariableSelector, WorkflowVariablePool, WorkflowVariablePoolError};
use serde::Deserialize;
use serde_json::Value;
use std::fmt;
use thiserror::Error;

/// The source handle an edge uses for the implicit ELSE path of a Condition node.
pub const ELSE_BRANCH_ID: &str = "else";

/// Wire shape of one `data.cases` entry on a Condition node.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireConditionCase {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub logic: Option<String>,
    #[serde(default)]
    pub conditions: Vec<WireConditionRule>,
}

/// Wire shape of one comparison inside a Condition case.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireConditionRule {
    #[serde(default)]
    pub variable_selector: Vec<String>,
    #[serde(default)]
    pub operator: Option<String>,
    #[serde(default)]
    pub value: Option<Value>,
}

/// The executable contract of a `condition` node.
///
/// Cases are evaluated in order; the first case whose rules hold wins, and the implicit trailing
/// `else` branch is selected when no case matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionConfig {
    pub cases: Vec<ConditionCase>,
}

/// One IF branch of a Condition node, joined internally by `logic`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionCase {
    pub id: String,
    pub logic: ConditionLogic,
    pub conditions: Vec<ConditionRule>,
}

/// How the rules inside one case combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionLogic {
    And,
    Or,
}

/// One comparison of a resolved variable against an expected value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionRule {
    pub variable_selector: VariableSelector,
    pub operator: ComparisonOperator,
    pub value: Option<Value>,
}

/// Failures raised while compiling or evaluating a Condition node.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConditionError {
    #[error("condition case is missing an id")]
    MissingCaseId,
    #[error("condition node has no executable cases")]
    MissingConfig,
    #[error("condition case has unknown logic {logic}")]
    UnknownLogic { logic: String },
    #[error("condition rule has unknown operator {operator}")]
    UnknownOperator { operator: String },
    #[error("condition rule selector {selector:?} is not a node id and root variable")]
    InvalidSelector { selector: Vec<String> },
    #[error("condition reads an undeclared variable: {selector}")]
    Undeclared { selector: String },
    #[error("condition variable {selector} has no object path segment {segment}")]
    InvalidPath { selector: String, segment: String },
    #[error(
        "operator {operator} cannot compare variable {selector} of type {actual} against {expected}"
    )]
    TypeMismatch {
        selector: String,
        operator: ComparisonOperator,
        actual: String,
        expected: String,
    },
    #[error("condition variable {selector} has not been assigned yet")]
    UnsetVariable { selector: String },
}

impl ConditionConfig {
    /// Compiles the wire `cases` array into an executable config.
    pub fn from_wire(cases: Vec<WireConditionCase>) -> Result<Self, ConditionError> {
        let mut compiled = Vec::new();
        for case in cases {
            let id = case.id.ok_or(ConditionError::MissingCaseId)?;
            let logic = match case.logic.as_deref() {
                Some("and") | None => ConditionLogic::And,
                Some("or") => ConditionLogic::Or,
                Some(other) => {
                    return Err(ConditionError::UnknownLogic {
                        logic: other.to_string(),
                    });
                }
            };
            let mut conditions = Vec::new();
            for rule in case.conditions {
                let operator = match rule.operator.as_deref() {
                    Some(value) => ComparisonOperator::parse(value).ok_or_else(|| {
                        ConditionError::UnknownOperator {
                            operator: value.to_string(),
                        }
                    })?,
                    None => {
                        return Err(ConditionError::UnknownOperator {
                            operator: String::new(),
                        });
                    }
                };
                let variable_selector = VariableSelector::try_from_parts(&rule.variable_selector)
                    .ok_or_else(|| ConditionError::InvalidSelector {
                    selector: rule.variable_selector.clone(),
                })?;
                conditions.push(ConditionRule {
                    variable_selector,
                    operator,
                    value: rule.value,
                });
            }
            compiled.push(ConditionCase {
                id,
                logic,
                conditions,
            });
        }
        Ok(Self { cases: compiled })
    }
}

/// Compares one declared variable against a literal using the configured operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOperator {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Empty,
    NotEmpty,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Is,
    IsNot,
    Exists,
    NotExists,
}

impl ComparisonOperator {
    /// Maps the wire operator string to a typed operator.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "equals" => Self::Equals,
            "not_equals" => Self::NotEquals,
            "contains" => Self::Contains,
            "not_contains" => Self::NotContains,
            "starts_with" => Self::StartsWith,
            "ends_with" => Self::EndsWith,
            "empty" => Self::Empty,
            "not_empty" => Self::NotEmpty,
            "greater_than" => Self::GreaterThan,
            "greater_than_or_equal" => Self::GreaterThanOrEqual,
            "less_than" => Self::LessThan,
            "less_than_or_equal" => Self::LessThanOrEqual,
            "is" => Self::Is,
            "is_not" => Self::IsNot,
            "exists" => Self::Exists,
            "not_exists" => Self::NotExists,
            _ => return None,
        })
    }
}

impl fmt::Display for ComparisonOperator {
    /// Renders the wire form of the operator for error messages.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let wire = match self {
            Self::Equals => "equals",
            Self::NotEquals => "not_equals",
            Self::Contains => "contains",
            Self::NotContains => "not_contains",
            Self::StartsWith => "starts_with",
            Self::EndsWith => "ends_with",
            Self::Empty => "empty",
            Self::NotEmpty => "not_empty",
            Self::GreaterThan => "greater_than",
            Self::GreaterThanOrEqual => "greater_than_or_equal",
            Self::LessThan => "less_than",
            Self::LessThanOrEqual => "less_than_or_equal",
            Self::Is => "is",
            Self::IsNot => "is_not",
            Self::Exists => "exists",
            Self::NotExists => "not_exists",
        };
        f.write_str(wire)
    }
}

/// Evaluates the whole Condition node against the run's variable pool, returning the selected
/// branch id (a case id or [`ELSE_BRANCH_ID`]).
pub fn evaluate_condition(
    config: &ConditionConfig,
    pool: &WorkflowVariablePool,
) -> Result<String, ConditionError> {
    for case in &config.cases {
        // An authored IF/ELIF shell has no rule until the user clicks Add condition; it must
        // fall through instead of matching vacuous `and` semantics and stealing the ELSE path.
        if case.conditions.is_empty() {
            continue;
        }
        let matched = match case.logic {
            ConditionLogic::And => case
                .conditions
                .iter()
                .map(|rule| evaluate_rule(rule, pool))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .all(|matched| matched),
            ConditionLogic::Or => case
                .conditions
                .iter()
                .map(|rule| evaluate_rule(rule, pool))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .any(|matched| matched),
        };
        if matched {
            return Ok(case.id.clone());
        }
    }
    Ok(ELSE_BRANCH_ID.to_string())
}

/// Evaluates one comparison rule against the run's variable pool.
fn evaluate_rule(
    rule: &ConditionRule,
    pool: &WorkflowVariablePool,
) -> Result<bool, ConditionError> {
    let resolved = pool
        .resolve(&rule.variable_selector)
        .map_err(|error| match error {
            WorkflowVariablePoolError::Undeclared { selector } => {
                ConditionError::Undeclared { selector }
            }
            WorkflowVariablePoolError::PathMissing { selector, segment } => {
                ConditionError::InvalidPath { selector, segment }
            }
            WorkflowVariablePoolError::PathNotObject { selector, path } => {
                ConditionError::TypeMismatch {
                    selector,
                    operator: rule.operator,
                    actual: path,
                    expected: "object".to_string(),
                }
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
    rule.operator.evaluate(
        resolved,
        rule.value.as_ref(),
        &rule.variable_selector.qualified(),
    )
}

impl ComparisonOperator {
    /// Evaluates this operator against the resolved variable, the expected literal, and the
    /// selector's qualified name used in error messages.
    ///
    /// Existence and emptiness operators tolerate an unassigned variable; every other operator
    /// fails on `None` so an unset value is never silently treated as null or an empty string.
    fn evaluate(
        self,
        resolved: Option<&Value>,
        expected: Option<&Value>,
        selector: &str,
    ) -> Result<bool, ConditionError> {
        match self {
            Self::Exists => return Ok(resolved.is_some()),
            Self::NotExists => return Ok(resolved.is_none()),
            Self::Empty => return Ok(is_empty(resolved)),
            Self::NotEmpty => return Ok(!is_empty(resolved)),
            _ => {}
        }
        let Some(actual) = resolved else {
            return Err(ConditionError::UnsetVariable {
                selector: selector.to_string(),
            });
        };
        let Some(expected) = expected else {
            return Err(ConditionError::TypeMismatch {
                selector: selector.to_string(),
                operator: self,
                actual: type_label(actual).to_string(),
                expected: "a comparison value".to_string(),
            });
        };
        let mismatched = || ConditionError::TypeMismatch {
            selector: selector.to_string(),
            operator: self,
            actual: type_label(actual).to_string(),
            expected: type_label(expected).to_string(),
        };
        match self {
            Self::Equals => Ok(equal(actual, expected)),
            Self::NotEquals => Ok(!equal(actual, expected)),
            Self::Contains => contains(actual, expected).ok_or_else(mismatched),
            Self::NotContains => Ok(!contains(actual, expected).ok_or_else(mismatched)?),
            Self::StartsWith => starts_with(actual, expected).ok_or_else(mismatched),
            Self::EndsWith => ends_with(actual, expected).ok_or_else(mismatched),
            Self::GreaterThan => compare_numbers(actual, expected)
                .map(std::cmp::Ordering::is_gt)
                .ok_or_else(mismatched),
            Self::GreaterThanOrEqual => compare_numbers(actual, expected)
                .map(std::cmp::Ordering::is_ge)
                .ok_or_else(mismatched),
            Self::LessThan => compare_numbers(actual, expected)
                .map(std::cmp::Ordering::is_lt)
                .ok_or_else(mismatched),
            Self::LessThanOrEqual => compare_numbers(actual, expected)
                .map(std::cmp::Ordering::is_le)
                .ok_or_else(mismatched),
            Self::Is => Ok(equal(actual, expected)),
            Self::IsNot => Ok(!equal(actual, expected)),
            Self::Empty | Self::NotEmpty | Self::Exists | Self::NotExists => {
                unreachable!("handled above")
            }
        }
    }
}

/// Whether a resolved variable counts as empty: unassigned, null, or an empty string/array/object.
fn is_empty(resolved: Option<&Value>) -> bool {
    match resolved {
        None => true,
        Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(map)) => map.is_empty(),
        Some(Value::Bool(_) | Value::Number(_)) => false,
    }
}

/// Value equality that understands numbers across integer and float representations.
fn equal(actual: &Value, expected: &Value) -> bool {
    match (as_number(actual), as_number(expected)) {
        (Some(left), Some(right)) => left == right,
        _ => actual == expected,
    }
}

/// Whether `actual` contains `expected`: substring for strings, element for arrays.
fn contains(actual: &Value, expected: &Value) -> Option<bool> {
    match actual {
        Value::String(text) => expected.as_str().map(|needle| text.contains(needle)),
        Value::Array(items) => Some(items.iter().any(|item| equal(item, expected))),
        _ => None,
    }
}

/// Whether a string starts with the expected substring.
fn starts_with(actual: &Value, expected: &Value) -> Option<bool> {
    Some(actual.as_str()?.starts_with(expected.as_str()?))
}

/// Whether a string ends with the expected substring.
fn ends_with(actual: &Value, expected: &Value) -> Option<bool> {
    Some(actual.as_str()?.ends_with(expected.as_str()?))
}

/// Numeric comparison across integers and floats, or `None` when either side is not a number.
fn compare_numbers(actual: &Value, expected: &Value) -> Option<std::cmp::Ordering> {
    let left = as_number(actual)?;
    let right = as_number(expected)?;
    left.partial_cmp(&right)
}

/// Reads a value as a number across integer and float representations.
fn as_number(value: &Value) -> Option<f64> {
    value.as_f64()
}

/// A short human-readable label for the type involved in a mismatch.
fn type_label(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(value) if value.is_i64() || value.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Compiles a wire case list, failing the test on invalid input.
    fn config(cases: Vec<WireConditionCase>) -> ConditionConfig {
        ConditionConfig::from_wire(cases).unwrap()
    }

    fn case(id: &str, logic: &str, rules: Vec<WireConditionRule>) -> WireConditionCase {
        WireConditionCase {
            id: Some(id.to_string()),
            logic: Some(logic.to_string()),
            conditions: rules,
        }
    }

    fn rule(selector: &[&str], operator: &str, value: Option<Value>) -> WireConditionRule {
        WireConditionRule {
            variable_selector: selector.iter().map(|part| part.to_string()).collect(),
            operator: Some(operator.to_string()),
            value,
        }
    }

    /// A pool preloaded with typed values matching a branch-aware review graph.
    fn pool_with_review() -> WorkflowVariablePool {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("review.structured_output", "object", "review");
        pool.declare("review.text", "string", "review");
        pool.set(
            "review.structured_output",
            "review",
            json!({ "approved": true, "score": 92 }),
        )
        .unwrap();
        pool.set("review.text", "review", json!("审查完成"))
            .unwrap();
        pool
    }

    /// Selects the matching case and falls back to `else` when no case holds.
    #[test]
    fn picks_the_first_matching_case_else_falls_back() {
        let pool = pool_with_review();
        let cfg = config(vec![
            case(
                "approved",
                "and",
                vec![rule(
                    &["review", "structured_output", "approved"],
                    "is",
                    Some(json!(true)),
                )],
            ),
            case(
                "score-hi",
                "and",
                vec![rule(
                    &["review", "structured_output", "score"],
                    "greater_than",
                    Some(json!(200)),
                )],
            ),
        ]);
        assert_eq!(evaluate_condition(&cfg, &pool).unwrap(), "approved");

        // Neither the approved nor the score case holds, so the implicit else branch wins.
        let mut rejected = pool_with_review();
        rejected
            .set(
                "review.structured_output",
                "review",
                json!({ "approved": false, "score": 10 }),
            )
            .unwrap();
        assert_eq!(
            evaluate_condition(&cfg, &rejected).unwrap(),
            ELSE_BRANCH_ID.to_string()
        );
    }

    /// A newly created empty IF shell falls through to ELSE until a rule is authored.
    #[test]
    fn empty_case_falls_through_to_else() {
        let cfg = config(vec![case("case-1", "and", Vec::new())]);

        assert_eq!(
            evaluate_condition(&cfg, &WorkflowVariablePool::default()).unwrap(),
            ELSE_BRANCH_ID.to_string()
        );
    }

    /// `and` requires every rule, `or` requires any rule.
    #[test]
    fn logic_and_requires_all_or_requires_any() {
        let pool = pool_with_review();
        let cfg = config(vec![case(
            "high",
            "and",
            vec![
                rule(&["review", "text"], "not_empty", None),
                rule(
                    &["review", "structured_output", "score"],
                    "greater_than",
                    Some(json!(90)),
                ),
            ],
        )]);
        assert_eq!(evaluate_condition(&cfg, &pool).unwrap(), "high");

        let or_cfg = config(vec![case(
            "flag",
            "or",
            vec![
                rule(&["review", "text"], "contains", Some(json!("不存在"))),
                rule(
                    &["review", "structured_output", "score"],
                    "greater_than_or_equal",
                    Some(json!(92)),
                ),
            ],
        )]);
        assert_eq!(evaluate_condition(&or_cfg, &pool).unwrap(), "flag");
    }

    /// Existence and emptiness operators tolerate an unassigned variable.
    #[test]
    fn existence_and_empty_handle_unset_variables() {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("agent-a.text", "string", "agent-a");
        let cfg = config(vec![case(
            "missing",
            "and",
            vec![rule(&["agent-a", "text"], "not_exists", None)],
        )]);
        assert_eq!(evaluate_condition(&cfg, &pool).unwrap(), "missing");
    }

    /// A value comparison against an unassigned variable fails instead of treating it as null.
    #[test]
    fn value_comparison_fails_on_unset_variable() {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("agent-a.text", "string", "agent-a");
        let cfg = config(vec![case(
            "matched",
            "and",
            vec![rule(&["agent-a", "text"], "equals", Some(json!("x")))],
        )]);
        assert_eq!(
            evaluate_condition(&cfg, &pool).unwrap_err(),
            ConditionError::UnsetVariable {
                selector: "agent-a.text".to_string()
            }
        );
    }

    /// String comparison operators cover equality, containment, and prefix/suffix checks.
    #[test]
    fn string_operators_match_on_text() {
        let pool = pool_with_review();
        let check = |operator: &str, value: Value| {
            config(vec![case(
                "c",
                "and",
                vec![rule(&["review", "text"], operator, Some(value))],
            )])
        };
        assert_eq!(
            evaluate_condition(&check("equals", json!("审查完成")), &pool).unwrap(),
            "c"
        );
        assert_eq!(
            evaluate_condition(&check("contains", json!("完成")), &pool).unwrap(),
            "c"
        );
        assert_eq!(
            evaluate_condition(&check("starts_with", json!("审查")), &pool).unwrap(),
            "c"
        );
        assert_eq!(
            evaluate_condition(&check("ends_with", json!("完成")), &pool).unwrap(),
            "c"
        );
        assert_eq!(
            evaluate_condition(&check("not_equals", json!("别的")), &pool).unwrap(),
            "c"
        );
    }

    /// Numeric operators compare across integer and float representations.
    #[test]
    fn numeric_operators_compare_across_representations() {
        let pool = pool_with_review();
        let check = |operator: &str, value: Value| {
            config(vec![case(
                "c",
                "and",
                vec![rule(
                    &["review", "structured_output", "score"],
                    operator,
                    Some(value),
                )],
            )])
        };
        assert_eq!(
            evaluate_condition(&check("greater_than", json!(90)), &pool).unwrap(),
            "c"
        );
        assert_eq!(
            evaluate_condition(&check("greater_than_or_equal", json!(92.0)), &pool).unwrap(),
            "c"
        );
        assert_eq!(
            evaluate_condition(&check("less_than", json!(100)), &pool).unwrap(),
            "c"
        );
        assert_eq!(
            evaluate_condition(&check("not_equals", json!(0)), &pool).unwrap(),
            "c"
        );
    }

    /// An unknown operator or a malformed selector fails compilation with a clear reason.
    #[test]
    fn rejects_unknown_operator_and_malformed_selector() {
        assert_eq!(
            ConditionConfig::from_wire(vec![case(
                "c",
                "and",
                vec![rule(&["review", "text"], "bogus", None)]
            )])
            .unwrap_err(),
            ConditionError::UnknownOperator {
                operator: "bogus".to_string()
            }
        );
        assert_eq!(
            ConditionConfig::from_wire(vec![case(
                "c",
                "and",
                vec![rule(&["only-node"], "exists", None)]
            )])
            .unwrap_err(),
            ConditionError::InvalidSelector {
                selector: vec!["only-node".to_string()]
            }
        );
    }

    /// A string operator applied to an object value fails as a type mismatch.
    #[test]
    fn string_operator_on_object_fails_as_type_mismatch() {
        let pool = pool_with_review();
        let cfg = config(vec![case(
            "c",
            "and",
            vec![rule(
                &["review", "structured_output"],
                "contains",
                Some(json!("approved")),
            )],
        )]);
        assert!(matches!(
            evaluate_condition(&cfg, &pool).unwrap_err(),
            ConditionError::TypeMismatch { .. }
        ));
    }

    /// The wire `cases` field round-trips through the compiler.
    #[test]
    fn compiles_wire_case_shape() {
        let cfg = config(vec![case(
            "approved",
            "or",
            vec![
                rule(&["review", "text"], "not_empty", None),
                rule(
                    &["review", "structured_output", "approved"],
                    "is",
                    Some(json!(true)),
                ),
            ],
        )]);
        assert_eq!(cfg.cases.len(), 1);
        assert_eq!(cfg.cases[0].id, "approved");
        assert_eq!(cfg.cases[0].logic, ConditionLogic::Or);
        assert_eq!(cfg.cases[0].conditions.len(), 2);
    }
}
