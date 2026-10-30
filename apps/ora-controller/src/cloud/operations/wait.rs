//! Round-local scheduling budgets. Cloud owns durable retry times and execution registration time.
use ora_controller_proto::v1 as proto;
use std::time::Duration;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::time::Instant;

/// A round has only one handshake allowance, even across reconnects or different sandboxes.
#[derive(Default)]
pub(super) struct NodeGrace {
    deadline: Option<Instant>,
}

impl NodeGrace {
    /// Starts the allowance on the first unavailable observation, never on a ready observation.
    pub(super) fn deadline(&mut self) -> Instant {
        *self
            .deadline
            .get_or_insert_with(|| Instant::now() + Duration::from_secs(/*secs*/ 2))
    }
}

/// Existing unresolved evidence must not acquire timeout's automatic recovery semantics.
pub(super) enum CloneEvidence {
    AwaitingResult,
    Unresolved,
}

/// At the durable deadline, preserves observed Unknown instead of asking Cloud to time it out.
pub(super) fn clone_deadline_reason(
    created_at: Option<&str>,
    now: OffsetDateTime,
    evidence: CloneEvidence,
) -> Option<proto::DeferReason> {
    let expired = created_at
        .and_then(|created_at| OffsetDateTime::parse(created_at, &Rfc3339).ok())
        .is_some_and(|registered| now - registered >= time::Duration::minutes(/*minutes*/ 30));
    expired.then_some(match evidence {
        CloneEvidence::AwaitingResult => proto::DeferReason::CloneResultTimeout,
        CloneEvidence::Unresolved => proto::DeferReason::CloneResultUnknown,
    })
}

/// Waiting for one registered execution gets one five-second window for this round.
pub(super) fn clone_deadline() -> Instant {
    Instant::now() + Duration::from_secs(/*secs*/ 5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use time::macros::datetime;

    /// Repeated checks and connection flaps keep the first unavailable observation's deadline.
    #[tokio::test(start_paused = true)]
    async fn handshake_grace_is_shared_and_cannot_renew() {
        let mut grace = NodeGrace::default();
        let start = Instant::now();
        assert_eq!(grace.deadline(), start + Duration::from_secs(/*secs*/ 2));
        tokio::time::advance(Duration::from_millis(/*millis*/ 1_900)).await;
        assert_eq!(grace.deadline(), start + Duration::from_secs(/*secs*/ 2));
        tokio::time::advance(Duration::from_millis(/*millis*/ 100)).await;
        assert_eq!(Instant::now(), grace.deadline());
    }

    /// Every observation shares one five-second budget and a new round alone gets a new budget.
    #[tokio::test(start_paused = true)]
    async fn clone_window_uses_monotonic_time() {
        let deadline = clone_deadline();
        tokio::time::advance(Duration::from_millis(/*millis*/ 4_999)).await;
        assert!(Instant::now() < deadline);
        tokio::time::advance(Duration::from_millis(/*millis*/ 1)).await;
        assert_eq!(Instant::now(), deadline);
        assert_eq!(clone_deadline(), deadline + Duration::from_secs(/*secs*/ 5));
    }

    /// Restart/reclaim never changes the execution's durable registration boundary.
    #[test]
    fn total_window_uses_registration_wall_clock_at_the_exact_boundary() {
        let registration = Some("2026-10-29T10:00:00+08:00");
        assert_eq!(
            clone_deadline_reason(
                registration,
                datetime!(2026-10-29 10:29:59.999 +08:00),
                CloneEvidence::AwaitingResult,
            ),
            None
        );
        assert_eq!(
            clone_deadline_reason(
                registration,
                datetime!(2026-10-29 10:30:00 +08:00),
                CloneEvidence::AwaitingResult,
            ),
            Some(proto::DeferReason::CloneResultTimeout)
        );
        assert_eq!(
            clone_deadline_reason(
                registration,
                datetime!(2026-10-29 11:00:00 +08:00),
                CloneEvidence::AwaitingResult,
            ),
            Some(proto::DeferReason::CloneResultTimeout)
        );
        assert_eq!(
            clone_deadline_reason(
                None,
                datetime!(2026-10-29 11:00:00 +08:00),
                CloneEvidence::AwaitingResult,
            ),
            None
        );
        assert_eq!(
            clone_deadline_reason(
                Some("invalid"),
                datetime!(2026-10-29 11:00:00 +08:00),
                CloneEvidence::AwaitingResult,
            ),
            None
        );
    }

    /// A fresh Unknown at the total boundary blocks as Unknown instead of recoverable timeout.
    #[test]
    fn fresh_unknown_cannot_be_reclassified_as_timeout() {
        let registration = Some("2026-10-29T10:00:00+08:00");
        assert_eq!(
            clone_deadline_reason(
                registration,
                datetime!(2026-10-29 10:29:59 +08:00),
                CloneEvidence::Unresolved
            ),
            None
        );
        for now in [
            datetime!(2026-10-29 10:30:00 +08:00),
            datetime!(2026-10-29 10:30:59 +08:00),
        ] {
            assert_eq!(
                clone_deadline_reason(registration, now, CloneEvidence::Unresolved),
                Some(proto::DeferReason::CloneResultUnknown)
            );
        }
    }
}
