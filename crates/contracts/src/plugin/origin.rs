//! User-facing origin of a plugin and of every Skill it provides.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Classifies where a plugin was first brought in from, so namesakes can be told apart.
///
/// The host derives it from the plugin's namespace and fills it into every list that shows a
/// plugin or a plugin Skill; the frontend only renders it and never parses namespaces or URLs.
/// `label` is computed per query and may change when a namesake source is added, so it is display
/// text only and never an identity to match on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin-origin.ts")]
pub enum PluginOrigin {
    /// Imported from a local `.orax` archive.
    LocalArchive,
    /// Published by the default first-party marketplace.
    Official,
    /// First brought in from a third-party marketplace source.
    ///
    /// `sourceUrl` is the source's canonical URL; it is null only when the host lost the
    /// namespace's source binding, and the label then falls back to the namespace slug.
    Marketplace {
        source_url: Option<String>,
        label: String,
    },
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    PluginOrigin::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::PluginOrigin;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Pins the wire shape the frontend switches on.
    #[test]
    fn serializes_with_a_kind_tag_and_camel_case_fields() {
        assert_eq!(
            [
                serde_json::to_value(PluginOrigin::LocalArchive).expect("serialize"),
                serde_json::to_value(PluginOrigin::Official).expect("serialize"),
                serde_json::to_value(PluginOrigin::Marketplace {
                    source_url: Some("https://github.com/acme/tools".to_string()),
                    label: "tools".to_string(),
                })
                .expect("serialize"),
            ],
            [
                json!({ "kind": "local_archive" }),
                json!({ "kind": "official" }),
                json!({
                    "kind": "marketplace",
                    "sourceUrl": "https://github.com/acme/tools",
                    "label": "tools",
                }),
            ],
        );
    }
}
