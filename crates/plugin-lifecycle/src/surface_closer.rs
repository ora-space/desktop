//! Injection point through which the host closes a plugin's open surfaces before its process stops.
//!
//! The lifecycle is constructed by the backend before any window exists, while the surfaces are
//! owned by the desktop shell that is built afterwards. The closer therefore has to be installed
//! after construction, and the lifecycle has no static knowledge of its type. That is the one
//! place in this crate where runtime polymorphism is genuinely required, so the installed closer
//! is stored type-erased; before anything is installed, closing is a no-op.

use ora_domain::PluginId;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

/// Closes every open surface instance of one plugin and resolves once they are gone.
///
/// Implementations are invoked inside the plugin's operation lock, immediately before the
/// lifecycle stops the runtime or removes the package. They must therefore be bounded: a closer
/// that never resolves blocks every later operation on that plugin.
pub trait SurfaceCloser: Send + Sync + 'static {
    /// Closes all surfaces of `plugin_id`; a plugin without surfaces is not an error.
    fn close_all(&self, plugin_id: &PluginId) -> impl Future<Output = ()> + Send;
}

/// Dyn-compatible mirror of `SurfaceCloser`, boxed so it can live behind a trait object.
trait ErasedSurfaceCloser: Send + Sync {
    /// Boxed form of `SurfaceCloser::close_all`.
    fn close_all_erased<'a>(
        &'a self,
        plugin_id: &'a PluginId,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

impl<Closer: SurfaceCloser> ErasedSurfaceCloser for Closer {
    /// Boxes the statically dispatched future so the slot can hold any closer type.
    fn close_all_erased<'a>(
        &'a self,
        plugin_id: &'a PluginId,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(self.close_all(plugin_id))
    }
}

/// Holds the optionally installed closer for the lifetime of one lifecycle.
#[derive(Default)]
pub(crate) struct SurfaceCloserSlot {
    closer: RwLock<Option<Arc<dyn ErasedSurfaceCloser>>>,
}

impl SurfaceCloserSlot {
    /// Installs or replaces the closer; later operations use the new one.
    pub(crate) fn install(&self, closer: impl SurfaceCloser) {
        *self.closer.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(closer));
    }

    /// Closes the plugin's surfaces through the installed closer, or returns at once if none is.
    pub(crate) async fn close_all(&self, plugin_id: &PluginId) {
        // Clone out of the lock so the closer's future never runs while the guard is held.
        let closer = self
            .closer
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(closer) = closer {
            closer.close_all_erased(plugin_id).await;
        }
    }
}
