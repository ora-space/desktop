//! The local interface between `ora-git` in an Agent sandbox and the Node (Node clone ADR D5).
//!
//! `ora-git` never runs Git against the network: it turns one of a fixed set of verbs into a
//! one-line JSON request on the session's Unix socket and prints what the Node answers. The
//! request carries no execution, Workspace or repository: the Node knows them from the socket the
//! request arrived on. Both sides validate the arguments with the same rules here.

use ora_node_protocol::{is_canonical_uuid, is_workspace_branch};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

/// Environment variable holding the session's socket path.
pub const SOCKET_ENV: &str = "ORA_GIT_SOCKET";
/// Largest request line, newline included.
pub const MAX_REQUEST_BYTES: usize = 4 * 1024;

/// One request; there is no way to name options, refspecs, URLs or Git arguments.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "verb", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Info {},
    List {},
    FetchUpstream {},
    FetchWorkspace {
        workspace_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
    },
    Push {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
    },
}

impl Request {
    /// Parses the command line after the program name; anything else is a usage error.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let request = match words.as_slice() {
            ["info"] => Self::Info {},
            ["list"] => Self::List {},
            ["fetch", "upstream"] => Self::FetchUpstream {},
            ["fetch", "workspace", workspace] => Self::FetchWorkspace {
                workspace_id: (*workspace).to_owned(),
                branch: None,
            },
            ["fetch", "workspace", workspace, branch] => Self::FetchWorkspace {
                workspace_id: (*workspace).to_owned(),
                branch: Some((*branch).to_owned()),
            },
            ["push"] => Self::Push { branch: None },
            ["push", branch] => Self::Push {
                branch: Some((*branch).to_owned()),
            },
            _ => return Err(USAGE.to_owned()),
        };
        request.validate()?;
        Ok(request)
    }

    /// Checks the arguments a verb carries; the Node runs the same check on what it receives.
    pub fn validate(&self) -> Result<(), String> {
        let (workspace, branch) = match self {
            Self::Info {} | Self::List {} | Self::FetchUpstream {} => (None, None),
            Self::FetchWorkspace {
                workspace_id,
                branch,
            } => (Some(workspace_id), branch.as_ref()),
            Self::Push { branch } => (None, branch.as_ref()),
        };
        if let Some(workspace) = workspace
            && !is_canonical_uuid(workspace)
        {
            return Err("a Workspace ID is a lowercase UUID".to_owned());
        }
        if let Some(branch) = branch
            && !is_workspace_branch(branch)
        {
            return Err(
                "a branch is a plain branch name: no HEAD, no leading '-', no ':', '+' or '*', at most 200 bytes"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

/// Runs the `ora-git` command line (the arguments after the program name): validates it, sends
/// it on the session's socket, prints the result on stdout or `ora-git: <code>: <hint>` on stderr,
/// and returns the stable exit code. The binary is only this call, so a test fixture that calls it
/// runs exactly what an Agent runs.
pub fn run(args: &[String]) -> ExitCode {
    let result = Request::parse(args)
        .map_err(|message| (ErrorCode::Usage, message))
        .and_then(|request| {
            let socket = std::env::var_os(SOCKET_ENV).ok_or_else(|| {
                (
                    ErrorCode::NoSession,
                    format!("{SOCKET_ENV} is not set; ora-git works only inside an Agent session"),
                )
            })?;
            send(Path::new(&socket), &request, &mut std::io::stderr())
        });
    match result {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err((code, message)) => {
            eprintln!("ora-git: {}: {message}", code.as_str());
            ExitCode::from(u8::try_from(code.exit_code()).unwrap_or(1))
        }
    }
}

/// Sends `request` on the session socket at `socket`, writes the Node's progress lines to
/// `progress`, and returns the result.
///
/// The Node closes a session's socket when the session ends, even under a request it has not
/// answered yet; a connection that ends without an answer therefore means the session is gone.
#[cfg(unix)]
pub fn send(
    socket: &Path,
    request: &Request,
    progress: &mut impl Write,
) -> Result<String, (ErrorCode, String)> {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    let closed = || {
        (
            ErrorCode::NoSession,
            "the Node closed the session socket; the session is no longer running".to_owned(),
        )
    };
    let mut stream = UnixStream::connect(socket).map_err(|_| {
        (
            ErrorCode::NoSession,
            "the session's socket is gone; the session is no longer running".to_owned(),
        )
    })?;
    let mut line = serde_json::to_vec(request).map_err(|_| {
        (
            ErrorCode::Internal,
            "request could not be encoded".to_owned(),
        )
    })?;
    line.push(b'\n');
    if line.len() > MAX_REQUEST_BYTES {
        return Err((ErrorCode::Usage, "request is too long".to_owned()));
    }
    stream.write_all(&line).map_err(|_| closed())?;
    for line in BufReader::new(stream).lines() {
        let line = line.map_err(|_| closed())?;
        match serde_json::from_str::<Reply>(&line) {
            Ok(Reply::Progress { message }) => {
                let _ = writeln!(progress, "{message}");
            }
            Ok(Reply::Done { output }) => return Ok(output),
            Ok(Reply::Failed { code, message }) => return Err((code, message)),
            Err(_) => {
                return Err((
                    ErrorCode::Internal,
                    "the Node's answer was not understood".to_owned(),
                ));
            }
        }
    }
    Err(closed())
}

/// The session socket is a Unix socket and the Node serves it only in Linux Agent sandboxes, so a
/// build for another platform can only say that it cannot reach a Node; the request types stay
/// available because the Node's protocol crates compile everywhere.
#[cfg(not(unix))]
pub fn send(
    _socket: &Path,
    _request: &Request,
    _progress: &mut impl Write,
) -> Result<String, (ErrorCode, String)> {
    Err((
        ErrorCode::Internal,
        "ora-git needs the session's Unix socket; it runs only inside a Linux Agent sandbox"
            .to_owned(),
    ))
}

/// The usage summary printed for any malformed command line.
pub const USAGE: &str = "usage: ora-git info | list | fetch upstream | fetch workspace <workspaceId> [<branch>] | push [<branch>]";

/// One line of the Node's answer: any number of progress lines, then exactly one result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Progress { message: String },
    Done { output: String },
    Failed { code: ErrorCode, message: String },
}

/// The stable error codes `ora-git` prints and exits with.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Usage,
    NoSession,
    Forbidden,
    Busy,
    NotFound,
    Conflict,
    NotFastForward,
    Denied,
    Unavailable,
    Internal,
}

impl ErrorCode {
    /// The process exit code of this error.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Usage => 2,
            Self::NoSession => 3,
            Self::Forbidden => 4,
            Self::Busy => 5,
            Self::NotFound => 6,
            Self::Conflict => 7,
            Self::NotFastForward => 8,
            Self::Denied => 9,
            Self::Unavailable => 10,
            Self::Internal => 11,
        }
    }

    /// The code as printed in `ora-git: <code>: <hint>`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::NoSession => "no_session",
            Self::Forbidden => "forbidden",
            Self::Busy => "busy",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::NotFastForward => "not_fast_forward",
            Self::Denied => "denied",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const WORKSPACE: &str = "4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a";

    /// Parses a command line given as words.
    fn parse(words: &[&str]) -> Result<Request, String> {
        Request::parse(
            &words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>(),
        )
    }

    /// Every verb parses to its request.
    #[test]
    fn fixed_verbs_parse() {
        assert_eq!(
            [
                parse(&["info"]),
                parse(&["list"]),
                parse(&["fetch", "upstream"]),
                parse(&["fetch", "workspace", WORKSPACE]),
                parse(&["fetch", "workspace", WORKSPACE, "feature/x"]),
                parse(&["push"]),
                parse(&["push", "feature/x"]),
            ],
            [
                Ok(Request::Info {}),
                Ok(Request::List {}),
                Ok(Request::FetchUpstream {}),
                Ok(Request::FetchWorkspace {
                    workspace_id: WORKSPACE.to_owned(),
                    branch: None
                }),
                Ok(Request::FetchWorkspace {
                    workspace_id: WORKSPACE.to_owned(),
                    branch: Some("feature/x".to_owned())
                }),
                Ok(Request::Push { branch: None }),
                Ok(Request::Push {
                    branch: Some("feature/x".to_owned())
                }),
            ]
        );
    }

    /// Options, refspecs, extra words and malformed identities are usage errors.
    #[test]
    fn anything_else_is_a_usage_error() {
        let long = "b".repeat(201);
        for words in [
            vec![],
            vec!["status"],
            vec!["push", "--force"],
            vec!["push", "main:refs/heads/workspaces/x/main"],
            vec!["push", "+main"],
            vec!["push", "HEAD"],
            vec!["push", "a*"],
            vec!["push", &long],
            vec!["push", "main", "extra"],
            vec!["info", "--verbose"],
            vec!["fetch"],
            vec!["fetch", "origin"],
            vec!["fetch", "workspace"],
            vec!["fetch", "workspace", "workspace-1"],
            vec!["fetch", "workspace", &WORKSPACE.to_uppercase()],
            vec!["fetch", "workspace", WORKSPACE, "main", "extra"],
        ] {
            assert!(parse(&words).is_err(), "{words:?} accepted");
        }
    }

    /// Reads one request line from a connection accepted on `listener`, then lets `answer`
    /// finish the connection.
    #[cfg(unix)]
    fn serve_once(
        listener: std::os::unix::net::UnixListener,
        answer: impl FnOnce(std::os::unix::net::UnixStream) + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        use std::io::{BufRead, BufReader};

        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap_or_else(|_| panic!("accept"));
            let mut request = String::new();
            let _ = BufReader::new(&stream).read_line(&mut request);
            answer(stream);
        })
    }

    /// Progress is relayed and the result returned; a socket that closes before answering, or
    /// that is gone, means the session ended.
    #[cfg(unix)]
    #[test]
    fn answers_and_closed_sockets_map_to_results() {
        let directory = std::env::temp_dir().join(format!("ora-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap_or_else(|_| panic!("directory"));
        let socket = directory.join("s.sock");
        let request = Request::Info {};

        let listener =
            std::os::unix::net::UnixListener::bind(&socket).unwrap_or_else(|_| panic!("bind"));
        let server = serve_once(listener, |mut stream| {
            let _ = stream.write_all(
                b"{\"kind\":\"progress\",\"message\":\"working\"}\n{\"kind\":\"done\",\"output\":\"ok\\n\"}\n",
            );
        });
        let mut progress = Vec::new();
        let answered = send(&socket, &request, &mut progress);
        let _ = server.join();
        let _ = std::fs::remove_file(&socket);

        let listener =
            std::os::unix::net::UnixListener::bind(&socket).unwrap_or_else(|_| panic!("bind"));
        let server = serve_once(listener, drop);
        let cut_off = send(&socket, &request, &mut Vec::new()).map_err(|(code, _)| code);
        let _ = server.join();
        let _ = std::fs::remove_file(&socket);
        let gone = send(&socket, &request, &mut Vec::new()).map_err(|(code, _)| code);
        let _ = std::fs::remove_dir_all(&directory);

        assert_eq!(
            (
                answered,
                String::from_utf8_lossy(&progress).into_owned(),
                cut_off,
                gone
            ),
            (
                Ok("ok\n".to_owned()),
                "working\n".to_owned(),
                Err(ErrorCode::NoSession),
                Err(ErrorCode::NoSession)
            )
        );
    }

    /// The wire shapes and exit codes are stable.
    #[test]
    fn wire_and_exit_codes_are_stable() {
        assert_eq!(
            serde_json::to_string(&Request::Push { branch: None }).unwrap_or_default(),
            r#"{"verb":"push"}"#
        );
        assert_eq!(
            serde_json::to_string(&Reply::Failed {
                code: ErrorCode::NotFastForward,
                message: "m".to_owned()
            })
            .unwrap_or_default(),
            r#"{"kind":"failed","code":"not_fast_forward","message":"m"}"#
        );
        assert_eq!(
            [
                ErrorCode::Usage,
                ErrorCode::NoSession,
                ErrorCode::Forbidden,
                ErrorCode::Busy,
                ErrorCode::NotFound,
                ErrorCode::Conflict,
                ErrorCode::NotFastForward,
                ErrorCode::Denied,
                ErrorCode::Unavailable,
                ErrorCode::Internal,
            ]
            .map(ErrorCode::exit_code),
            [2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        );
        assert!(
            serde_json::from_str::<Request>(r#"{"verb":"push","refspec":"x"}"#).is_err(),
            "unknown request fields must be refused"
        );
    }
}
