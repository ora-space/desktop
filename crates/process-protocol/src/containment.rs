/// The caller's policy when the deployment cannot provide strong containment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainmentRequest {
    RequireStrong,
    PreferStrong,
    BestEffort,
}

/// The guarantee selected before any business process is allowed to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainmentGuarantee {
    Strong,
    BestEffort,
}
