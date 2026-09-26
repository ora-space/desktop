//! The Controller's side of Cloud's `Watch` stream: when to open it, what each signal or stream
//! end means for claiming, and the three states that decide the claim cadence. Signals only
//! accelerate claiming; ownership is still decided when `RecordDispatch` commits, so a lost signal
//! only costs latency.
//!
//! The transitions are pure and generic over the stream type, so every (state, event) pair is
//! unit-tested with `()` in place of a live gRPC stream.
use super::{CloudStore, fault};
use ora_controller_proto::v1::{self as proto, watch_response::Signal};
use tonic::codec::Streaming;

/// The live stream as the coordinator holds it.
pub(super) type Stream = Streaming<proto::WatchResponse>;

/// Where the stream stands; it alone decides whether a tick claims.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Signals<S> {
    /// No stream: claim every `claim_interval_ms` and keep trying to open one.
    Closed,
    /// A stream is established: claim on signals and once after every renewal.
    Live(S),
    /// The Cloud instance said it is stopping: claim nothing until a new stream opens.
    Drained,
}

/// How one attempt to open the stream ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Opening<S> {
    /// Response headers arrived, so the subscription exists and no later signal is missed.
    Established(S),
    /// `UNAVAILABLE` or no headers in time: no Cloud serves the stream yet.
    Unavailable(String),
    /// The epoch was stale; the adapter has already dropped it.
    Stale,
    /// A serving Cloud declined the stream with any other status.
    Refused(String),
}

/// What arrived on a live stream.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Event {
    WorkAvailable,
    /// A runtime Workspace operation became claimable.
    OperationAvailable,
    NodeAssignment,
    Drain,
    /// The stream ended with OK, which Cloud only does while draining.
    Ended,
    /// The stream ended with an error status: transport loss, forced stop, keepalive timeout.
    Broken,
    /// A signal this Controller does not know; newer Cloud contracts may add variants.
    Unrecognized,
}

/// Which coordinator tick an open attempt or claim decision belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tick {
    /// The ten-second lease renewal.
    Renew,
    /// The `claim_interval_ms` tick.
    Claim,
}

/// Whether the coordinator claims now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Claim {
    Now,
    Skip,
}

impl<S> Signals<S> {
    /// The epoch to open a stream under: only a held lease, and only when none is live.
    pub(super) fn open_epoch(&self, lease: Option<i64>) -> Option<i64> {
        match self {
            Self::Closed | Self::Drained => lease,
            Self::Live(_) => None,
        }
    }

    /// Applies an open attempt. Establishing always goes live and claims at once, which covers
    /// work Cloud accepted before the subscription existed. A drain ends only through a new stream
    /// or a refusal from a serving Cloud: `UNAVAILABLE` and timeouts are what a stopping or absent
    /// Cloud answers, and a stale epoch says nothing about whether the draining instance is gone.
    pub(super) fn opened(self, opening: Opening<S>) -> (Self, Claim) {
        match (self, opening) {
            (_, Opening::Established(stream)) => (Self::Live(stream), Claim::Now),
            (Self::Drained, Opening::Refused(_)) => (Self::Closed, Claim::Skip),
            (state @ (Self::Closed | Self::Drained), Opening::Unavailable(_) | Opening::Stale)
            | (state @ Self::Closed, Opening::Refused(_)) => (state, Claim::Skip),
            // The coordinator never opens a second stream; keep the live one if it ever did.
            (
                state @ Self::Live(_),
                Opening::Unavailable(_) | Opening::Stale | Opening::Refused(_),
            ) => (state, Claim::Skip),
        }
    }

    /// Applies what arrived on the stream. A clean end counts as a drain because Cloud ends
    /// `Watch` with OK only while draining, and unlike the `Drain` message it cannot be dropped
    /// from a full subscriber buffer.
    pub(super) fn received(self, event: Event) -> (Self, Claim) {
        match (self, event) {
            (Self::Live(stream), Event::WorkAvailable | Event::OperationAvailable) => {
                (Self::Live(stream), Claim::Now)
            }
            (Self::Live(stream), Event::NodeAssignment | Event::Unrecognized) => {
                (Self::Live(stream), Claim::Skip)
            }
            (Self::Live(_), Event::Drain | Event::Ended) => (Self::Drained, Claim::Skip),
            (Self::Live(_), Event::Broken) => (Self::Closed, Claim::Skip),
            // Only a live stream produces events.
            (state @ (Self::Closed | Self::Drained), _) => (state, Claim::Skip),
        }
    }

    /// A dropped epoch closes a live stream, which was opened under it; a drain outlives the
    /// epoch because only a new stream or a serving Cloud's refusal ends it.
    pub(super) fn lease_lost(self) -> Self {
        match self {
            Self::Live(_) => Self::Closed,
            state @ (Self::Closed | Self::Drained) => state,
        }
    }

    /// Whether a tick claims once any open attempt has been applied: without a stream the claim
    /// tick claims; with one the renewal backstops signals Cloud dropped from a full buffer.
    pub(super) fn claims_on(&self, tick: Tick) -> Claim {
        match (self, tick) {
            (Self::Closed, Tick::Claim) | (Self::Live(_), Tick::Renew) => Claim::Now,
            (Self::Closed, Tick::Renew)
            | (Self::Live(_), Tick::Claim)
            | (Self::Drained, Tick::Renew | Tick::Claim) => Claim::Skip,
        }
    }
}

/// Opens `Watch` under `epoch`. Cloud registers the subscription before it sends headers, so a
/// returned stream sees every signal published afterwards. A stale verdict drops the epoch through
/// the adapter like any other call. Failures are not logged here: the coordinator retries on every
/// tick and reports only when the kind of failure changes.
pub(super) async fn open(store: &CloudStore, epoch: i64) -> Opening<Stream> {
    let call = async {
        let request = store.request(proto::WatchRequest { epoch });
        store.signals().watch(request).await
    };
    match fault::open(call).await {
        Ok(stream) => Opening::Established(stream),
        Err(fault::Refusal::Unavailable(detail)) => Opening::Unavailable(detail.to_string()),
        Err(fault::Refusal::Verdict(verdict @ fault::Verdict::Stale(_))) => {
            store.settle(verdict);
            Opening::Stale
        }
        Err(fault::Refusal::Verdict(verdict)) => {
            Opening::Refused(store.settle(verdict).to_string())
        }
    }
}

/// Waits for the next event on a live stream; without one it never resolves, so the coordinator
/// can poll it unconditionally.
pub(super) async fn next(signals: &mut Signals<Stream>) -> Event {
    let Signals::Live(stream) = signals else {
        return std::future::pending().await;
    };
    match stream.message().await {
        Ok(Some(response)) => match response.signal {
            Some(Signal::WorkAvailable(work)) => {
                ora_logging::ora_info!(
                    operation_id = work.operation_id.as_deref().unwrap_or("unspecified"),
                    "Cloud signalled available work"
                );
                Event::WorkAvailable
            }
            Some(Signal::OperationAvailable(operation)) => {
                ora_logging::ora_info!(
                    operation_id = %operation.operation_id,
                    "Cloud signalled an available Workspace operation"
                );
                Event::OperationAvailable
            }
            Some(Signal::NodeAssignment(assignment)) => {
                ora_logging::ora_info!(
                    node_id = %assignment.node_id,
                    "Cloud signalled a Node assignment; ignored until scale-out defines it"
                );
                Event::NodeAssignment
            }
            Some(Signal::Drain(_)) => Event::Drain,
            None => Event::Unrecognized,
        },
        Ok(None) => Event::Ended,
        Err(status) => {
            ora_logging::ora_warn!(code = ?status.code(), message = %status.message(), "Cloud signal stream broke");
            Event::Broken
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    type State = Signals<()>;

    /// Opening is attempted only under a held lease and never beside a live stream.
    #[test]
    fn opens_only_under_a_lease_without_a_live_stream() {
        let cases = [
            (State::Closed, Some(3), Some(3)),
            (State::Drained, Some(3), Some(3)),
            (State::Live(()), Some(3), None),
            (State::Closed, None, None),
            (State::Drained, None, None),
            (State::Live(()), None, None),
        ];
        for (state, lease, expected) in cases {
            assert_eq!(
                state.open_epoch(lease),
                expected,
                "{state:?} lease={lease:?}"
            );
        }
    }

    /// Every (state, opening) pair lands where D1 and D3 of the decision put it.
    #[test]
    fn open_attempts_follow_the_decision_table() {
        let cases = [
            (
                State::Closed,
                Opening::Established(()),
                State::Live(()),
                Claim::Now,
            ),
            (
                State::Closed,
                Opening::Unavailable(String::new()),
                State::Closed,
                Claim::Skip,
            ),
            (State::Closed, Opening::Stale, State::Closed, Claim::Skip),
            (
                State::Closed,
                Opening::Refused(String::new()),
                State::Closed,
                Claim::Skip,
            ),
            (
                State::Drained,
                Opening::Established(()),
                State::Live(()),
                Claim::Now,
            ),
            (
                State::Drained,
                Opening::Unavailable(String::new()),
                State::Drained,
                Claim::Skip,
            ),
            (State::Drained, Opening::Stale, State::Drained, Claim::Skip),
            (
                State::Drained,
                Opening::Refused(String::new()),
                State::Closed,
                Claim::Skip,
            ),
            (
                State::Live(()),
                Opening::Unavailable(String::new()),
                State::Live(()),
                Claim::Skip,
            ),
            (
                State::Live(()),
                Opening::Stale,
                State::Live(()),
                Claim::Skip,
            ),
            (
                State::Live(()),
                Opening::Refused(String::new()),
                State::Live(()),
                Claim::Skip,
            ),
        ];
        for (state, opening, expected, claim) in cases {
            let label = format!("{state:?} + {opening:?}");
            assert_eq!(state.opened(opening), (expected, claim), "{label}");
        }
    }

    /// Stream events claim on work, pause on a drain or clean end, and fall back on a break.
    #[test]
    fn stream_events_follow_the_decision_table() {
        let cases = [
            (Event::WorkAvailable, State::Live(()), Claim::Now),
            (Event::OperationAvailable, State::Live(()), Claim::Now),
            (Event::NodeAssignment, State::Live(()), Claim::Skip),
            (Event::Unrecognized, State::Live(()), Claim::Skip),
            (Event::Drain, State::Drained, Claim::Skip),
            (Event::Ended, State::Drained, Claim::Skip),
            (Event::Broken, State::Closed, Claim::Skip),
        ];
        for (event, expected, claim) in cases {
            let label = format!("{event:?}");
            assert_eq!(
                State::Live(()).received(event),
                (expected, claim),
                "{label}"
            );
        }
        assert_eq!(
            State::Closed.received(Event::WorkAvailable),
            (State::Closed, Claim::Skip)
        );
        assert_eq!(
            State::Drained.received(Event::WorkAvailable),
            (State::Drained, Claim::Skip)
        );
    }

    /// Losing the lease closes a live stream but leaves a drain in place.
    #[test]
    fn a_lost_lease_closes_only_a_live_stream() {
        assert_eq!(State::Live(()).lease_lost(), State::Closed);
        assert_eq!(State::Closed.lease_lost(), State::Closed);
        assert_eq!(State::Drained.lease_lost(), State::Drained);
    }

    /// Without a stream the claim tick claims; with one only the renewal does; a drain claims
    /// on neither.
    #[test]
    fn ticks_claim_by_stream_state() {
        let cases = [
            (State::Closed, Tick::Claim, Claim::Now),
            (State::Closed, Tick::Renew, Claim::Skip),
            (State::Live(()), Tick::Claim, Claim::Skip),
            (State::Live(()), Tick::Renew, Claim::Now),
            (State::Drained, Tick::Claim, Claim::Skip),
            (State::Drained, Tick::Renew, Claim::Skip),
        ];
        for (state, tick, expected) in cases {
            assert_eq!(state.claims_on(tick), expected, "{state:?} on {tick:?}");
        }
    }
}
