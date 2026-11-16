use super::*;
use ora_plugin_config::SettingValue;

fn member(revision: u64, value: &str) -> SessionMcpUpdateMember {
    SessionMcpUpdateMember::Configured {
        plugin_id: PluginId::new("test/mcp"),
        plugin_version: PluginVersion::new("1.0.0"),
        configuration_revision: revision,
        settings: [("token".into(), SettingValue::String(value.into()))].into(),
    }
}
fn owner() -> Control {
    Control::new(&[SessionMcpBinding {
        plugin_id: PluginId::new("test/mcp"),
        plugin_version: PluginVersion::new("1.0.0"),
        settings: [("token".into(), SettingValue::String("secret".into()))].into(),
    }])
}
fn update(generation: u64, members: Vec<SessionMcpUpdateMember>) -> UpdateSessionMcp {
    UpdateSessionMcp {
        node_id: NodeId::new("node"),
        request_id: RequestId::new("request"),
        generation,
        members,
    }
}
fn probe(generation: u64, attempt: u64) -> ProbeSessionMcp {
    ProbeSessionMcp {
        node_id: NodeId::new("node"),
        request_id: RequestId::new("probe"),
        generation,
        plugin_id: PluginId::new("test/mcp"),
        probe_attempt: attempt,
    }
}

#[test]
fn generation_comparison_precedes_resolution_and_admission_tracks_latest_target() {
    let owner = owner();
    owner.begin();
    owner.loaded(0);
    assert!(owner.admit(|| ()).is_ok());
    let next = update(1, vec![member(1, "new-secret")]);
    assert_eq!(owner.update(&next).0, SessionMcpResult::Accepted);
    assert!(matches!(
        owner.admit(|| panic!("must not commit")),
        Err(Admission::Pending)
    ));
    assert_eq!(owner.update(&next).0, SessionMcpResult::Duplicate);
    assert_eq!(
        owner.update(&update(0, vec![member(0, "changed")])).0,
        SessionMcpResult::Stale
    );
    owner.begin();
    owner.loaded(1);
    assert!(owner.admit(|| ()).is_ok());
    assert!(
        !serde_json::to_string(&owner.snapshot(true))
            .unwrap()
            .contains("secret")
    );
}

#[test]
fn active_and_inflight_values_prevent_reusing_a_configuration_revision() {
    let owner = owner();
    owner.begin();
    owner.loaded(0);
    let flight = owner.claim_probe().unwrap();
    owner.update(&update(1, vec![member(1, "new")]));
    owner.begin();
    owner.loaded(1);
    assert_eq!(
        owner.update(&update(2, vec![member(0, "different")])),
        (
            SessionMcpResult::Conflict,
            Some(SessionMcpErrorCode::ConfigurationRevisionConflict)
        )
    );
    owner.probe_result(&flight, SessionMcpHealth::Healthy, false);
    assert_eq!(
        owner.update(&update(2, vec![member(0, "different")])).0,
        SessionMcpResult::Accepted
    );
}

#[test]
fn stale_load_never_admits_and_unavailable_selection_is_accepted_but_blocked() {
    let owner = owner();
    owner.begin();
    owner.update(&update(
        1,
        vec![SessionMcpUpdateMember::Unavailable {
            plugin_id: PluginId::new("test/mcp"),
            code: SessionMcpUnavailable::McpPluginMissing,
        }],
    ));
    owner.loaded(0);
    assert_eq!(owner.snapshot(true).active.unwrap().generation, 0);
    assert!(owner.admit(|| ()).is_err());
    assert_eq!(owner.probe(&probe(1, 1)), SessionMcpResult::NotConfigured);
    assert_eq!(
        owner.update(&update(2, vec![])).0,
        SessionMcpResult::InvalidSelection
    );
}

#[test]
fn probe_intents_coalesce_and_old_generations_cannot_overlap_or_publish_health() {
    let owner = owner();
    owner.begin();
    owner.loaded(0);
    let old = owner.claim_probe().unwrap();
    assert_eq!(owner.probe(&probe(0, 2)), SessionMcpResult::Accepted);
    assert_eq!(owner.probe(&probe(0, 1)), SessionMcpResult::Stale);
    assert!(owner.claim_probe().is_none());
    owner.update(&update(1, vec![member(1, "new")]));
    owner.begin();
    owner.loaded(1);
    assert!(owner.claim_probe().is_none());
    owner.probe_result(&old, SessionMcpHealth::Healthy, false);
    assert!(matches!(
        owner.snapshot(true).members[0].health,
        SessionMcpHealth::Unknown { .. }
    ));
    let next = owner.claim_probe().unwrap();
    assert_eq!(next.generation, 1);
    owner.probe_result(
        &next,
        SessionMcpHealth::Unknown {
            reason: SessionMcpUnknownReason::ContextMissing,
        },
        true,
    );
    assert!(owner.claim_probe().is_none());
    owner.stop();
    assert_eq!(owner.probe(&probe(1, 3)), SessionMcpResult::SessionEnded);
    assert_eq!(
        owner.update(&update(2, vec![member(2, "later")])).0,
        SessionMcpResult::SessionEnded
    );
}
