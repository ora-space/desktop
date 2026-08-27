//! Surface failures and their projection onto the desktop command error contract.

use crate::error::CommandError;
use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_domain::PluginId;
use ora_surface::{CommandError as RegistryCommandError, SurfaceInstanceId, TransitionError};
use thiserror::Error;

/// Why a surface operation was refused or failed.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SurfaceError {
    #[error("plugin {0} is not installed")]
    PluginNotFound(PluginId),
    #[error("surface instance {} is not registered", .0.value())]
    InstanceNotFound(SurfaceInstanceId),
    #[error(transparent)]
    Transition(TransitionError),
    #[cfg(not(feature = "embedded-surfaces"))]
    #[error("{0} is not supported by this build")]
    Unsupported(&'static str),
    #[error("{0}")]
    Internal(String),
}

impl From<RegistryCommandError> for SurfaceError {
    fn from(error: RegistryCommandError) -> Self {
        match error {
            RegistryCommandError::UnknownInstance(instance) => Self::InstanceNotFound(instance),
            RegistryCommandError::Transition(transition) => Self::Transition(transition),
        }
    }
}

impl From<SurfaceError> for CommandError {
    /// Maps surface failures onto the existing public error vocabulary.
    ///
    /// The contract crate has no surface-specific codes yet, so the closest existing code is
    /// used and the precise reason travels in the logged context: not-found instances and
    /// unsupported operations are `invalid_request`, a busy instance is `resource_in_use`.
    fn from(error: SurfaceError) -> Self {
        let context = error.to_string();
        let (classification, public_error) = match &error {
            SurfaceError::PluginNotFound(_) => (
                ErrorClassification::NotFound,
                PublicError::PluginNotFound(EmptyErrorParams {}),
            ),
            SurfaceError::InstanceNotFound(_) => (
                ErrorClassification::NotFound,
                PublicError::InvalidRequest(EmptyErrorParams {}),
            ),
            SurfaceError::Transition(TransitionError::Busy { .. }) => (
                ErrorClassification::Conflict,
                PublicError::ResourceInUse(EmptyErrorParams {}),
            ),
            #[cfg(not(feature = "embedded-surfaces"))]
            SurfaceError::Unsupported(_) => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
            ),
            SurfaceError::Transition(TransitionError::InvalidForState { .. }) => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
            ),
            SurfaceError::Internal(_) => (
                ErrorClassification::Internal,
                PublicError::InternalError(EmptyErrorParams {}),
            ),
        };
        CommandError::from_backend(BackendError::new(classification, public_error, context))
    }
}
