use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Stdio policy used when spawning a child process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProcessStdio {
    /// Create an owned async pipe that callers can take from the managed process.
    #[default]
    Piped,
    /// Inherit the corresponding stdio stream from the parent process.
    Inherit,
    /// Connect the corresponding stdio stream to the platform null device.
    Null,
}

/// Controls whether a child can inherit ambient Host environment variables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EnvironmentPolicy {
    /// Preserve the ordinary process-spawner behavior and apply explicit overrides.
    #[default]
    InheritAndOverride,
    /// Clear the environment before applying the explicit Host-owned allowlist.
    ClearAndAllowlist,
}

impl ProcessStdio {
    pub(crate) fn as_stdio(self) -> Stdio {
        match self {
            Self::Piped => Stdio::piped(),
            Self::Inherit => Stdio::inherit(),
            Self::Null => Stdio::null(),
        }
    }
}

/// Spawn configuration for one OS child process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    program: OsString,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    envs: Vec<(OsString, OsString)>,
    environment_policy: EnvironmentPolicy,
    stdin: ProcessStdio,
    stdout: ProcessStdio,
    stderr: ProcessStdio,
    kill_on_drop: bool,
}

impl ProcessSpec {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            envs: Vec::new(),
            environment_policy: EnvironmentPolicy::InheritAndOverride,
            stdin: ProcessStdio::Piped,
            stdout: ProcessStdio::Piped,
            stderr: ProcessStdio::Piped,
            kill_on_drop: true,
        }
    }

    /// Appends one argument to the child process command line.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Appends multiple arguments to the child process command line.
    pub fn args<Args, Arg>(mut self, args: Args) -> Self
    where
        Args: IntoIterator<Item = Arg>,
        Arg: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets the working directory for the child process.
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Adds or overrides one environment variable for the child process.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Clears ambient inheritance so only explicit environment bindings reach the child.
    pub fn clear_and_allowlist_environment(mut self) -> Self {
        self.environment_policy = EnvironmentPolicy::ClearAndAllowlist;
        self
    }

    /// Preserves ambient inheritance for ordinary non-plugin leaf processes.
    pub fn inherit_and_override_environment(mut self) -> Self {
        self.environment_policy = EnvironmentPolicy::InheritAndOverride;
        self
    }

    /// Sets the stdin policy for the child process.
    pub fn stdin(mut self, stdin: ProcessStdio) -> Self {
        self.stdin = stdin;
        self
    }

    /// Sets the stdout policy for the child process.
    pub fn stdout(mut self, stdout: ProcessStdio) -> Self {
        self.stdout = stdout;
        self
    }

    /// Sets the stderr policy for the child process.
    pub fn stderr(mut self, stderr: ProcessStdio) -> Self {
        self.stderr = stderr;
        self
    }

    /// Configures the child process to be killed when the managed handle is dropped.
    pub fn kill_on_drop(mut self) -> Self {
        self.kill_on_drop = true;
        self
    }

    /// Configures the child process to keep running when the managed handle is dropped.
    pub fn keep_alive_on_drop(mut self) -> Self {
        self.kill_on_drop = false;
        self
    }

    /// Returns the executable path or name that will be passed to the OS.
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// Returns the configured command-line arguments in insertion order.
    pub fn args_iter(&self) -> impl Iterator<Item = &OsStr> {
        self.args.iter().map(OsString::as_os_str)
    }

    /// Returns the configured working directory, if one was set.
    pub fn cwd_path(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    /// Returns the configured environment overrides in insertion order.
    pub fn envs(&self) -> impl Iterator<Item = (&OsStr, &OsStr)> {
        self.envs
            .iter()
            .map(|(key, value)| (key.as_os_str(), value.as_os_str()))
    }

    /// Returns the explicit ambient-environment policy.
    pub fn environment_policy(&self) -> EnvironmentPolicy {
        self.environment_policy
    }

    /// Returns the configured stdin policy.
    pub fn stdin_policy(&self) -> ProcessStdio {
        self.stdin
    }

    /// Returns the configured stdout policy.
    pub fn stdout_policy(&self) -> ProcessStdio {
        self.stdout
    }

    /// Returns the configured stderr policy.
    pub fn stderr_policy(&self) -> ProcessStdio {
        self.stderr
    }

    /// Returns whether the child process should be killed when the handle is dropped.
    pub fn should_kill_on_drop(&self) -> bool {
        self.kill_on_drop
    }
}
