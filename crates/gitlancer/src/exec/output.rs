/// Represents normalized process output from one Git command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

impl GitOutput {
    /// Creates normalized process output so parsers never need to know about the process API directly.
    pub fn new(code: Option<i32>, stdout: String, stderr: String, duration_ms: u64) -> Self {
        Self {
            code,
            stdout,
            stderr,
            duration_ms,
        }
    }
}
