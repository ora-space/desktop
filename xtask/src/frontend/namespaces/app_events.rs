//! Endpoint declarations for the application-event stream namespace.

use crate::frontend::{FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS};
use ora_contracts::APP_EVENT_WATCH_PATH;

const NAMESPACE: &str = "appEvents";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "watchAppEvents",
    namespace: NAMESPACE,
    member_name: "watch",
    method: FrontendHttpMethod::Post,
    path_template: APP_EVENT_WATCH_PATH,
    request_type: "WatchAppEventsRequest",
    response_type: "AppEvent",
    path_params: NO_PATH_PARAMS,
    has_json_body: true,
}];
