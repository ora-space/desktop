//! Exercises the same configuration inputs and preview requests as the run form.
use super::*;
use serde_json::json;

const CONDITION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/app-shell/src/features/workflow-editor/fixtures/run-condition-preview.json"
));
const DEFAULTS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/app-shell/src/features/workflow-editor/fixtures/run-debug-defaults.json"
));

/// Deserialize the wire request and apply it through the preview's production patch validator.
fn apply(data: &mut Value, kind: &str, preview: &Value) {
    let request: PreviewWorkflowDebugRequest = serde_json::from_value(preview.clone()).unwrap();
    replace(data, kind, &request.patch).unwrap();
}

#[test]
fn condition_form_edits_the_existing_branch_in_place() {
    let sample: Value = serde_json::from_str(CONDITION).unwrap();
    let mut data = json!({"kind": "condition", "cases": sample["cases"]});
    super::super::normalize::data(&mut data);
    let mut expected = data.clone();
    expected["cases"][0]["conditions"][0]["operator"] = json!("not_empty");
    apply(&mut data, "condition", &sample["preview"]);
    assert_eq!(data, expected);
}

#[test]
fn missing_form_fields_use_the_same_defaults_before_preview() {
    let samples: Value = serde_json::from_str(DEFAULTS).unwrap();
    for (kind, sample) in samples.as_object().unwrap() {
        let patch = &sample["preview"]["patch"][0];
        let path = patch["path"].as_str().unwrap();
        let mut data = sample["data"].clone();
        assert!(data.pointer(path).is_none());
        super::super::normalize::data(&mut data);
        assert_eq!(data.pointer(path), Some(&sample["default"]));
        apply(&mut data, kind, &sample["preview"]);
        assert_eq!(data.pointer(path), Some(&patch["value"]));
    }
}
