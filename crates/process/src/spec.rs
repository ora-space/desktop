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
    removed_env_prefixes: Vec<String>,
    stdin: ProcessStdio,
    stdout: ProcessStdio,
    stderr: ProcessStdio,
    kill_on_drop: bool,
    reaper_registration: ReaperRegistration,
}

/// Records whether a spawned process participates in process-global crash cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReaperRegistration {
    Register,
    Skip,
}

impl ProcessSpec {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            envs: Vec::new(),
            removed_env_prefixes: Vec::new(),
            stdin: ProcessStdio::Piped,
            stdout: ProcessStdio::Piped,
            stderr: ProcessStdio::Piped,
            kill_on_drop: true,
            reaper_registration: ReaperRegistration::Register,
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

    /// Removes inherited variables matching a case-insensitive prefix before explicit overrides.
    pub fn remove_env_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.removed_env_prefixes
            .push(prefix.into().to_ascii_uppercase());
        self
    }

    /// Reports whether an inherited variable is excluded by the caller's environment policy.
    pub fn excludes_inherited_env(&self, key: &OsStr) -> bool {
        let key = key.to_string_lossy().to_ascii_uppercase();
        self.removed_env_prefixes
            .iter()
            .any(|prefix| key.starts_with(prefix))
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

    /// Skips process-global reaper registration for a bounded, one-shot child process.
    ///
    /// The process remains locally managed and can still be waited on or killed tree-wide. Use
    /// this only when the caller owns the complete short-lived lifecycle and does not need the
    /// sidecar to clean up the process after an abnormal Ora exit.
    pub fn skip_reaper_registration(mut self) -> Self {
        self.reaper_registration = ReaperRegistration::Skip;
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

    /// Returns the typed reaper policy used by the production lifecycle implementation.
    pub(crate) fn reaper_registration(&self) -> ReaperRegistration {
        self.reaper_registration
    }
}

#[cfg(test)]
mod tests {
    use super::{ProcessSpec, ReaperRegistration};
    use pretty_assertions::assert_eq;

    /// Verifies crash cleanup remains the default and one-shot callers can explicitly opt out.
    #[test]
    fn models_reaper_registration_as_an_explicit_policy() {
        assert_eq!(
            ProcessSpec::new("agent").reaper_registration(),
            ReaperRegistration::Register
        );
        assert_eq!(
            ProcessSpec::new("rg")
                .skip_reaper_registration()
                .reaper_registration(),
            ReaperRegistration::Skip
        );
    }
}
