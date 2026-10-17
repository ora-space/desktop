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
}

impl CloneConfig {
    /// Cloud clones reach only the Git Server, with the Node's git-access identity on the command
    /// line; the workload configuration may hold nothing but an HTTPS CA reference, so no
    /// deployment credential or helper can enter the workload's Git.
    pub(crate) fn validate_cloud_policy(&self) -> Result<(), Error> {
        let content = std::fs::read_to_string(&self.git_config)
            .map_err(|e| Error::Configuration(e.to_string()))?;
        let mut http = false;
        for line in content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with(';'))
        {
            if line.eq_ignore_ascii_case("[http]") {
                http = true;
                continue;
            }
            if !http
                || !line.split_once('=').is_some_and(|(key, value)| {
                    key.trim().eq_ignore_ascii_case("sslCAInfo") && !value.trim().is_empty()
                })
            {
                return Err(Error::Configuration(
                    "Cloud Git configuration permits only an HTTPS CA reference".into(),
                ));
            }
        }
        Ok(())
    }
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
        Ok(GitEnv::default()
            .with_variable("PATH", search_path)
            .with_variable("GIT_CONFIG_NOSYSTEM", "1")
            .with_variable("GIT_CONFIG_GLOBAL", utf8(&self.git_config)?)
            .with_variable("GIT_TERMINAL_PROMPT", "0")
            .with_variable("GIT_ASKPASS", "")
            .with_variable("SSH_ASKPASS", "")
            .with_variable("GIT_LFS_SKIP_SMUDGE", "1")
            .with_variable("GIT_OPTIONAL_LOCKS", "0"))
    }

    /// Applies fixed per-command policies without changing any user's repository configuration.
    ///
    /// SSH is never allowed: clones come only from the Git Server over HTTPS. The one workload
    /// command that reads a transport repository allows `file` for itself alone.
    pub(super) fn constrain(&self, command: &mut GitCommand) {
        let mut prefix = Vec::new();
        for config in [
            "protocol.allow=never",
            "protocol.https.allow=always",
            "protocol.ssh.allow=never",
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
