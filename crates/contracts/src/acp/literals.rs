use serde::{Deserialize, Serialize};

/// Names of all methods that agents handle.
///
/// Provides a centralized definition of method names used in the protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentMethodNames {
    /// Method for initializing the connection.
    pub initialize: &'static str,
    /// Method for authenticating with the agent.
    pub authenticate: &'static str,
    /// Method name for protocol-level request cancellation notifications.
    pub cancel_request: &'static str,
    /// Method for creating a new session.
    pub session_new: &'static str,
    /// Method for loading an existing session.
    pub session_load: &'static str,
    /// Method for setting the mode for a session.
    pub session_set_mode: &'static str,
    /// Method for setting a configuration option for a session.
    pub session_set_config_option: &'static str,
    /// Method for sending a prompt to the agent.
    pub session_prompt: &'static str,
    /// Notification for cancelling operations.
    pub session_cancel: &'static str,
    /// Method for listing existing sessions.
    pub session_list: &'static str,
    /// Method for deleting an existing session.
    pub session_delete: &'static str,
    /// Method for resuming an existing session.
    pub session_resume: &'static str,
    /// Method for closing an active session.
    pub session_close: &'static str,
    /// Method for logging out of an authenticated session.
    pub logout: &'static str,
}

/// Constant containing all agent method names.
pub const AGENT_METHOD_NAMES: AgentMethodNames = AgentMethodNames {
    initialize: INITIALIZE_METHOD_NAME,
    authenticate: AUTHENTICATE_METHOD_NAME,
    cancel_request: CANCEL_REQUEST_METHOD_NAME,
    session_new: SESSION_NEW_METHOD_NAME,
    session_load: SESSION_LOAD_METHOD_NAME,
    session_set_mode: SESSION_SET_MODE_METHOD_NAME,
    session_set_config_option: SESSION_SET_CONFIG_OPTION_METHOD_NAME,
    session_prompt: SESSION_PROMPT_METHOD_NAME,
    session_cancel: SESSION_CANCEL_METHOD_NAME,
    session_list: SESSION_LIST_METHOD_NAME,
    session_delete: SESSION_DELETE_METHOD_NAME,
    session_resume: SESSION_RESUME_METHOD_NAME,
    session_close: SESSION_CLOSE_METHOD_NAME,
    logout: LOGOUT_METHOD_NAME,
};

/// Method name for the initialize request.
pub(crate) const INITIALIZE_METHOD_NAME: &str = "initialize";
/// Method name for the authenticate request.
pub(crate) const AUTHENTICATE_METHOD_NAME: &str = "authenticate";
/// Method name for general cancel notification
pub(crate) const CANCEL_REQUEST_METHOD_NAME: &str = "$/cancel_request";
/// Method name for creating a new session.
pub(crate) const SESSION_NEW_METHOD_NAME: &str = "session/new";
/// Method name for loading an existing session.
pub(crate) const SESSION_LOAD_METHOD_NAME: &str = "session/load";
/// Method name for setting the mode for a session.
pub(crate) const SESSION_SET_MODE_METHOD_NAME: &str = "session/set_mode";
/// Method name for setting a configuration option for a session.
pub(crate) const SESSION_SET_CONFIG_OPTION_METHOD_NAME: &str = "session/set_config_option";
/// Method name for sending a prompt.
pub(crate) const SESSION_PROMPT_METHOD_NAME: &str = "session/prompt";
/// Method name for the cancel notification.
pub(crate) const SESSION_CANCEL_METHOD_NAME: &str = "session/cancel";
/// Method name for listing existing sessions.
pub(crate) const SESSION_LIST_METHOD_NAME: &str = "session/list";
/// Method name for deleting an existing session.
pub(crate) const SESSION_DELETE_METHOD_NAME: &str = "session/delete";
/// Method name for resuming an existing session.
pub(crate) const SESSION_RESUME_METHOD_NAME: &str = "session/resume";
/// Method name for closing an active session.
pub(crate) const SESSION_CLOSE_METHOD_NAME: &str = "session/close";
/// Method name for logging out of an authenticated session.
pub(crate) const LOGOUT_METHOD_NAME: &str = "logout";

/// Names of all methods that clients handle.
///
/// Provides a centralized definition of method names used in the protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientMethodNames {
    /// Method for requesting permission from the user.
    pub session_request_permission: &'static str,
    /// Notification for session updates.
    pub session_update: &'static str,
    /// Method for writing text files.
    pub fs_write_text_file: &'static str,
    /// Method for reading text files.
    pub fs_read_text_file: &'static str,
    /// Method for creating new terminals.
    pub terminal_create: &'static str,
    /// Method for getting terminals output.
    pub terminal_output: &'static str,
    /// Method for releasing a terminal.
    pub terminal_release: &'static str,
    /// Method for waiting for a terminal to finish.
    pub terminal_wait_for_exit: &'static str,
    /// Method for killing a terminal.
    pub terminal_kill: &'static str,
}

/// Constant containing all client method names.
pub const CLIENT_METHOD_NAMES: ClientMethodNames = ClientMethodNames {
    session_update: SESSION_UPDATE_NOTIFICATION,
    session_request_permission: SESSION_REQUEST_PERMISSION_METHOD_NAME,
    fs_write_text_file: FS_WRITE_TEXT_FILE_METHOD_NAME,
    fs_read_text_file: FS_READ_TEXT_FILE_METHOD_NAME,
    terminal_create: TERMINAL_CREATE_METHOD_NAME,
    terminal_output: TERMINAL_OUTPUT_METHOD_NAME,
    terminal_release: TERMINAL_RELEASE_METHOD_NAME,
    terminal_wait_for_exit: TERMINAL_WAIT_FOR_EXIT_METHOD_NAME,
    terminal_kill: TERMINAL_KILL_METHOD_NAME,
};

/// Notification name for session updates.
pub(crate) const SESSION_UPDATE_NOTIFICATION: &str = "session/update";
/// Method name for requesting user permission.
pub(crate) const SESSION_REQUEST_PERMISSION_METHOD_NAME: &str = "session/request_permission";
/// Method name for writing text files.
pub(crate) const FS_WRITE_TEXT_FILE_METHOD_NAME: &str = "fs/write_text_file";
/// Method name for reading text files.
pub(crate) const FS_READ_TEXT_FILE_METHOD_NAME: &str = "fs/read_text_file";
/// Method name for creating a new terminal.
pub(crate) const TERMINAL_CREATE_METHOD_NAME: &str = "terminal/create";
/// Method for getting terminals output.
pub(crate) const TERMINAL_OUTPUT_METHOD_NAME: &str = "terminal/output";
/// Method for releasing a terminal.
pub(crate) const TERMINAL_RELEASE_METHOD_NAME: &str = "terminal/release";
/// Method for waiting for a terminal to finish.
pub(crate) const TERMINAL_WAIT_FOR_EXIT_METHOD_NAME: &str = "terminal/wait_for_exit";
/// Method for killing a terminal.
pub(crate) const TERMINAL_KILL_METHOD_NAME: &str = "terminal/kill";
