//! Expand editable defaults on a private copy of the frozen graph before replay.
use serde_json::{Value, json};

pub(super) fn data(data: &mut Value) {
    match data["kind"].as_str() {
        Some("agent") => {
            if let Some(config) = data.get_mut("agentConfig").and_then(Value::as_object_mut) {
                config.entry("prompt").or_insert(json!(""));
                if let Some(executor) = config.get_mut("executor").and_then(Value::as_object_mut) {
                    executor.entry("modelId").or_insert(json!(""));
                }
            }
        }
        Some("iteration") => {
            if let Some(config) = data
                .get_mut("iterationConfig")
                .and_then(Value::as_object_mut)
            {
                config.entry("errorStrategy").or_insert(json!("fail"));
            }
        }
        Some("condition") => {
            if data.get("cases").is_none() {
                if let Some(cases) = data.get("conditionCases").cloned() {
                    data["cases"] = cases;
                } else if let Some(branches) = data["conditionBranches"].as_array() {
                    data["cases"] =
                        Value::Array(branches.iter().enumerate().map(legacy_case).collect());
                }
            }
            for case in data["cases"].as_array_mut().into_iter().flatten() {
                if let Some(obj) = case.as_object_mut() {
                    obj.entry("logic").or_insert(json!("and"));
                }
            }
        }
        _ => {}
    }
}

/// Mirrors the editor's case IDs and named operator aliases for previously saved branches.
fn legacy_case((index, branch): (usize, &Value)) -> Value {
    let conditions: Vec<_> = branch["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(legacy_rule)
        .collect();
    json!({"id":format!("case-{}",index+1),"logic":branch["logic"].as_str().unwrap_or("and"),"conditions":conditions})
}

fn legacy_rule(rule: &Value) -> Value {
    let operator = match rule["operator"].as_str().unwrap_or("") {
        "is_empty" => "empty",
        "is_not_empty" => "not_empty",
        value => value,
    };
    let operator = match (operator, rule["negated"].as_bool()) {
        ("equals", Some(true)) => "not_equals",
        ("contains", Some(true)) => "not_contains",
        ("empty", Some(true)) => "not_empty",
        _ => operator,
    };
    let selector: Vec<_> = rule["variable"]
        .as_str()
        .unwrap_or("")
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let mut result = json!({"variableSelector":selector,"operator":operator});
    if let Some(value) = rule.get("value").filter(|v| *v != "") {
        result["value"] = value.clone();
    }
    result
}
