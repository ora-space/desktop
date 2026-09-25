//! The adapter's long-running coordination with Cloud: keep the lease, keep a `Watch` stream
//! while it is held, and claim when the stream state says so. Cloud stays the authority
//! throughout; this loop only decides when to ask.
use super::{
    CloudStore,
    claim::{self, Backlog, Refusals},
    lease,
    signals::{self, Claim, Opening, Signals, Stream, Tick},
};
use std::{io, mem};
use tokio::time::{MissedTickBehavior, interval};

/// Keeps this Controller eligible and fed until `shutdown` resolves, then closes the stream and
/// releases the lease so a successor need not wait for expiry. Every failure is logged and retried
/// on a later tick; the loop never gives up on the authority, because nothing local could take
/// its place.
///
/// Branches are biased in priority order: shutdown, renewal (a long claim backlog must not let the
/// lease expire), stream events, a pending backlog, then the claim tick.
pub(super) async fn coordinate(
    store: CloudStore,
    shutdown: impl Future<Output = ()>,
) -> io::Result<()> {
    let mut renew = interval(lease::RENEW_INTERVAL);
    renew.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut claim_tick = interval(store.inner.claim_interval);
    claim_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut signals = Signals::<Stream>::Closed;
    let mut report = Report::default();
    let mut refusals = Refusals::default();
    let mut backlog = Backlog::Settled;
    tokio::pin!(shutdown);
    loop {
        let claim = tokio::select! {
            biased;
            _ = &mut shutdown => break,
            _ = renew.tick() => {
                lease::keep_lease(&store).await;
                tick(&store, &mut signals, &mut report, Tick::Renew).await
            }
            event = signals::next(&mut signals) => {
                let (next, claim) = mem::replace(&mut signals, Signals::Closed).received(event);
                signals = next;
                report.state(&signals);
                claim
            }
            _ = std::future::ready(()), if backlog == Backlog::Pending => Claim::Now,
            _ = claim_tick.tick() => tick(&store, &mut signals, &mut report, Tick::Claim).await,
        };
        backlog = match (claim, &signals) {
            (Claim::Now, _) => claim::batch(&store, &mut refusals).await,
            // A drain stops the backlog too: the instance that holds it is stopping.
            (Claim::Skip, Signals::Drained) => Backlog::Settled,
            (Claim::Skip, Signals::Closed | Signals::Live(_)) => backlog,
        };
        // Any stale verdict above dropped the epoch; the stream opened under it goes with it.
        if store.lease().is_none() {
            signals = mem::replace(&mut signals, Signals::Closed).lease_lost();
            report.state(&signals);
        }
    }
    // Dropping the stream cancels it before the lease it was opened under is released.
    drop(signals);
    lease::release(&store).await;
    Ok(())
}

/// Runs one tick: opens the stream when the lease is held and none is live, then decides whether
/// the tick claims. Establishing a stream claims at once so work accepted before the subscription
/// existed is not left for the backstop.
async fn tick(
    store: &CloudStore,
    signals: &mut Signals<Stream>,
    report: &mut Report,
    tick: Tick,
) -> Claim {
    let opened = match signals.open_epoch(store.lease()) {
        Some(epoch) => {
            let opening = signals::open(store, epoch).await;
            report.opening(&opening);
            let (next, claim) = mem::replace(signals, Signals::Closed).opened(opening);
            *signals = next;
            report.state(signals);
            claim
        }
        None => Claim::Skip,
    };
    match (opened, signals.claims_on(tick)) {
        (Claim::Now, _) | (_, Claim::Now) => Claim::Now,
        (Claim::Skip, Claim::Skip) => Claim::Skip,
    }
}

/// Logs stream transitions and open failures once per change, not once per tick: without a
/// stream the loop retries on every claim tick, which may be many times a second.
#[derive(Default)]
struct Report {
    state: Option<Phase>,
    failure: Option<Failure>,
}

/// The state kind a transition is reported for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Closed,
    Live,
    Drained,
}

/// The kind of open failure last reported.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Failure {
    Unavailable,
    Refused,
}

impl Report {
    /// Reports a state the loop entered, if it differs from the last reported one.
    fn state(&mut self, signals: &Signals<Stream>) {
        let phase = match signals {
            Signals::Closed => Phase::Closed,
            Signals::Live(_) => Phase::Live,
            Signals::Drained => Phase::Drained,
        };
        let previous = self.state.replace(phase);
        if previous == Some(phase) {
            return;
        }
        match (previous, phase) {
            // Starting without a stream is the expected initial state, not a fallback.
            (None, Phase::Closed) => {}
            (_, Phase::Live) => {
                self.failure = None;
                ora_logging::ora_info!("Cloud signal stream established; claiming on signals");
            }
            (_, Phase::Drained) => ora_logging::ora_info!(
                "Cloud is draining; claiming paused until a new signal stream opens"
            ),
            (Some(Phase::Drained), Phase::Closed) => ora_logging::ora_warn!(
                "Cloud refused the signal stream after draining; claiming periodically"
            ),
            (Some(Phase::Live | Phase::Closed), Phase::Closed) => ora_logging::ora_warn!(
                "Cloud signal stream closed; claiming periodically until it reopens"
            ),
        }
    }

    /// Reports an open failure when its kind differs from the last reported one.
    fn opening<S>(&mut self, opening: &Opening<S>) {
        let (failure, detail) = match opening {
            Opening::Established(_) | Opening::Stale => return,
            Opening::Unavailable(detail) => (Failure::Unavailable, detail),
            Opening::Refused(detail) => (Failure::Refused, detail),
        };
        if self.failure.replace(failure) == Some(failure) {
            return;
        }
        match failure {
            Failure::Unavailable => {
                ora_logging::ora_warn!(detail = %detail, "Cloud signal stream unavailable");
            }
            Failure::Refused => {
                ora_logging::ora_warn!(detail = %detail, "Cloud refused the signal stream");
            }
        }
    }
}
