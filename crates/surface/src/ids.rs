use ora_domain::PluginId;
use ora_plugin_manager::SurfaceId;
use std::fmt;

/// Identifies one manifest-contributed surface; stable across processes and restarts.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceDefinitionId {
    pub plugin_id: PluginId,
    pub surface_id: SurfaceId,
}

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

/// Host-generated webview label.
///
/// Tauri labels only accept `[A-Za-z0-9-/:_]`, so the plugin id is rendered as
/// `<namespace>_<name>` with every `.` of the name mapped to `_`. The mapping is unambiguous
/// because slugs never contain `_`. Labels are never used for authorization decisions; callers
/// resolve them through the registry instead.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WebviewLabel(String);

impl WebviewLabel {
    pub const REMOTE_PREFIX: &'static str = "remote-surface:";
    pub const PANEL_PREFIX: &'static str = "panel-surface:";

    /// Builds the label of one remote site surface instance, e.g.
    /// `remote-surface:official_ora-space_skillhub:market:7`.
    pub fn remote(definition: &SurfaceDefinitionId, instance: SurfaceInstanceId) -> Self {
        Self::with_prefix(Self::REMOTE_PREFIX, definition, instance)
    }

    /// Builds the label of one panel surface instance, e.g.
    /// `panel-surface:official_ora-space_hello-panel:counter:7`. The prefix identifies the panel family
    /// so the bridge can tell panel webviews apart from remote-site webviews.
    pub fn panel(definition: &SurfaceDefinitionId, instance: SurfaceInstanceId) -> Self {
        Self::with_prefix(Self::PANEL_PREFIX, definition, instance)
    }

    fn with_prefix(
        prefix: &str,
        definition: &SurfaceDefinitionId,
        instance: SurfaceInstanceId,
    ) -> Self {
        let namespace = definition.plugin_id.namespace();
        let name = definition.plugin_id.name().replace('.', "_");
        let surface = definition.surface_id.as_str();
        let instance = instance.value();
        Self(format!("{prefix}{namespace}_{name}:{surface}:{instance}"))
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

#[cfg(test)]
mod tests {
    use super::{SurfaceDefinitionId, SurfaceInstanceId, WebviewLabel};
    use ora_domain::PluginId;
    use ora_plugin_manager::SurfaceId;
    use pretty_assertions::assert_eq;

    /// Verifies the documented label shape for a namespaced two-segment plugin name.
    #[test]
    fn remote_label_joins_namespace_and_maps_name_dot_to_underscore() {
        let definition = SurfaceDefinitionId {
            plugin_id: PluginId::new("official", "ora-space.skillhub").expect("plugin id"),
            surface_id: SurfaceId::parse("market").expect("valid surface id"),
        };
        assert_eq!(
            WebviewLabel::remote(&definition, SurfaceInstanceId::new(7)).as_str(),
            "remote-surface:official_ora-space_skillhub:market:7"
        );
    }

    /// Verifies the panel family shares the shape and differs only by prefix.
    #[test]
    fn panel_label_uses_panel_prefix() {
        let definition = SurfaceDefinitionId {
            plugin_id: PluginId::new("official", "ora-space.hello-panel").expect("plugin id"),
            surface_id: SurfaceId::parse("counter").expect("valid surface id"),
        };
        assert_eq!(
            WebviewLabel::panel(&definition, SurfaceInstanceId::new(7)).as_str(),
            "panel-surface:official_ora-space_hello-panel:counter:7"
        );
    }

    /// Enumerates every character a slug-based plugin id and surface id can contain and checks
    /// the resulting label stays inside the Tauri label alphabet `[A-Za-z0-9-/:_]`.
    #[test]
    fn remote_label_only_contains_tauri_label_characters() {
        let slug_alphabet: String = ('a'..='z').chain('0'..='9').collect();
        let plugin_id = PluginId::new(
            format!("{slug_alphabet}-n"),
            format!("{slug_alphabet}-x.{slug_alphabet}-y"),
        )
        .expect("plugin id");
        let surface_id = SurfaceId::parse(&format!("{}-{}", &slug_alphabet[..20], "9"))
            .expect("valid surface id");
        let definition = SurfaceDefinitionId {
            plugin_id,
            surface_id,
        };
        let label = WebviewLabel::remote(&definition, SurfaceInstanceId::new(u64::MAX));
        let offending: Vec<char> = label
            .as_str()
            .chars()
            .filter(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')))
            .collect();
        assert_eq!((offending, label.as_str().contains('.')), (vec![], false));
    }
}
