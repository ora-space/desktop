use super::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn one_execution_cannot_take_a_third_slot_even_when_node_has_six_free() {
    let scheduler = Scheduler::default();
    let owner = control();
    let (started, mut receive) = tokio::sync::mpsc::unbounded_channel();
    scheduler.register(ExecutionId::new("only"), owner.clone(), move |input| {
        started.send(input).unwrap();
        Box::pin(std::future::pending())
    });
    scheduler.dispatch();
    let first = receive.recv().await.unwrap();
    let second = receive.recv().await.unwrap();
    assert!(receive.try_recv().is_err());
    assert_eq!(
        owner
            .snapshot(true)
            .members
            .iter()
            .filter(|m| m.probe_in_flight)
            .count(),
        2
    );
    owner.probe_result(
        &first,
        SessionMcpHealth::Unhealthy {
            code: SessionMcpProbeErrorCode::McpProbeTimeout,
        },
        true,
    );
    owner.probe_result(
        &second,
        SessionMcpHealth::Unhealthy {
            code: SessionMcpProbeErrorCode::McpProbeTimeout,
        },
        true,
    );
    scheduler.dispatch();
    assert!(receive.try_recv().is_err());
    assert_eq!(
        serde_json::to_value(scheduler.summary()).unwrap(),
        serde_json::json!({
            "status":"probe_capacity", "total":8,"running":0,"cleanup_pending":2,"queued":2,"available":6,"residues":[]
        })
    );
    scheduler.unregister(&ExecutionId::new("only"));
    assert_eq!(
        serde_json::to_value(scheduler.summary()).unwrap()["cleanup_pending"],
        2
    );
}

/// Each fixture has more pending work than its per-session allowance.
fn control() -> Control {
    let control = Control::new(
        &(0..4)
            .map(|index| SessionMcpBinding {
                plugin_id: PluginId::new(format!("test/mcp-{index}")),
                plugin_version: PluginVersion::new("1.0.0"),
                settings: Default::default(),
            })
            .collect::<Vec<_>>(),
    );
    control.begin();
    control.loaded(0);
    control
}

#[tokio::test]
async fn global_eight_session_two_and_round_robin_survive_slow_cleanup() {
    let scheduler = Scheduler::default();
    let (started, mut receive) = tokio::sync::mpsc::unbounded_channel();
    for index in 0..5 {
        let started = started.clone();
        scheduler.register(
            ExecutionId::new(format!("session-{index}")),
            control(),
            move |_input| {
                let (release, wait) = tokio::sync::oneshot::channel::<()>();
                started.send((index, release)).unwrap();
                Box::pin(async {
                    let _ = wait.await;
                })
            },
        );
    }
    scheduler.dispatch();
    let mut flights = Vec::new();
    let mut counts = [0; 5];
    for _ in 0..8 {
        let (index, release) =
            tokio::time::timeout(std::time::Duration::from_secs(1), receive.recv())
                .await
                .unwrap()
                .unwrap();
        counts[index] += 1;
        flights.push(release);
    }
    assert!(counts.iter().all(|count| *count <= 2));
    assert!(!scheduler.available());
    scheduler.dispatch();
    assert!(receive.try_recv().is_err());
    flights.remove(0).send(()).unwrap();
    let (next, _) = tokio::time::timeout(std::time::Duration::from_secs(1), receive.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next, 3);
    for index in 0..5 {
        scheduler.unregister(&ExecutionId::new(format!("session-{index}")));
    }
    drop(flights);
}

#[tokio::test]
async fn recovered_cleanup_consumes_capacity_before_any_new_values_are_captured() {
    let scheduler = Scheduler::default();
    scheduler.retain_recovered(8);
    let owner = control();
    scheduler.register(ExecutionId::new("session"), owner.clone(), |_| {
        panic!("no permit")
    });
    scheduler.dispatch();
    assert!(!scheduler.available());
    assert!(
        owner
            .snapshot(false)
            .members
            .iter()
            .all(|member| member.queued && !member.probe_in_flight)
    );
}
