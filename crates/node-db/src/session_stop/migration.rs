//! Inspect the old schema before migrating; an undrained upgrade preserves uncertainty.
use super::*;
use ora_node_protocol::{NodeId, NodeIncarnationId, NodeRuntimeIdentity, PluginId, PluginVersion};

/// The pre-migration inventory retains only the package identities needed to fence installation.
pub(crate) struct LegacyAgent {
    execution: ExecutionId,
    checkout: ExecutionId,
    packages: Vec<SessionStopPackage>,
}

/// A read-only inventory taken before any migration can rewrite legacy execution state.
pub(crate) fn legacy_running(
    connection: &Connection,
    version: i64,
) -> Result<Vec<LegacyAgent>, Error> {
    if !(8..14).contains(&version) {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare("SELECT execution,input FROM node_executions WHERE kind='agent_session' AND state='running'")?;
    statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .map(|row| {
            let (execution, input) = row?;
            let input: serde_json::Value = serde_json::from_str(&input)?;
            let spec = &input["payload"]["spec"];
            let text = |key| {
                spec[key]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(Error::InvalidSchema)
            };
            let mut packages = vec![SessionStopPackage {
                plugin_id: PluginId::new(text("agent_plugin_id")?),
                plugin_version: PluginVersion::new(text("agent_plugin_version")?),
                view_key: None,
            }];
            for binding in spec["mcp"].as_array().into_iter().flatten() {
                packages.push(SessionStopPackage {
                    plugin_id: PluginId::new(
                        binding["plugin_id"].as_str().ok_or(Error::InvalidSchema)?,
                    ),
                    plugin_version: PluginVersion::new(
                        binding["plugin_version"]
                            .as_str()
                            .ok_or(Error::InvalidSchema)?,
                    ),
                    view_key: None,
                });
            }
            Ok(LegacyAgent {
                execution: ExecutionId::new(execution),
                checkout: ExecutionId::new(text("checkout_execution_id")?),
                packages,
            })
        })
        .collect()
}

/// Installs the schema and legacy fences in the same transaction, without manufacturing exits.
pub(crate) fn migrate(
    connection: &mut Connection,
    legacy: Vec<LegacyAgent>,
    node_id: &NodeId,
    host: &SessionStopHost,
) -> Result<(), Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::InvalidTransition)?
        .as_millis();
    let now = i64::try_from(now).map_err(|_| Error::InvalidTransition)?;
    let tx = connection.transaction()?;
    tx.execute_batch(include_str!("../session_stop.sql"))?;
    for LegacyAgent {
        execution,
        checkout,
        packages,
    } in legacy
    {
        let identity = SessionStopIdentity {
            attempt: AttemptId::generate(),
            execution,
            checkout,
            node: NodeRuntimeIdentity {
                node_id: node_id.clone(),
                incarnation_id: NodeIncarnationId::new("legacy-unknown"),
            },
            host: host.clone(),
            kind: SessionStopKind::LegacyAgent,
            packages,
        };
        tx.execute("INSERT INTO session_stop_attempts(attempt,execution,checkout,record,state,created_at_ms,first_stop_at_ms,bound_at_ms) VALUES(?1,?2,?3,?4,'legacy_unknown',?5,?5,?6)",
            params![identity.attempt.as_str(),identity.execution.as_str(),identity.checkout.as_str(),serde_json::to_string(&identity)?,now,now.saturating_add(600_000)])?;
    }
    tx.pragma_update(None, "user_version", 14)?;
    tx.commit()?;
    Ok(())
}
