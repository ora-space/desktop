use std::fmt;

/// Identifies one live instance produced by an `open`; monotonic within a process, never persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceInstanceId(u64);

impl SurfaceInstanceId {
    /// Wraps a registry-allocated counter value; exposed so hosts can round-trip ids from events.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw counter value used in events and labels.
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Host-generated webview label: an opaque `<family>:<instance>` value.
///
/// The label carries no plugin identity on purpose. It exists for two lookups only: Tauri
/// capability matching (`plugin-webview:*` receives the bridge command, `remote-webview:*`
/// receives nothing) and the registry's `resolve_label`. A host must never parse plugin id,
/// version, or generation out of it; those come from the registry record.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WebviewLabel(String);

impl WebviewLabel {
    /// Prefix of every workbench page webview; the only family with a Tauri capability.
    pub const WORKBENCH_PREFIX: &'static str = "plugin-webview:";
    /// Prefix of every external-site webview; matched by no capability at all.
    pub const REMOTE_PREFIX: &'static str = "remote-webview:";

    /// Builds the label of one workbench instance, e.g. `plugin-webview:7`.
    pub fn workbench(instance: SurfaceInstanceId) -> Self {
        Self(format!("{}{}", Self::WORKBENCH_PREFIX, instance.value()))
    }

    /// Builds the label of one external-site instance, e.g. `remote-webview:7`.
    pub fn remote(instance: SurfaceInstanceId) -> Self {
        Self(format!("{}{}", Self::REMOTE_PREFIX, instance.value()))
    }

    /// Returns the label text handed to the webview runtime.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WebviewLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Ticket of one asynchronous operation (open/close/migrate); completions must carry it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OperationId(u64);

impl OperationId {
    /// Wraps a registry-allocated counter value; exposed so hosts can thread tickets through
    /// their own async callbacks.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw ticket value for logging.
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Generation of the page inside one instance; bumped each time the webview is rebuilt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewGeneration(u32);

impl ViewGeneration {
    /// Generation of a freshly opened instance.
    pub const INITIAL: Self = Self(0);

    /// Returns the generation that follows a rebuild.
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Returns the raw generation counter for logging.
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Identifies one browser download within the process; allocated by the host, never by a page.
///
/// A download URL is not an identity: the same URL can be downloaded concurrently, and `blob:`
/// URLs carry no stable business meaning across events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DownloadId(u64);

impl DownloadId {
    /// Wraps a raw counter value; exposed so hosts can build fixtures and round-trip ids.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw counter value used in events.
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{SurfaceInstanceId, WebviewLabel};
    use pretty_assertions::assert_eq;

    /// Labels are family prefix plus instance counter and nothing else; both families stay inside
    /// the Tauri label alphabet `[A-Za-z0-9-/:_]`.
    #[test]
    fn labels_are_opaque_family_prefixed_counters() {
        let workbench = WebviewLabel::workbench(SurfaceInstanceId::new(7));
        let remote = WebviewLabel::remote(SurfaceInstanceId::new(u64::MAX));
        let offending = |label: &WebviewLabel| {
            label
                .as_str()
                .chars()
                .filter(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')))
                .count()
        };
        assert_eq!(
            (
                workbench.as_str(),
                remote.as_str(),
                offending(&workbench),
                offending(&remote),
            ),
            (
                "plugin-webview:7",
                "remote-webview:18446744073709551615",
                0,
                0,
            )
        );
    }
}
