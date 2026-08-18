//! Endpoint declarations for the session generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "session";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "warmSession",
        namespace: NAMESPACE,
        member_name: "warm",
        request_type: "WarmSessionRequest",
        response_type: "WarmSessionResponse",
    },
    FrontendEndpoint {
        operation_name: "setSessionConfig",
        namespace: NAMESPACE,
        member_name: "setConfig",
        request_type: "SetSessionConfigRequest",
        response_type: "SetSessionConfigResponse",
    },
    FrontendEndpoint {
        operation_name: "attachSession",
        namespace: NAMESPACE,
        member_name: "attach",
        request_type: "AttachSessionRequest",
        response_type: "AttachSessionResponse",
    },
    FrontendEndpoint {
        operation_name: "getSession",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetSessionRequest",
        response_type: "GetSessionResponse",
    },
    FrontendEndpoint {
        operation_name: "listSessions",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListSessionsRequest",
        response_type: "ListSessionsResponse",
    },
    FrontendEndpoint {
        operation_name: "loadSession",
        namespace: NAMESPACE,
        member_name: "load",
        request_type: "LoadSessionRequest",
        response_type: "LoadSessionEvent",
    },
    FrontendEndpoint {
        operation_name: "promptSession",
        namespace: NAMESPACE,
        member_name: "prompt",
        request_type: "PromptSessionRequest",
        response_type: "PromptSessionEvent",
    },
    FrontendEndpoint {
        operation_name: "respondToSessionPermission",
        namespace: NAMESPACE,
        member_name: "respondToPermission",
        request_type: "RespondToPermissionRequest",
        response_type: "RespondToPermissionResponse",
    },
    FrontendEndpoint {
        operation_name: "stopSession",
        namespace: NAMESPACE,
        member_name: "stop",
        request_type: "StopSessionRequest",
        response_type: "StopSessionResponse",
    },
    FrontendEndpoint {
        operation_name: "switchSessionAgent",
        namespace: NAMESPACE,
        member_name: "switchAgent",
        request_type: "SwitchSessionAgentRequest",
        response_type: "SwitchSessionAgentResponse",
    },
    FrontendEndpoint {
        operation_name: "resumeSessionHistory",
        namespace: NAMESPACE,
        member_name: "resumeHistory",
        request_type: "ResumeSessionHistoryRequest",
        response_type: "ResumeSessionHistoryResponse",
    },
    FrontendEndpoint {
        operation_name: "deleteSession",
        namespace: NAMESPACE,
        member_name: "delete",
        request_type: "DeleteSessionRequest",
        response_type: "DeleteSessionResponse",
    },
    FrontendEndpoint {
        operation_name: "renameSession",
        namespace: NAMESPACE,
        member_name: "rename",
        request_type: "RenameSessionRequest",
        response_type: "RenameSessionResponse",
    },
];
