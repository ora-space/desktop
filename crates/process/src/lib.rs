mod spec;
mod tokio_process;
mod traits;
#[cfg(windows)]
mod windows_tree;

pub use spec::{EnvironmentPolicy, ProcessSpec, ProcessStdio};
pub use tokio_process::{TokioManagedProcess, TokioProcessSpawner};
pub use traits::{
    ManagedProcess, ManagedProcessTree, ManagedProcessTreeParts, PluginStdio, ProcessExit,
    ProcessSpawner, ProcessTreeController, ProcessTreeError, ProcessTreeParts, ProcessTreeSpawner,
};
#[cfg(windows)]
pub use windows_tree::{WindowsJobProcessTree, WindowsJobProcessTreeSpawner};
