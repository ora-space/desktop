use std::future::Future;
use std::pin::Pin;

/// A pinned, heap-allocated future that resolves to unit and is safe to send across threads.
/// Used as the return type of [`Job::run`] to keep the trait object-safe.
pub type BoxFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Contract for a recurring task managed by the [`crate::Scheduler`].
///
/// Implementors declare their desired fire frequency via [`schedule`] (a standard five-field
/// cron expression) and perform their work inside [`run`].  The scheduler parses the expression
/// once at startup and then drives each registered job independently.
pub trait Job: Send + Sync + 'static {
    /// Returns a human-readable identifier for this job, used in log output.
    fn name(&self) -> &str;

    /// Returns the five-field cron expression that controls when this job fires.
    ///
    /// Example: `"15 2 * * *"` fires at 02:15 every day.
    fn schedule(&self) -> &str;

    /// Executes one iteration of this job's work.
    ///
    /// Returning [`BoxFuture`] instead of `async fn` keeps the trait object-safe so the
    /// scheduler can store heterogeneous jobs as `Box<dyn Job>`.
    fn run(&self) -> BoxFuture<'_>;
}
