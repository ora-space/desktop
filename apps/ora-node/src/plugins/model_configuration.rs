//! Management-owned package receipts bind a declared execution to verified archive bytes.

use super::DirectoryPluginCatalog;
use ora_plugin_protocol::{AgentModelConfiguration, AgentModelProtocol};
use std::io::{Read, Write};

/// The downloaded artifact's verified identity, selected by the installer before any reuse.
pub(super) struct PackageArtifact<'a> {
    pub(super) sha256: &'a str,
    pub(super) target: &'a str,
}

impl DirectoryPluginCatalog {
    /// Receipts live outside package content, so an archive cannot supply its own proof.
    fn receipt_path(
        &self,
        id: &ora_domain::PluginId,
        version: &semver::Version,
    ) -> std::path::PathBuf {
        self.home
            .join("plugins")
            .join(".node-receipts")
            .join(id.namespace())
            .join(id.name())
            .join(format!("{version}.json"))
    }

    /// Called only after digest-verified archive publication, under the catalog's use lock.
    pub(super) fn record_package(
        &self,
        id: &ora_domain::PluginId,
        version: &semver::Version,
        digest: &str,
        target: &str,
    ) -> std::io::Result<()> {
        let path = self.receipt_path(id, version);
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("invalid receipt path"))?;
        ora_utils::path::create_directories_without_symlinks(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(
            &serde_json::json!({ "sha256":digest, "target":target }),
        )?)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    /// Absence is unproven legacy state, never permission to adopt the current claimed digest.
    pub(super) fn package_recorded(
        &self,
        id: &ora_domain::PluginId,
        version: &semver::Version,
        digest: &str,
        target: &str,
    ) -> bool {
        let path = self.receipt_path(id, version);
        if path
            .parent()
            .is_none_or(|parent| ora_utils::path::check_directory_without_symlinks(parent).is_err())
        {
            return false;
        }
        if !std::fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() < 4096)
        {
            return false;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .is_ok_and(|value| value == serde_json::json!({ "sha256":digest, "target":target }))
    }

    /// Revalidates the installed declaration while the session holds its immutable package lease.
    pub(super) fn declared_protocols(
        &self,
        configuration: &AgentModelConfiguration,
    ) -> Option<Vec<AgentModelProtocol>> {
        let id = ora_domain::PluginId::parse(&configuration.plugin_id).ok()?;
        let version = semver::Version::parse(&configuration.plugin_version).ok()?;
        let target = ora_plugin_registry::current_host_target()?;
        if target.as_str() != configuration.target
            || configuration.adapter_version != 1
            || configuration.schema_version != 2
            || !self.package_recorded(
                &id,
                &version,
                &configuration.package_sha256,
                &configuration.target,
            )
        {
            return None;
        }
        let path = ora_plugin_manager::installed_root(&self.home)
            .join(id.namespace())
            .join(id.name())
            .join(version.to_string())
            .join("assets")
            .join("config.json");
        if !std::fs::symlink_metadata(&path).is_ok_and(|metadata| {
            metadata.is_file() && metadata.len() <= ora_plugin_config::MAX_DECLARATION_BYTES as u64
        }) {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(ora_plugin_config::MAX_DECLARATION_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        let declaration = ora_plugin_config::compile_agent_configuration(&bytes).ok()?;
        let settings = serde_json::from_str(&configuration.settings_json).ok()?;
        if declaration.fingerprint != configuration.declaration_fingerprint
            || !ora_plugin_config::validate_agent_settings(&declaration, &settings)
        {
            return None;
        }
        Some(
            declaration
                .model_access
                .protocols
                .into_iter()
                .map(|protocol| match protocol {
                    ora_plugin_config::AgentModelProtocol::OpenaiCompletions => {
                        AgentModelProtocol::OpenaiCompletions
                    }
                    ora_plugin_config::AgentModelProtocol::AnthropicMessages => {
                        AgentModelProtocol::AnthropicMessages
                    }
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_plugin_protocol::AgentBoundModel;
    use pretty_assertions::assert_eq;

    /// Archive proof and exact declaration/settings are all required, even for a third-party id.
    #[test]
    fn declaration_identity_requires_a_verified_artifact_receipt() {
        let root = tempfile::tempdir().expect("Node home");
        let catalog = DirectoryPluginCatalog::new(root.path().to_path_buf());
        let id = ora_domain::PluginId::parse("official/minimal").expect("plugin id");
        let version = semver::Version::new(1, 0, 0);
        let target = ora_plugin_registry::current_host_target().expect("Linux target");
        let source=b"{\"schemaVersion\":2,\"agent\":{\"modelAccess\":{\"contractVersion\":1,\"protocols\":[\"anthropic-messages\"],\"modelCardinality\":\"single\"}}}";
        let path = ora_plugin_manager::installed_root(root.path())
            .join("official")
            .join("minimal")
            .join("1.0.0")
            .join("assets");
        std::fs::create_dir_all(&path).expect("package");
        std::fs::write(path.join("config.json"), source).expect("declaration");
        let mut configuration = AgentModelConfiguration {
            plugin_id: id.canonical(),
            plugin_version: version.to_string(),
            target: target.as_str().into(),
            package_sha256: "a".repeat(64),
            schema_version: 2,
            declaration_fingerprint: ora_utils::hash::sha256_hex(source),
            adapter_version: 1,
            configuration_revision: 1,
            settings_json: "{}".into(),
            protocol: AgentModelProtocol::AnthropicMessages,
            model: AgentBoundModel {
                id: "vendor/model".into(),
                name: "Model".into(),
                context_window: 128000,
                max_tokens: 8192,
            },
        };
        assert_eq!(catalog.declared_protocols(&configuration), None);
        catalog
            .record_package(
                &id,
                &version,
                &configuration.package_sha256,
                &configuration.target,
            )
            .expect("verified receipt");
        assert_eq!(
            catalog.declared_protocols(&configuration),
            Some(vec![AgentModelProtocol::AnthropicMessages])
        );
        configuration.package_sha256 = "b".repeat(64);
        assert_eq!(catalog.declared_protocols(&configuration), None);
        configuration.package_sha256 = "a".repeat(64);
        configuration.settings_json = "{\"unexpected\":true}".into();
        assert_eq!(catalog.declared_protocols(&configuration), None);
    }
}
