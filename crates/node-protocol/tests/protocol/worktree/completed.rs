use super::*;

#[path = "completed/fixtures.rs"]
mod fixtures;
#[path = "completed/rejections.rs"]
mod rejections;

/// Keeps historical Worktree results byte-compatible through the common result seam.
#[tokio::test]
async fn preserves_completed_worktree_contracts() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_wire().await?;
        case.assert_round_trip().await?;
        case.assert_extensions().await?;
        case.assert_envelope_rejections().await?;
        case.assert_historical_node().await?;
    }
    Ok(())
}
