//! The single place where Cloud's gRPC verdicts become persistence classes, and the retry rule
//! for writes whose reply was lost. Nothing outside this module inspects a status code.
use ora_controller_proto::v1::{ErrorCode, ErrorDetail};
use prost::Message;
use std::{fmt, future::Future, time::Duration};
use tonic::{Code, Status};

/// Each call waits this long for a reply before treating the outcome as lost.
pub(super) const RPC_TIMEOUT: Duration = Duration::from_secs(/*secs*/ 10);
/// How many times one submission identity is presented before an unknown outcome is reported.
const WRITE_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(/*millis*/ 250);

/// Cloud's classification of one call, before the adapter applies its side effects: a stale
/// verdict drops the held lease.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// No durable record exists for the identity; reads report `None`, facts report a conflict.
    NotFound,
    /// Identity or content disagrees with the durable record; retrying as-is cannot succeed.
    Conflict,
    /// The lease epoch is not current or another holder owns the lease.
    Stale(Detail),
    /// Nothing was committed and the same call may be retried later.
    Unavailable(Detail),
    /// The reply was lost; the call may have been committed.
    Unknown(Detail),
}

/// What the status carried, kept for logs and messages: the refined contract code when Cloud
/// attached one, otherwise the transport-level code and message.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Detail {
    code: Code,
    refined: Option<ErrorCode>,
    message: String,
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.refined {
            Some(refined) => write!(
                f,
                "{} ({:?}): {}",
                refined.as_str_name(),
                self.code,
                self.message
            ),
            None => write!(f, "{:?}: {}", self.code, self.message),
        }
    }
}

/// `google.rpc.Status` as tonic carries it in the `grpc-status-details-bin` trailer; only the
/// details are read, the code and message are already on the status itself.
#[derive(Clone, PartialEq, Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<prost_types::Any>,
}

/// Maps one status to the contract's classification: the status code is primary, the attached
/// `ErrorDetail` refines it. Cloud produces both in one place, so they never disagree; the refined
/// code is therefore informational here.
pub(super) fn classify(status: &Status) -> Verdict {
    let refined = RpcStatus::decode(status.details())
        .ok()
        .and_then(|rpc| {
            rpc.details
                .iter()
                .find_map(|any| ErrorDetail::decode(any.value.as_slice()).ok())
        })
        .and_then(|detail| ErrorCode::try_from(detail.code).ok());
    let detail = Detail {
        code: status.code(),
        refined,
        message: status.message().to_owned(),
    };
    match status.code() {
        Code::NotFound => Verdict::NotFound,
        Code::Aborted | Code::InvalidArgument | Code::AlreadyExists | Code::OutOfRange => {
            Verdict::Conflict
        }
        Code::FailedPrecondition => Verdict::Stale(detail),
        // Cloud authenticates no Controller at this stage, so a refusal means the deployments
        // disagree; nothing was committed and the call is retried like an outage.
        Code::Unavailable
        | Code::ResourceExhausted
        | Code::Unimplemented
        | Code::Unauthenticated
        | Code::PermissionDenied => Verdict::Unavailable(detail),
        // A deadline, cancellation or broken stream after the request left may have committed.
        Code::DeadlineExceeded
        | Code::Cancelled
        | Code::Unknown
        | Code::Internal
        | Code::DataLoss
        | Code::Ok => Verdict::Unknown(detail),
    }
}

/// One attempt of a call: the response or the status Cloud answered with.
pub(super) type Attempt<T> = Result<tonic::Response<T>, Status>;

/// Runs one read. A lost reply commits nothing, so it is unavailable rather than unknown.
pub(super) async fn read<T>(call: impl Future<Output = Attempt<T>>) -> Result<T, Verdict> {
    match tokio::time::timeout(RPC_TIMEOUT, call).await {
        Ok(Ok(response)) => Ok(response.into_inner()),
        Ok(Err(status)) => Err(classify(&status)),
        Err(_elapsed) => Err(Verdict::Unavailable(Detail {
            code: Code::DeadlineExceeded,
            refined: None,
            message: "no reply before the read deadline".into(),
        })),
    }
}

/// Why a `Watch` did not open. Opening commits nothing, so like a read it is never unknown; unlike a
/// read it must tell `UNAVAILABLE` apart from other refusals: a draining or stopped Cloud refuses
/// with exactly that code, while any other verdict comes from a serving Cloud that declines the
/// stream, which the signal loop must not mistake for a drain that never ends.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// `UNAVAILABLE` or no response headers before the deadline: no Cloud is serving yet.
    Unavailable(Detail),
    /// Any other status, classified as for any call (a stale epoch included).
    Verdict(Verdict),
}

/// Waits for a stream to open: the response headers, which Cloud sends only after the
/// subscription exists, or the status it refused with.
pub(super) async fn open<T>(call: impl Future<Output = Attempt<T>>) -> Result<T, Refusal> {
    match tokio::time::timeout(RPC_TIMEOUT, call).await {
        Ok(Ok(response)) => Ok(response.into_inner()),
        Ok(Err(status)) if status.code() == Code::Unavailable => {
            Err(Refusal::Unavailable(Detail {
                code: Code::Unavailable,
                refined: None,
                message: status.message().to_owned(),
            }))
        }
        Ok(Err(status)) => Err(Refusal::Verdict(classify(&status))),
        Err(_elapsed) => Err(Refusal::Unavailable(Detail {
            code: Code::DeadlineExceeded,
            refined: None,
            message: "no response headers before the open deadline".into(),
        })),
    }
}

/// Runs one write under a stable submission identity. Only an unknown outcome is retransmitted,
/// and only with the same identity, so Cloud replays the recorded response instead of applying
/// the effect twice; every other verdict is final for this identity.
pub(super) async fn write<T, F, Fut>(mut attempt: F) -> Result<T, Verdict>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Attempt<T>>,
{
    let submission = uuid::Uuid::new_v4().to_string();
    let mut outcome = Verdict::Unknown(Detail {
        code: Code::Unknown,
        refined: None,
        message: "no attempt was made".into(),
    });
    for round in 0..WRITE_ATTEMPTS {
        if round > 0 {
            tokio::time::sleep(RETRY_DELAY).await;
        }
        outcome = match tokio::time::timeout(RPC_TIMEOUT, attempt(submission.clone())).await {
            Ok(Ok(response)) => return Ok(response.into_inner()),
            Ok(Err(status)) => match classify(&status) {
                Verdict::Unknown(detail) => Verdict::Unknown(detail),
                verdict => return Err(verdict),
            },
            Err(_elapsed) => Verdict::Unknown(Detail {
                code: Code::DeadlineExceeded,
                refined: None,
                message: "no reply before the write deadline".into(),
            }),
        };
    }
    Err(outcome)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Encodes a status the way Cloud's server does: code, message and one `ErrorDetail`.
    fn cloud_status(code: Code, message: &str, refined: ErrorCode) -> Status {
        let rpc = RpcStatus {
            code: code as i32,
            message: message.into(),
            details: vec![prost_types::Any {
                type_url: "type.googleapis.com/ora.cloud.internal.v1.ErrorDetail".into(),
                value: ErrorDetail {
                    code: refined as i32,
                }
                .encode_to_vec(),
            }],
        };
        Status::with_details(code, message, rpc.encode_to_vec().into())
    }

    /// The status code decides the class; the attached detail is carried into the message.
    #[test]
    fn statuses_map_to_contract_classes() {
        assert_eq!(
            classify(&cloud_status(
                Code::Aborted,
                "submission_conflict",
                ErrorCode::Conflict
            )),
            Verdict::Conflict
        );
        assert_eq!(
            classify(&cloud_status(
                Code::NotFound,
                "not_found",
                ErrorCode::NotFound
            )),
            Verdict::NotFound
        );
        let stale = classify(&cloud_status(
            Code::FailedPrecondition,
            "stale_controller",
            ErrorCode::StaleController,
        ));
        let Verdict::Stale(detail) = &stale else {
            panic!("expected stale, got {stale:?}");
        };
        assert_eq!(
            detail.to_string(),
            "ERROR_CODE_STALE_CONTROLLER (FailedPrecondition): stale_controller"
        );
        assert!(matches!(
            classify(&Status::unavailable("persistence unavailable")),
            Verdict::Unavailable(_)
        ));
        assert!(matches!(
            classify(&Status::unauthenticated("deployment mismatch")),
            Verdict::Unavailable(_)
        ));
        assert!(matches!(
            classify(&Status::deadline_exceeded("timeout")),
            Verdict::Unknown(_)
        ));
        assert_eq!(
            classify(&Status::unknown("transport error")).to_string_for_test(),
            "Unknown: transport error"
        );
    }

    impl Verdict {
        /// Renders the carried detail so tests can pin the message shape without matching structure.
        fn to_string_for_test(&self) -> String {
            match self {
                Self::NotFound | Self::Conflict => format!("{self:?}"),
                Self::Stale(detail) | Self::Unavailable(detail) | Self::Unknown(detail) => {
                    detail.to_string()
                }
            }
        }
    }

    /// Only `UNAVAILABLE` and a missing reply mean no Cloud serves the stream yet; every other
    /// status is a verdict from a serving Cloud, a stale epoch included.
    #[tokio::test]
    async fn opening_separates_unavailable_from_refusals() {
        let unavailable = open::<()>(async { Err(Status::unavailable("draining")) }).await;
        assert_eq!(
            unavailable,
            Err(Refusal::Unavailable(Detail {
                code: Code::Unavailable,
                refined: None,
                message: "draining".into(),
            }))
        );
        let unimplemented = open::<()>(async { Err(Status::unimplemented("no signals")) }).await;
        assert!(matches!(
            unimplemented,
            Err(Refusal::Verdict(Verdict::Unavailable(_)))
        ));
        let stale = open::<()>(async {
            Err(cloud_status(
                Code::FailedPrecondition,
                "stale_controller",
                ErrorCode::StaleController,
            ))
        })
        .await;
        assert!(matches!(stale, Err(Refusal::Verdict(Verdict::Stale(_)))));
        assert_eq!(open(async { Ok(tonic::Response::new(7)) }).await, Ok(7));
    }

    /// A stream that never answers is unavailable once the deadline passes, never unknown.
    #[tokio::test(start_paused = true)]
    async fn opening_without_headers_times_out_as_unavailable() {
        let pending = open::<()>(std::future::pending()).await;
        assert!(matches!(
            pending,
            Err(Refusal::Unavailable(Detail {
                code: Code::DeadlineExceeded,
                ..
            }))
        ));
    }

    /// Unknown outcomes are retried with the same submission identity; final verdicts are not.
    #[tokio::test]
    async fn writes_retransmit_only_unknown_outcomes_under_one_identity() {
        let mut seen = Vec::new();
        let result: Result<(), Verdict> = write(|submission| {
            seen.push(submission);
            let round = seen.len();
            async move {
                if round < 3 {
                    Err(Status::deadline_exceeded("lost"))
                } else {
                    Ok(tonic::Response::new(()))
                }
            }
        })
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(seen.len(), 3);
        assert!(seen.iter().all(|submission| submission == &seen[0]));

        let mut rounds = 0;
        let result: Result<(), Verdict> = write(|_| {
            rounds += 1;
            async { Err(Status::aborted("submission_conflict")) }
        })
        .await;
        assert_eq!(result, Err(Verdict::Conflict));
        assert_eq!(rounds, 1);

        let mut rounds = 0;
        let result: Result<(), Verdict> = write(|_| {
            rounds += 1;
            async { Err(Status::unknown("transport error")) }
        })
        .await;
        assert!(matches!(result, Err(Verdict::Unknown(_))));
        assert_eq!(rounds, WRITE_ATTEMPTS);
    }
}
