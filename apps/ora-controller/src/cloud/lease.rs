//! The global lease that fences every write: acquire it, keep it renewed, and give it back on
//! shutdown. Cloud records the holder and epoch; this module only asks.
use super::{CloudStore, fault};
use crate::*;
use ora_controller_proto::v1 as proto;
use std::time::Duration;

/// Cloud grants thirty seconds per lease; renewing every ten leaves two missed renewals of slack.
pub(super) const RENEW_INTERVAL: Duration = Duration::from_secs(/*secs*/ 10);

/// Acquires the lease when none is held, otherwise renews it; a renewal Cloud refuses as stale is
/// followed by an immediate acquisition attempt so eligibility returns without waiting a tick.
pub(super) async fn keep_lease(store: &CloudStore) {
    let held = match store.lease() {
        None => None,
        Some(epoch) => match lease_call(store, proto::RenewLeaseRequest { epoch }).await {
            Ok(lease) => Some(lease),
            Err(Error::StaleEligibility) => None,
            Err(error) => {
                ora_logging::ora_warn!(error = %error, "Cloud lease renewal failed; still eligible until it expires");
                return;
            }
        },
    };
    let lease = match held {
        Some(lease) => lease,
        None => match lease_call(store, proto::AcquireLeaseRequest {}).await {
            Ok(lease) => {
                ora_logging::ora_info!(epoch = lease.epoch, "Cloud lease acquired");
                lease
            }
            Err(error) => {
                ora_logging::ora_warn!(error = %error, "Cloud lease not acquired; no work is claimed or written");
                return;
            }
        },
    };
    store.set_lease(Some(lease.epoch));
}

/// Runs one lease RPC; the verdict's side effects apply as for any other call.
async fn lease_call<R: LeaseRequest>(
    store: &CloudStore,
    message: R,
) -> Result<proto::Lease, Error> {
    let call = async {
        let request = store.request(message);
        R::send(store, request).await
    };
    fault::read(call)
        .await
        .map_err(|verdict| store.settle(verdict))?
        .ok_or(Error::Conflict)
}

/// Gives the lease back on shutdown; failure only means a successor waits for expiry.
pub(super) async fn release(store: &CloudStore) {
    let Some(epoch) = store.lease() else {
        return;
    };
    store.set_lease(None);
    if let Err(error) = lease_call(store, proto::ReleaseLeaseRequest { epoch }).await {
        ora_logging::ora_warn!(error = %error, "Cloud lease not released; it expires on its own");
    }
}

/// The three lease requests share one call path; each names the client method it belongs to.
trait LeaseRequest: Sized {
    fn send(
        store: &CloudStore,
        request: tonic::Request<Self>,
    ) -> impl Future<Output = Result<tonic::Response<LeaseReply>, tonic::Status>> + Send;
}

/// The lease each reply carries, unwrapped from the reply message that names the call.
type LeaseReply = Option<proto::Lease>;

impl LeaseRequest for proto::AcquireLeaseRequest {
    async fn send(
        store: &CloudStore,
        request: tonic::Request<Self>,
    ) -> Result<tonic::Response<LeaseReply>, tonic::Status> {
        store
            .leases()
            .acquire_lease(request)
            .await
            .map(|response| response.map(|reply| reply.lease))
    }
}

impl LeaseRequest for proto::RenewLeaseRequest {
    async fn send(
        store: &CloudStore,
        request: tonic::Request<Self>,
    ) -> Result<tonic::Response<LeaseReply>, tonic::Status> {
        store
            .leases()
            .renew_lease(request)
            .await
            .map(|response| response.map(|reply| reply.lease))
    }
}

impl LeaseRequest for proto::ReleaseLeaseRequest {
    async fn send(
        store: &CloudStore,
        request: tonic::Request<Self>,
    ) -> Result<tonic::Response<LeaseReply>, tonic::Status> {
        store
            .leases()
            .release_lease(request)
            .await
            .map(|response| response.map(|reply| reply.lease))
    }
}
