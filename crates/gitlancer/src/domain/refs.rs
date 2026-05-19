/// Identifies a branch ref by its fully qualified or shorthand name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BranchName(String);

impl BranchName {
    /// Creates a branch wrapper so branch-oriented APIs do not traffic in raw strings everywhere.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Exposes the branch name for command assembly and display.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifies a commit by its object ID as returned by Git.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitId(String);

impl CommitId {
    /// Creates a commit identifier wrapper so commit-oriented APIs can be more explicit.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the commit identifier for command assembly and display.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
