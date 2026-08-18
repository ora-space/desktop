//! Endpoint declarations for the application-event stream namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "appEvents";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "watchAppEvents",
    namespace: NAMESPACE,
    member_name: "watch",
    request_type: "WatchAppEventsRequest",
    response_type: "AppEvent",
}];
