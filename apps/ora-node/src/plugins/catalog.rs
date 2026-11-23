//! Package discovery and use leases share the same short critical section as publish/remove.
use super::model_configuration::PackageArtifact;
use crate::session::PluginCatalog;
use ora_node_protocol::{PluginFailureCode, PluginId, PluginVersion};
use ora_plugin_manager::{
    ExpectedAgentDeclaration, PreparedPlugin, inspect_planned_package, installed_root,
    verify_agent_declaration,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

/// A missing version retains the whole plugin identity; MCP leases name one exact version.
type PackageUses = HashMap<(String, Option<String>), usize>;

/// Exact-version package catalogue and lease owner for one Node home.
#[derive(Clone)]
pub struct DirectoryPluginCatalog {
    pub(super) home: PathBuf,
    uses: Arc<Mutex<PackageUses>>,
    pub(super) stops: Option<ora_node_db::SessionJournal>,
}

/// Keeping this value alive prevents replacement or removal of its plugin.
pub struct PluginUseLease {
    uses: Arc<Mutex<PackageUses>>,
    id: (String, Option<String>),
}

impl Drop for PluginUseLease {
    /// Releases exactly one reader; no file operations happen when the last reader leaves.
    fn drop(&mut self) {
        let mut uses = self.uses.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = uses.get_mut(&self.id) {
            *count -= 1;
            if *count == 0 {
                uses.remove(&self.id);
            }
        }
    }
}

impl DirectoryPluginCatalog {
    /// Combines live exact leases with restart-retained package ownership under the use lock.
    fn held_versions(
        &self,
        uses: &PackageUses,
        id: &ora_domain::PluginId,
    ) -> Result<Vec<Option<String>>, PluginFailureCode> {
        let canonical = id.canonical();
        let mut held: Vec<_> = uses
            .keys()
            .filter(|(key, _)| key == &canonical)
            .map(|(_, version)| version.clone())
            .collect();
        if let Some(journal) = &self.stops {
            for record in journal
                .session_stop_records(None)
                .map_err(|_| PluginFailureCode::PluginInUse)?
            {
                if !record.resources_released {
                    held.extend(
                        record
                            .identity
                            .packages
                            .iter()
                            .filter(|package| package.plugin_id.as_str() == canonical)
                            .map(|package| Some(package.plugin_version.as_str().to_owned())),
                    );
                }
            }
        }
        Ok(held)
    }

    /// Only another intact MCP version may coexist; unknown kinds and same-version repairs block.
    fn mutation_blocked(
        &self,
        uses: &PackageUses,
        id: &ora_domain::PluginId,
        target: &semver::Version,
    ) -> Result<bool, PluginFailureCode> {
        for version in self.held_versions(uses, id)? {
            let Some(version) = version.and_then(|version| version.parse::<semver::Version>().ok())
            else {
                return Ok(true);
            };
            let path = self.path(id, &version);
            if &version == target
                || inspect_planned_package(&path, id, &version).is_err()
                || !ora_plugin_manager::is_mcp_package(&path).unwrap_or(false)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Creates the shared catalogue without touching a potentially unavailable plugin root.
    pub(super) fn new(home: PathBuf) -> Self {
        Self {
            home,
            uses: Arc::default(),
            stops: None,
        }
    }

    /// Keeps temporary packages outside discovery's installed tree.
    pub(super) fn staging_root(&self) -> PathBuf {
        self.home.join("plugins").join(".node-installs")
    }

    /// Serializes lease acquisition with the filesystem publication boundary.
    fn uses(&self) -> MutexGuard<'_, PackageUses> {
        self.uses.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Restarted ownership and failed journal reads keep packages protected without a live actor.
    fn retained(&self, id: &ora_domain::PluginId) -> Result<bool, PluginFailureCode> {
        match &self.stops {
            Some(journal) => Ok(journal
                .session_stop_records(None)
                .map_err(|_| PluginFailureCode::PluginInUse)?
                .iter()
                .any(|record| {
                    !record.resources_released
                        && record
                            .identity
                            .packages
                            .iter()
                            .any(|package| package.plugin_id.as_str() == id.canonical())
                })),
            None => Ok(false),
        }
    }

    /// Returns the location derived only from plugin-domain validated segments.
    pub(super) fn path(&self, id: &ora_domain::PluginId, version: &semver::Version) -> PathBuf {
        installed_root(&self.home)
            .join(id.namespace())
            .join(id.name())
            .join(version.to_string())
    }

    /// A valid exact version needs no mutation, including when a session already uses it.
    pub(super) fn already_installed(
        &self,
        id: &ora_domain::PluginId,
        version: &semver::Version,
        declaration: Option<&ExpectedAgentDeclaration>,
        artifact: &PackageArtifact<'_>,
    ) -> Result<bool, PluginFailureCode> {
        let uses = self.uses();
        let path = self.path(id, version);
        if ora_utils::path::check_directory_without_symlinks(&path).is_ok()
            && inspect_planned_package(&path, id, version).is_ok()
        {
            // A previous unassociated plan proves no declaration identity. Recheck under the
            // same lock that fences publish/remove, without mutating a leased package.
            if let Some(expected) = declaration {
                verify_agent_declaration(&path, expected)
                    .map_err(|_| PluginFailureCode::InvalidPackage)?;
            }
            // A declared package installed without a receipt (or from other bytes) is
            // reinstalled, so the configured session that follows can prove its artifact.
            if declaration.is_none() || self.package_recorded(id, version, artifact) {
                return Ok(true);
            }
        }
        if self.mutation_blocked(&uses, id, version)? {
            return Err(PluginFailureCode::PluginInUse);
        }
        Ok(false)
    }

    /// Checks again after downloading: a newly started session wins over replacing its package.
    pub(super) fn publish(
        &self,
        id: &ora_domain::PluginId,
        version: &semver::Version,
        prepared: PreparedPlugin,
        artifact: &PackageArtifact<'_>,
    ) -> Result<(), PluginFailureCode> {
        let uses = self.uses();
        if self.mutation_blocked(&uses, id, version)? {
            return Err(PluginFailureCode::PluginInUse);
        }
        let held = self.held_versions(&uses, id)?;
        if !held.is_empty() && !prepared.is_mcp().map_err(super::failure)? {
            return Err(PluginFailureCode::PluginInUse);
        }
        self.clear_package_record(id, version)
            .map_err(|_| PluginFailureCode::InstallFailed)?;
        let destination = prepared.commit(&self.home).map_err(super::failure)?;
        // Retire only version directories belonging to this plugin; its data/config live elsewhere.
        let parent = destination
            .parent()
            .ok_or(PluginFailureCode::InstallFailed)?;
        for entry in std::fs::read_dir(parent).map_err(|_| PluginFailureCode::InstallFailed)? {
            let entry = entry.map_err(|_| PluginFailureCode::InstallFailed)?;
            if entry.path() != self.path(id, version)
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
                && semver::Version::parse(&entry.file_name().to_string_lossy()).is_ok()
                && !held.iter().any(|version| {
                    version.as_deref() == Some(entry.file_name().to_string_lossy().as_ref())
                })
            {
                std::fs::remove_dir_all(entry.path())
                    .map_err(|_| PluginFailureCode::InstallFailed)?;
            }
        }
        self.record_package(id, version, artifact)
            .map_err(|_| PluginFailureCode::InstallFailed)
    }

    /// Removes the requested package identity without following a link or deleting plugin data.
    pub(super) fn remove(
        &self,
        id: &PluginId,
        version: &PluginVersion,
    ) -> Result<(), PluginFailureCode> {
        let (id, _) = identity(id, version)?;
        let uses = self.uses();
        if uses.keys().any(|(key, _)| key == &id.canonical()) || self.retained(&id)? {
            return Err(PluginFailureCode::PluginInUse);
        }
        let path = installed_root(&self.home)
            .join(id.namespace())
            .join(id.name());
        match ora_utils::path::check_directory_without_symlinks(&path) {
            Ok(()) => std::fs::remove_dir_all(path).map_err(|_| PluginFailureCode::InstallFailed),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(PluginFailureCode::InstallFailed),
        }
    }
}

impl PluginCatalog for DirectoryPluginCatalog {
    type Lease = PluginUseLease;

    /// Keeps an exact version alive while allowing another validated MCP version to publish.
    fn lease_mcp(&self, plugin_id: &PluginId, version: &PluginVersion) -> Self::Lease {
        let id = (
            plugin_id.as_str().to_owned(),
            Some(version.as_str().to_owned()),
        );
        *self.uses().entry(id.clone()).or_default() += 1;
        PluginUseLease {
            uses: Arc::clone(&self.uses),
            id,
        }
    }

    /// Takes a use lease before looking up files, so a session never observes a disappearing tree.
    fn lease(&self, plugin_id: &PluginId) -> Self::Lease {
        let id = (plugin_id.as_str().to_owned(), None);
        *self.uses().entry(id.clone()).or_default() += 1;
        PluginUseLease {
            uses: Arc::clone(&self.uses),
            id,
        }
    }

    /// Returns only a validated exact version, never a nearby/highest version or symlink escape.
    fn installed(&self, plugin_id: &PluginId, version: &PluginVersion) -> Option<PathBuf> {
        let (id, version) = identity(plugin_id, version).ok()?;
        let _uses = self.uses();
        let path = self.path(&id, &version);
        ora_utils::path::check_directory_without_symlinks(&path).ok()?;
        inspect_planned_package(&path, &id, &version).ok()?;
        Some(path)
    }

    fn check_configuration(
        &self,
        configuration: &ora_node_protocol::FrozenAgentConfiguration,
    ) -> Result<(), crate::session::InstalledConfigurationError> {
        let _uses = self.uses();
        self.check_installed_configuration(configuration)
    }
}

/// Wire identities are opaque; only the plugin domain may admit them as directory segments.
pub(super) fn identity(
    id: &PluginId,
    version: &PluginVersion,
) -> Result<(ora_domain::PluginId, semver::Version), PluginFailureCode> {
    let id =
        ora_domain::PluginId::parse(id.as_str()).map_err(|_| PluginFailureCode::InvalidPackage)?;
    let parsed =
        semver::Version::parse(version.as_str()).map_err(|_| PluginFailureCode::InvalidPackage)?;
    if parsed.to_string() != version.as_str() {
        return Err(PluginFailureCode::InvalidPackage);
    }
    Ok((id, parsed))
}
