//! Package discovery and use leases share the same short critical section as publish/remove.
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

/// Exact-version package catalogue and lease owner for one Node home.
#[derive(Clone)]
pub struct DirectoryPluginCatalog {
    pub(super) home: PathBuf,
    uses: Arc<Mutex<HashMap<String, usize>>>,
}

/// Keeping this value alive prevents replacement or removal of its plugin.
pub struct PluginUseLease {
    uses: Arc<Mutex<HashMap<String, usize>>>,
    id: String,
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
    /// Creates the shared catalogue without touching a potentially unavailable plugin root.
    pub(super) fn new(home: PathBuf) -> Self {
        Self {
            home,
            uses: Arc::default(),
        }
    }

    /// Keeps temporary packages outside discovery's installed tree.
    pub(super) fn staging_root(&self) -> PathBuf {
        self.home.join("plugins").join(".node-installs")
    }

    /// Serializes lease acquisition with the filesystem publication boundary.
    fn uses(&self) -> MutexGuard<'_, HashMap<String, usize>> {
        self.uses.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns the location derived only from plugin-domain validated segments.
    fn path(&self, id: &ora_domain::PluginId, version: &semver::Version) -> PathBuf {
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
        artifact: &super::model_configuration::PackageArtifact<'_>,
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
            if declaration.is_none()
                || self.package_recorded(id, version, artifact.sha256, artifact.target)
            {
                return Ok(true);
            }
        }
        if uses.contains_key(&id.canonical()) {
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
        artifact: &super::model_configuration::PackageArtifact<'_>,
    ) -> Result<(), PluginFailureCode> {
        let uses = self.uses();
        if uses.contains_key(&id.canonical()) {
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
            {
                std::fs::remove_dir_all(entry.path())
                    .map_err(|_| PluginFailureCode::InstallFailed)?;
            }
        }
        self.record_package(id, version, artifact.sha256, artifact.target)
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
        if uses.contains_key(&id.canonical()) {
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

    /// Takes a use lease before looking up files, so a session never observes a disappearing tree.
    fn lease(&self, plugin_id: &PluginId) -> Self::Lease {
        let id = plugin_id.as_str().to_owned();
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

    fn model_protocols(
        &self,
        configuration: &ora_plugin_protocol::AgentModelConfiguration,
    ) -> Option<Vec<ora_plugin_protocol::AgentModelProtocol>> {
        self.declared_protocols(configuration)
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
