//! Actual gRPC adapter proof transport and committed-fact recovery; not a PostgreSQL substitute.
#![allow(clippy::unwrap_used)]
#[allow(dead_code)]
#[path = "fake_cloud.rs"]
mod fake_cloud;
use super::super::{lease, tests::config};
use crate::*;
use fake_cloud::FakeCloud;
#[path = "non_acceptance_fixture.rs"]
mod fixture;
use fixture::fence;
use pretty_assertions::assert_eq;

/// Exact original work/input survives both wire directions, while the RPC uses the current lease.
#[test]
fn original_proof_round_trips_and_reconnect_reads_committed_fact() {
    ora_logging::with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let cloud = FakeCloud::new();
                cloud.seed_execution_fence(fence());
                let served = cloud.serve().await;
                let mut configuration = config("owner");
                if let Persistence::Cloud { endpoint, .. } = &mut configuration.persistence {
                    endpoint.clone_from(&served.endpoint);
                }
                let store = CloudStore::open(&configuration).unwrap();
                lease::keep_lease(&store).await;
                let batch = store.execution_fences(&NodeId::new("node")).await.unwrap();
                assert_eq!(batch.committed, vec![]);
                assert_eq!(batch.requests.len(), 1);
                let original = batch.requests.into_iter().next().unwrap();
                assert_eq!(original.origin.binding.controller_epoch, 1);
                assert_eq!(
                    original.origin.input_json,
                    r#"{"kind":"deliver_revision","frozen":"原字节"}"#
                );
                let proof = ExecutionNonAcceptance {
                    proof_id: "11111111-1111-4111-8111-111111111111".into(),
                    fence: original,
                };
                let message = NodeToControllerMessage::ExecutionNotAccepted(proof.clone());
                let ack = take_over(
                    &store,
                    &NodeRuntimeIdentity {
                        node_id: NodeId::new("node"),
                        incarnation_id: NodeIncarnationId::new("original-incarnation"),
                    },
                    &message,
                )
                .await
                .unwrap();
                assert_eq!(ack, None);
                // Model a connection losing its local success observation. A fresh adapter uses durable proof,
                // rather than treating disappearance from ListPendingDispatches as evidence.
                lease::release(&store).await;
                let replacement = CloudStore::open(&configuration).unwrap();
                lease::keep_lease(&replacement).await;
                assert_eq!(replacement.lease(), Some(2));
                let recovered = replacement
                    .execution_fences(&NodeId::new("node"))
                    .await
                    .unwrap();
                assert_eq!(
                    recovered,
                    ExecutionFenceBatch {
                        requests: vec![],
                        committed: vec![ExecutionId::new("execution")]
                    }
                );
                assert_eq!(
                    take_over(
                        &replacement,
                        &NodeRuntimeIdentity {
                            node_id: NodeId::new("node"),
                            incarnation_id: NodeIncarnationId::new("new-incarnation")
                        },
                        &message
                    )
                    .await
                    .unwrap(),
                    None
                );
            });
    });
}
