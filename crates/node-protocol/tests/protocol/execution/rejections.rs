use super::*;

/// Checks required structure separately from present-but-empty opaque values.
#[tokio::test]
async fn rejects_missing_and_empty_fields() -> Result<(), TestError> {
    fixtures::get_execution_status()
        .assert_fields(
            &[
                "/message_type",
                "/protocol_version",
                "/payload",
                "/operation_id",
                "/execution_id",
                "/payload/node_id",
            ],
            &[
                ("/operation_id", "operation_id"),
                ("/execution_id", "execution_id"),
                ("/payload/node_id", "node_id"),
            ],
        )
        .await?;
    fixtures::event_ack()
        .assert_fields(
            &[
                "/message_type",
                "/protocol_version",
                "/payload",
                "/sequence",
                "/operation_id",
                "/execution_id",
                "/payload/node_id",
            ],
            &[
                ("/operation_id", "operation_id"),
                ("/execution_id", "execution_id"),
                ("/payload/node_id", "node_id"),
            ],
        )
        .await?;
    fixtures::execution_status_unknown()
        .assert_fields(
            &[
                "/message_type",
                "/protocol_version",
                "/payload",
                "/payload/state",
                "/payload/state/state",
                "/operation_id",
                "/execution_id",
                "/payload/node/node_id",
                "/payload/node/incarnation_id",
            ],
            &[
                ("/operation_id", "operation_id"),
                ("/execution_id", "execution_id"),
                ("/payload/node/node_id", "node.node_id"),
                ("/payload/node/incarnation_id", "node.incarnation_id"),
            ],
        )
        .await?;
    fixtures::execution_status_accepted()
        .assert_fields(
            &[
                "/message_type",
                "/protocol_version",
                "/payload",
                "/payload/state",
                "/payload/state/state",
                "/operation_id",
                "/execution_id",
                "/payload/node/node_id",
                "/payload/node/incarnation_id",
            ],
            &[
                ("/operation_id", "operation_id"),
                ("/execution_id", "execution_id"),
                ("/payload/node/node_id", "node.node_id"),
                ("/payload/node/incarnation_id", "node.incarnation_id"),
            ],
        )
        .await?;
    fixtures::execution_status_running()
        .assert_fields(
            &[
                "/message_type",
                "/protocol_version",
                "/payload",
                "/payload/state",
                "/payload/state/state",
                "/operation_id",
                "/execution_id",
                "/payload/node/node_id",
                "/payload/node/incarnation_id",
            ],
            &[
                ("/operation_id", "operation_id"),
                ("/execution_id", "execution_id"),
                ("/payload/node/node_id", "node.node_id"),
                ("/payload/node/incarnation_id", "node.incarnation_id"),
            ],
        )
        .await?;
    Ok(())
}
