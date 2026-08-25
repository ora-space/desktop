use std::error::Error;

use thiserror::Error;

pub type BoxRepositorySource = Box<dyn Error + Send + Sync + 'static>;

/// Stable application port error that preserves the concrete adapter failure.
#[derive(Debug, Error)]
#[error("repository operation failed")]
pub struct RepositoryError {
    #[source]
    source: BoxRepositorySource,
}

impl RepositoryError {
    pub fn new(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(error),
        }
    }

    pub fn from_boxed(source: BoxRepositorySource) -> Self {
        Self { source }
    }

    /// Tests a concrete semantic adapter error without flattening its source chain into text.
    pub fn is<T: Error + 'static>(&self) -> bool {
        self.source.is::<T>()
    }

    #[doc(hidden)]
    pub fn from_message(message: impl Into<String>) -> Self {
        Self::new(std::io::Error::other(message.into()))
    }
}
