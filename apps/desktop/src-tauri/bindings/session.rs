//! Desktop bindings for session.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "startSession",
        handler: "commands::session::start_session",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "setSessionConfig",
        handler: "commands::session::set_session_config",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getSession",
        handler: "commands::session::get_session",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listSessions",
        handler: "commands::session::list_sessions",
        permission: Permission::MainWebview,
    },
    Binding::Stream {
        operation: "loadSession",
        handler: "commands::session::start_load",
    },
    Binding::Stream {
        operation: "promptSession",
        handler: "commands::session::start_prompt",
    },
    Binding::Unary {
        operation: "respondToSessionPermission",
        handler: "commands::session::respond_to_session_permission",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "cancelSessionPrompt",
        handler: "commands::session::cancel_session_prompt",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "stopSession",
        handler: "commands::session::stop_session",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "switchSessionAgent",
        handler: "commands::session::switch_session_agent",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "resumeSessionHistory",
        handler: "commands::session::resume_session_history",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteSession",
        handler: "commands::session::delete_session",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "renameSession",
        handler: "commands::session::rename_session",
        permission: Permission::MainWebview,
    },
];
