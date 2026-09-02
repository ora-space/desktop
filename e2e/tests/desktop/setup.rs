//! Shared setup and teardown for isolated Desktop E2E cases.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ora_backend::BackendPaths;
use ora_logging::{InitializedLogging, LogLevel, LogOutput, LoggingConfig, init_logging};
use tempfile::{TempDir, tempdir};

/// Holds the writer and level handles for the whole test binary, or why they could not be built.
///
/// The Backend expects its composition root to have installed the process clock and subscriber
/// before it runs, and its own logging resolves local time through that clock. Cargo runs every
/// case in this binary in one process, so both singletons are installed once and their guard is
/// kept alive here rather than dropped at the end of whichever case happened to run first.
static LOGGING: OnceLock<Result<InitializedLogging, String>> = OnceLock::new();

/// Brings this test process up to the same logging preconditions the application starts with.
fn initialize_process_logging() -> io::Result<()> {
    // `get_or_init` runs its initializer exactly once and blocks concurrent callers, so neither
    // process-wide singleton can be installed twice by cases running in parallel.
    LOGGING
        .get_or_init(|| {
            init_logging(LoggingConfig::new(
                LogLevel::Warn,
                LogOutput::Stdout,
                chrono_tz::Asia::Shanghai,
            ))
            .map_err(|error| error.to_string())
        })
        .as_ref()
        .map(|_| ())
        .map_err(|error| io::Error::other(error.clone()))
}

/// Owns the filesystem sandbox and Backend paths for one Desktop E2E case.
pub(crate) struct DesktopTestSetup {
    directory: TempDir,
    paths: BackendPaths,
}

impl DesktopTestSetup {
    /// Creates one isolated filesystem sandbox using the production file-backed database layout.
    pub(crate) fn new() -> io::Result<Self> {
        initialize_process_logging()?;
        let directory = tempdir()?;
        let root = directory.path().to_path_buf();
        let app_data_directory = root.join("app_data");
        let home_directory = root.join("home");
        fs::create_dir_all(&app_data_directory)?;
        fs::create_dir_all(&home_directory)?;

        Ok(Self {
            paths: BackendPaths {
                app_data_directory,
                home_directory,
                deno_path: PathBuf::from("deno"),
                relative_path_base: root,
                timezone: chrono_tz::Asia::Shanghai,
            },
            directory,
        })
    }

    /// Returns the root that contains every file owned by this E2E case.
    pub(crate) fn root(&self) -> &Path {
        self.directory.path()
    }

    /// Returns the production-shaped paths assigned to this E2E case.
    pub(crate) fn backend_paths(&self) -> &BackendPaths {
        &self.paths
    }
}
