use crate::{Error, ProcessConfig};
use gitlancer::{GitCommand, GitEnv};
use ora_utils::path::{CanonicalPathRoot, TrustedPathKind, open_trusted_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Deployment provides non-secret configuration references, never an arbitrary environment map.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloneConfig {
    pub repository_root: PathBuf,
    pub git_config: PathBuf,
    pub search_path: Vec<PathBuf>,
    pub ssh: CloneSsh,
}

/// HTTPS-only deployments need not install SSH identities or a client configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CloneSsh {
    Disabled,
    Configured { program: PathBuf, config: PathBuf },
}

impl CloneConfig {
    /// Freezes an existing trusted root and rejects overlap with every protected state/checkout root.
    pub(super) fn validate(
        &mut self,
        protected: &[PathBuf],
        process: &ProcessConfig,
    ) -> Result<(), Error> {
        // SAFETY: identity inspection does not alter process credentials.
        let uid = unsafe { libc::geteuid() };
        open_trusted_path(&self.repository_root, uid, TrustedPathKind::Directory)
            .map_err(|e| Error::Configuration(e.to_string()))?;
        self.repository_root = CanonicalPathRoot::new(&self.repository_root)
            .map_err(|e| Error::Configuration(e.to_string()))?
            .as_path()
            .to_owned();
        for path in protected
            .iter()
            .chain(std::iter::once(&process.host_directory))
        {
            let path = ora_utils::path::canonicalize_longest_existing_prefix(path);
            if path.starts_with(&self.repository_root) || self.repository_root.starts_with(path) {
                return Err(Error::Configuration(
                    "repository root overlaps protected state or checkout".into(),
                ));
            }
        }
        open_trusted_path(&self.git_config, uid, TrustedPathKind::File)
            .map_err(|e| Error::Configuration(e.to_string()))?;
        if self.search_path.is_empty() || self.search_path.iter().any(|p| !p.is_absolute()) {
            return Err(Error::Configuration(
                "clone search path must contain explicit absolute directories".into(),
            ));
        }
        for path in &self.search_path {
            open_trusted_path(path, uid, TrustedPathKind::Directory)
                .map_err(|e| Error::Configuration(e.to_string()))?;
        }
        if let CloneSsh::Configured { program, config } = &self.ssh {
            for path in [program, config] {
                open_trusted_path(path, uid, TrustedPathKind::File)
                    .map_err(|e| Error::Configuration(e.to_string()))?;
            }
        }
        self.environment()?;
        Ok(())
    }

    /// Uses only references to deployment configuration; secrets stay outside RunSpec and journals.
    pub(super) fn environment(&self) -> Result<GitEnv, Error> {
        let utf8 = |path: &Path| {
            path.to_str().map(str::to_owned).ok_or_else(|| {
                Error::Configuration("clone configuration paths must be UTF-8".into())
            })
        };
        let search_path = std::env::join_paths(&self.search_path)
            .map_err(|e| Error::Configuration(e.to_string()))?
            .into_string()
            .map_err(|_| Error::Configuration("clone search path must be UTF-8".into()))?;
        let mut env = GitEnv::default()
            .with_variable("PATH", search_path)
            .with_variable("GIT_CONFIG_NOSYSTEM", "1")
            .with_variable("GIT_CONFIG_GLOBAL", utf8(&self.git_config)?)
            .with_variable("GIT_TERMINAL_PROMPT", "0")
            .with_variable("GIT_ASKPASS", "")
            .with_variable("SSH_ASKPASS", "")
            .with_variable("GIT_LFS_SKIP_SMUDGE", "1")
            .with_variable("GIT_OPTIONAL_LOCKS", "0");
        if let CloneSsh::Configured { program, config } = &self.ssh {
            let parts = [
                utf8(program)?,
                "-F".into(),
                utf8(config)?,
                "-oBatchMode=yes".into(),
                "-oStrictHostKeyChecking=yes".into(),
            ];
            let command = shlex::try_join(parts.iter().map(String::as_str))
                .map_err(|e| Error::Configuration(e.to_string()))?;
            env = env
                .with_variable("GIT_SSH_COMMAND", command)
                .with_variable("GIT_SSH_VARIANT", "ssh");
        }
        Ok(env)
    }

    /// Applies fixed per-command policies without changing any user's repository configuration.
    pub(super) fn constrain(&self, command: &mut GitCommand) {
        let ssh = match self.ssh {
            CloneSsh::Disabled => "protocol.ssh.allow=never",
            CloneSsh::Configured { .. } => "protocol.ssh.allow=always",
        };
        let mut prefix = Vec::new();
        for config in [
            "protocol.allow=never",
            "protocol.https.allow=always",
            ssh,
            "core.hooksPath=/dev/null",
            "core.askPass=",
            "submodule.recurse=false",
            "filter.lfs.required=false",
            "filter.lfs.smudge=",
            "filter.lfs.process=",
        ] {
            prefix.extend(["-c".into(), config.into()]);
        }
        prefix.append(&mut command.args);
        command.args = prefix;
    }
}
