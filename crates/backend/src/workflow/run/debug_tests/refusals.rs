//! Confirm that both language packs exercise the wire errors produced by real previews.
use super::*;
use ora_contracts::RequestId;
use pretty_assertions::assert_eq;

/// Each refusal comes from the same started workflow and goes through the transport projection.
#[test]
fn rejected_previews_match_the_run_page_fixtures() {
    run_test(async {
        let fixture = Fixture::new(Arc::new(NoSessions));
        let agent = fixture.row("x");
        let expected: Vec<serde_json::Value> = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packages/app-shell/src/features/workflow-run/fixtures/debug-refusals.json"
        )))
        .unwrap();
        let pending = json!({"pendingTarget": {
            "rootScopeId": agent.scope_id.as_ref(), "scopeId": agent.scope_id.as_ref(), "nodeId": "out"
        }});
        let current = json!({"nodeRunId": agent.id.as_ref()});
        let requests = [
            (pending, json!("new")),
            (current.clone(), json!("{{#absent.value#}}")),
            (json!({"nodeRunId":"unknown"}), json!("new")),
            (current, json!(false)),
        ];
        let actual: Vec<_> = requests
            .into_iter()
            .map(|(mut selection, value)| {
                selection["runId"] = json!(fixture.run_id.as_ref());
                selection["baseRevision"] = json!(0);
                selection["patch"] =
                    json!([{"op":"replace","path":"/agentConfig/prompt","value":value}]);
                let error = fixture
                    .coordinator
                    .preview(serde_json::from_value(selection).unwrap())
                    .unwrap_err();
                let id: RequestId =
                    serde_json::from_value(json!("00000000-0000-4000-8000-000000000001")).unwrap();
                serde_json::to_value(error.contract_error(id)).unwrap()
            })
            .collect();
        assert_eq!(actual, expected);
    });
}
