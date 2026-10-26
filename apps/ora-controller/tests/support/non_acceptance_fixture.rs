use ora_controller_proto::v1 as proto;

/// Keeps a complete original binding distinguishable from the replacement Controller's lease.
pub fn fence() -> proto::ExecutionFence {
    let origin = proto::RuntimeBinding {
        tenant_id: "tenant".into(),
        workspace_id: "workspace".into(),
        sandbox_id: "sandbox".into(),
        runtime_generation: 3,
        node_id: "node".into(),
        node_incarnation_id: "original-incarnation".into(),
        node_instance_id: "original-instance".into(),
        controller_epoch: 1,
        control_epoch: 4,
        control_version: 5,
        session_id: String::new(),
        actor_user_id: "actor".into(),
        operation_id: "original-run".into(),
        execution_id: "execution".into(),
        node_operation_id: "node-operation".into(),
        input_closed: false,
        issued_at_ms: 1_000,
        expires_at_ms: 61_000,
    };
    let mut authority = origin.clone();
    authority.controller_epoch = 1;
    proto::ExecutionFence {
        authority: Some(authority),
        origin: Some(proto::ExecutionOrigin {
            binding: Some(origin),
            work_id: "original-work".into(),
            input_json: r#"{"kind":"deliver_revision","frozen":"原字节"}"#.into(),
        }),
    }
}
