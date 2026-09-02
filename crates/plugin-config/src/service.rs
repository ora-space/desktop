use crate::declaration::parse_strict_json;
use crate::filesystem::{ConfigurationFileSystem, StandardConfigurationFileSystem};
use crate::mcp::{
    CompileConfigurationFileError, CompiledConfigurationFile, compile_configuration_file,
};
use crate::values::{StoredConfiguration, details_from, validate_values};
use crate::{
    CompileDeclarationError, CompiledDeclaration, MAX_DECLARATION_BYTES, SettingDeclaration,
    SettingValue,
};
use ora_domain::PluginId;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Reports whether every required Setting has an available type-correct effective value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationCompleteness {
    Complete,
    Incomplete,
}

/// Represents the exclusive host-visible configuration state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationSummary {
    NotDeclared,
    Available {
        completeness: ConfigurationCompleteness,
    },
    Unavailable {
        error_code: String,
    },
}

/// Formats the local-time suffix used when preserving a damaged `store.json`.
pub fn recovery_backup_label(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
) -> String {
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}")
}

/// Identifies where the currently effective value originates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveValueSource {
    Stored,
    Default,
    Absent,
}

/// Projects one declaration together with its stored and effective values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingDetails {
    pub declaration: SettingDeclaration,
    pub stored_value: Option<SettingValue>,
    pub effective_value: Option<SettingValue>,
    pub source: EffectiveValueSource,
    pub value_error_code: Option<String>,
}

/// Carries the complete editor snapshot bound to one revision and declaration fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationDetails {
    pub declaration: CompiledDeclaration,
    pub settings: Vec<SettingDetails>,
    pub revision: u64,
    pub summary: ConfigurationSummary,
}

/// Identifies one invalid submitted override without requiring consumers to parse messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationFieldError {
    pub setting_id: String,
    pub error_code: String,
}

/// Reports configuration declarations or stored values that cannot be served safely.
#[derive(Debug, Error)]
pub enum ConfigurationError {
    #[error("failed to access Plugin Configuration at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Plugin Configuration declaration is invalid: {0}")]
    InvalidDeclaration(#[from] CompileConfigurationFileError),
    #[error("plugin does not declare configuration")]
    NotDeclared,
    #[error("Plugin Configuration declaration changed while it was being edited")]
    DeclarationChanged,
    #[error("Plugin Configuration revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("Plugin Configuration contains invalid field values")]
    InvalidValues {
        field_errors: Vec<ConfigurationFieldError>,
    },
    #[error("Plugin Configuration value file could not be loaded: {reason}")]
    LoadFailed { reason: String },
    #[error("Plugin Configuration could not be serialized: {reason}")]
    PersistFailed { reason: String },
    #[error("plugin identifier `{plugin_id}` is not a valid namespaced slug")]
    InvalidPluginId { plugin_id: String },
    #[error("Plugin Configuration revision has reached its maximum value")]
    RevisionExhausted,
    #[error("Plugin Configuration write lock is unavailable")]
    LockUnavailable,
    #[error("Plugin Configuration recovery was requested for a readable value file")]
    RecoveryNotRequired,
    #[error(
        "Plugin Configuration recovery could not restore `{path}` after write failure `{write_error}`: {restore_error}"
    )]
    RecoveryRestoreFailed {
        path: PathBuf,
        write_error: String,
        #[source]
        restore_error: std::io::Error,
    },
}

impl From<CompileDeclarationError> for ConfigurationError {
    /// Keeps `?` working for Settings-only compile failures after the variant broadened to
    /// carry both strict `assets/config.json` shapes.
    fn from(error: CompileDeclarationError) -> Self {
        Self::InvalidDeclaration(error.into())
    }
}

/// Owns declaration lookup and Stored Setting Value resolution below one host data root.
#[derive(Debug, Clone)]
pub struct ConfigurationService<FileSystem = StandardConfigurationFileSystem> {
    data_root: std::path::PathBuf,
    file_system: FileSystem,
    locks: Arc<Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>>>,
}

impl ConfigurationService<StandardConfigurationFileSystem> {
    /// Creates a production service rooted in the Ora data directory.
    pub fn new(data_root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
            file_system: StandardConfigurationFileSystem,
            locks: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Loads one package declaration without constructing a data-root-backed service.
    ///
    /// Installation validation only needs the immutable declaration; using the package directory
    /// as a fake data root would make later store reads write beside plugin source.
    pub fn declaration_from_package(
        package_root: &Path,
    ) -> Result<Option<CompiledDeclaration>, ConfigurationError> {
        Self::package_service().load_declaration(package_root)
    }

    /// Loads and compiles one package `assets/config.json` in whichever strict shape it declares.
    ///
    /// Package validation dispatches on the result to enforce kind policy: an MCP package must
    /// compile to the MCP shape and every other kind must not.
    pub fn configuration_file_from_package(
        package_root: &Path,
    ) -> Result<Option<CompiledConfigurationFile>, ConfigurationError> {
        Self::package_service().load_configuration_file(package_root)
    }

    /// Builds a throwaway service for package-scoped reads that never touch a data root.
    fn package_service() -> Self {
        Self {
            data_root: PathBuf::new(),
            file_system: StandardConfigurationFileSystem,
            locks: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}

impl<FileSystem> ConfigurationService<FileSystem>
where
    FileSystem: ConfigurationFileSystem,
{
    /// Creates a service with an injected filesystem port for deterministic fault tests.
    pub fn with_file_system(data_root: impl Into<PathBuf>, file_system: FileSystem) -> Self {
        Self {
            data_root: data_root.into(),
            file_system,
            locks: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Loads editor details for one installed plugin package.
    pub fn get(
        &self,
        plugin_id: &str,
        package_root: &Path,
    ) -> Result<Option<ConfigurationDetails>, ConfigurationError> {
        let Some(declaration) = self.load_declaration(package_root)? else {
            return Ok(None);
        };
        match self.load_store(plugin_id) {
            Ok(store) => Ok(Some(details_from(declaration, store))),
            Err(ConfigurationError::LoadFailed { .. } | ConfigurationError::Io { .. }) => {
                let mut details = details_from(declaration, StoredConfiguration::default());
                details.summary = ConfigurationSummary::Unavailable {
                    error_code: "configuration_load_failed".to_string(),
                };
                Ok(Some(details))
            }
            Err(error) => Err(error),
        }
    }

    /// Loads the optional immutable declaration for installation validation and orchestration.
    pub fn declaration(
        &self,
        package_root: &Path,
    ) -> Result<Option<CompiledDeclaration>, ConfigurationError> {
        self.load_declaration(package_root)
    }

    /// Returns the list-facing state without converting unreadable data into missing values.
    pub fn summary(&self, plugin_id: &str, package_root: &Path) -> ConfigurationSummary {
        match self.get(plugin_id, package_root) {
            Ok(None) => ConfigurationSummary::NotDeclared,
            Ok(Some(details)) => details.summary,
            Err(ConfigurationError::InvalidDeclaration(_)) => ConfigurationSummary::Unavailable {
                error_code: "plugin_configuration_declaration_invalid".to_string(),
            },
            Err(_) => ConfigurationSummary::Unavailable {
                error_code: "configuration_load_failed".to_string(),
            },
        }
    }

    /// Persists a complete explicit override replacement after optimistic concurrency checks.
    pub fn save(
        &self,
        plugin_id: &str,
        package_root: &Path,
        expected_revision: u64,
        declaration_fingerprint: &str,
        values: BTreeMap<String, SettingValue>,
    ) -> Result<ConfigurationDetails, ConfigurationError> {
        self.save_preserving(
            plugin_id,
            package_root,
            expected_revision,
            declaration_fingerprint,
            values,
            &std::collections::BTreeSet::new(),
        )
    }

    /// Replaces overrides while retaining only host-redacted Settings named by the UI.
    pub fn save_preserving(
        &self,
        plugin_id: &str,
        package_root: &Path,
        expected_revision: u64,
        declaration_fingerprint: &str,
        mut values: BTreeMap<String, SettingValue>,
        preserved_setting_ids: &std::collections::BTreeSet<String>,
    ) -> Result<ConfigurationDetails, ConfigurationError> {
        let declaration = self
            .load_declaration(package_root)?
            .ok_or(ConfigurationError::NotDeclared)?;
        if declaration.fingerprint != declaration_fingerprint {
            return Err(ConfigurationError::DeclarationChanged);
        }
        let declared_ids = declaration
            .settings
            .iter()
            .map(|setting| setting.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let mut field_errors = preserved_setting_ids
            .iter()
            .filter(|setting_id| !declared_ids.contains(setting_id.as_str()))
            .map(|setting_id| ConfigurationFieldError {
                setting_id: setting_id.clone(),
                error_code: "setting_not_declared".to_string(),
            })
            .collect::<Vec<_>>();
        field_errors.extend(validate_values(&declaration, &values));
        if !field_errors.is_empty() {
            return Err(ConfigurationError::InvalidValues { field_errors });
        }
        let plugin_lock = self.plugin_lock(plugin_id)?;
        let _guard = plugin_lock
            .lock()
            .map_err(|_| ConfigurationError::LockUnavailable)?;
        let current = self.load_store(plugin_id)?;
        if current.revision != expected_revision {
            return Err(ConfigurationError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        for setting_id in preserved_setting_ids {
            if let Some(value) = current.values.get(setting_id) {
                values.insert(setting_id.clone(), value.clone());
            }
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or(ConfigurationError::RevisionExhausted)?;
        let replacement = StoredConfiguration {
            schema_version: 1,
            revision,
            values,
        };
        self.write_store(plugin_id, &replacement)?;
        Ok(details_from(declaration, replacement))
    }

    /// Removes every explicit override while preserving revision monotonicity.
    pub fn reset_all(
        &self,
        plugin_id: &str,
        package_root: &Path,
        expected_revision: u64,
        declaration_fingerprint: &str,
    ) -> Result<ConfigurationDetails, ConfigurationError> {
        self.save(
            plugin_id,
            package_root,
            expected_revision,
            declaration_fingerprint,
            BTreeMap::new(),
        )
    }

    /// Backs up a malformed value file before replacing it with an empty first recovery revision.
    pub fn recover_corrupt(
        &self,
        plugin_id: &str,
        package_root: &Path,
        declaration_fingerprint: &str,
        local_timestamp: &str,
    ) -> Result<ConfigurationDetails, ConfigurationError> {
        let declaration = self
            .load_declaration(package_root)?
            .ok_or(ConfigurationError::NotDeclared)?;
        if declaration.fingerprint != declaration_fingerprint {
            return Err(ConfigurationError::DeclarationChanged);
        }
        let plugin_lock = self.plugin_lock(plugin_id)?;
        let _guard = plugin_lock
            .lock()
            .map_err(|_| ConfigurationError::LockUnavailable)?;
        match self.load_store(plugin_id) {
            Err(ConfigurationError::LoadFailed { .. } | ConfigurationError::Io { .. }) => {}
            Err(error) => return Err(error),
            Ok(_) => return Err(ConfigurationError::RecoveryNotRequired),
        }
        let path = self.store_path(plugin_id)?;
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| ConfigurationError::LoadFailed {
                reason: "value file name is not valid UTF-8".to_string(),
            })?;
        let backup = (0_u16..=u16::MAX)
            .find_map(|attempt| {
                let suffix = if attempt == 0 {
                    String::new()
                } else {
                    format!("-{attempt}")
                };
                let candidate =
                    path.with_file_name(format!("{file_name}.corrupt-{local_timestamp}{suffix}"));
                match self.file_system.move_no_replace(&path, &candidate) {
                    Ok(()) => Some(Ok(candidate)),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(source) => Some(Err(ConfigurationError::Io {
                        path: path.clone(),
                        source,
                    })),
                }
            })
            .transpose()?
            .ok_or_else(|| ConfigurationError::Io {
                path: path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "could not allocate a unique corrupt configuration backup",
                ),
            })?;
        let replacement = StoredConfiguration {
            schema_version: 1,
            revision: 1,
            values: BTreeMap::new(),
        };
        if let Err(error) = self.write_store(plugin_id, &replacement) {
            if let Err(restore_error) = self.file_system.move_no_replace(&backup, &path) {
                return Err(ConfigurationError::RecoveryRestoreFailed {
                    path,
                    write_error: error.to_string(),
                    restore_error,
                });
            }
            return Err(error);
        }
        Ok(details_from(declaration, replacement))
    }

    /// Loads and compiles the optional immutable package declaration.
    ///
    /// An MCP-shaped file contributes exactly its Settings subset here, so the value editor and
    /// summaries treat an MCP package without Settings like any package without a declaration.
    fn load_declaration(
        &self,
        package_root: &Path,
    ) -> Result<Option<CompiledDeclaration>, ConfigurationError> {
        Ok(self
            .load_configuration_file(package_root)?
            .and_then(|file| match file {
                CompiledConfigurationFile::Settings(declaration) => Some(declaration),
                CompiledConfigurationFile::Mcp(configuration) => configuration.settings,
                // A Hook Configuration may declare a future plugin-global Settings subset; RTK
                // v0.1.0 declares none, so the settings summary stays NotDeclared until one ships.
                CompiledConfigurationFile::Hook(configuration) => configuration.settings,
            }))
    }

    /// Loads and compiles the optional `assets/config.json` in whichever strict shape it declares.
    fn load_configuration_file(
        &self,
        package_root: &Path,
    ) -> Result<Option<CompiledConfigurationFile>, ConfigurationError> {
        let declaration_path = package_root.join("assets").join("config.json");
        // `NotADirectory` means an intermediate path component is a file (common when uninstall
        // staging leaves a non-directory package root). That is the same as a missing declaration
        // for list summaries: there is no `assets/config.json` to compile.
        let source = match self
            .file_system
            .read_bounded(&declaration_path, MAX_DECLARATION_BYTES)
        {
            Ok(None) => return Ok(None),
            Ok(Some(contents)) => contents,
            Err(source) if source.kind() == std::io::ErrorKind::NotADirectory => return Ok(None),
            Err(source) => {
                return Err(ConfigurationError::Io {
                    path: declaration_path,
                    source,
                });
            }
        };
        Ok(Some(compile_configuration_file(&source)?))
    }

    /// Reads one revisioned value file while preserving malformed data for explicit recovery.
    fn load_store(&self, plugin_id: &str) -> Result<StoredConfiguration, ConfigurationError> {
        let path = self.store_path(plugin_id)?;
        let Some(source) = self
            .file_system
            .read_bounded(&path, MAX_STORE_BYTES)
            .map_err(|source| ConfigurationError::Io {
                path: path.clone(),
                source,
            })?
        else {
            return Ok(StoredConfiguration::default());
        };
        if source.len() > MAX_STORE_BYTES {
            return Err(ConfigurationError::LoadFailed {
                reason: format!("value file exceeds the {MAX_STORE_BYTES}-byte limit"),
            });
        }
        let value = parse_strict_json(&source).map_err(|error| ConfigurationError::LoadFailed {
            reason: error.to_string(),
        })?;
        let store: StoredConfiguration =
            serde_json::from_value(value).map_err(|error| ConfigurationError::LoadFailed {
                reason: error.to_string(),
            })?;
        if store.schema_version != 1 {
            return Err(ConfigurationError::LoadFailed {
                reason: format!(
                    "unsupported value schema version {schema_version}",
                    schema_version = store.schema_version
                ),
            });
        }
        Ok(store)
    }

    /// Serializes one validated value replacement into the plugin-owned data directory.
    fn write_store(
        &self,
        plugin_id: &str,
        store: &StoredConfiguration,
    ) -> Result<(), ConfigurationError> {
        let path = self.store_path(plugin_id)?;
        let parent = path
            .parent()
            .ok_or_else(|| ConfigurationError::PersistFailed {
                reason: "value file has no parent directory".to_string(),
            })?;
        self.file_system
            .create_dir_all(parent)
            .map_err(|source| ConfigurationError::Io {
                path: path.clone(),
                source,
            })?;
        let source =
            serde_json::to_vec(store).map_err(|error| ConfigurationError::PersistFailed {
                reason: error.to_string(),
            })?;
        if source.len() > MAX_STORE_BYTES {
            return Err(ConfigurationError::InvalidValues {
                field_errors: vec![ConfigurationFieldError {
                    setting_id: String::new(),
                    error_code: "configuration_too_large".to_string(),
                }],
            });
        }
        self.file_system
            .atomic_write(&path, &source)
            .map_err(|source| ConfigurationError::Io { path, source })
    }

    /// Resolves a plugin-global value path from a validated namespaced identifier.
    fn store_path(&self, plugin_id: &str) -> Result<PathBuf, ConfigurationError> {
        let plugin_id =
            PluginId::parse(plugin_id).map_err(|_| ConfigurationError::InvalidPluginId {
                plugin_id: plugin_id.to_string(),
            })?;
        Ok(self
            .data_root
            .join("plugins")
            .join("data")
            .join(plugin_id.namespace())
            .join(plugin_id.name())
            .join("store.json"))
    }

    /// Returns the process-local serialization gate for one plugin identifier.
    fn plugin_lock(&self, plugin_id: &str) -> Result<Arc<Mutex<()>>, ConfigurationError> {
        // The store path is constructed from canonical plugin-id segments, so differently cased
        // requests use the same lock on case-insensitive filesystems as well as the same value file.
        let lock_key = self.store_path(plugin_id)?;
        let mut locks = self
            .locks
            .lock()
            .map_err(|_| ConfigurationError::LockUnavailable)?;
        Ok(locks
            .entry(lock_key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }
}

const MAX_STORE_BYTES: usize = 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::{
        ConfigurationCompleteness, ConfigurationDetails, ConfigurationService,
        ConfigurationSummary, EffectiveValueSource, SettingDetails,
    };
    use crate::{CompiledDeclaration, SettingDeclaration, SettingType, SettingValue};
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;

    /// A package declaration resolves defaults without creating a Stored Setting Value file.
    #[test]
    fn loads_declaration_defaults_without_persisting_them() {
        let temporary = TempDir::new().expect("create plugin configuration root");
        let package_root = temporary.path().join("package");
        fs::create_dir_all(package_root.join("assets")).expect("create package assets");
        fs::write(
            package_root.join("assets").join("config.json"),
            r#"{
              "schemaVersion": 1,
              "settings": {
                "endpoint": {"type":"string","title":"Endpoint","description":"Service URL","required":true},
                "retries": {"type":"number","title":"Retries","description":"Attempts","default":3}
              }
            }"#,
        )
        .expect("write declaration");
        let service = ConfigurationService::new(temporary.path());

        let details = service
            .get("official/weather", &package_root)
            .expect("load configuration")
            .expect("declaration exists");
        let endpoint = SettingDeclaration {
            id: "endpoint".to_string(),
            title: "Endpoint".to_string(),
            description: "Service URL".to_string(),
            setting_type: SettingType::String,
            required: true,
            order: None,
            default: None,
        };
        let retries = SettingDeclaration {
            id: "retries".to_string(),
            title: "Retries".to_string(),
            description: "Attempts".to_string(),
            setting_type: SettingType::Number,
            required: false,
            order: None,
            default: Some(SettingValue::Number(3.into())),
        };

        assert_eq!(
            details,
            ConfigurationDetails {
                declaration: CompiledDeclaration {
                    schema_version: 1,
                    settings: vec![endpoint.clone(), retries.clone()],
                    fingerprint: "ed254f53f4f9ff2e8e008641b3502d6f60dcb9ac77e9839fc524a940227833dd"
                        .to_string(),
                },
                settings: vec![
                    SettingDetails {
                        declaration: endpoint,
                        stored_value: None,
                        effective_value: None,
                        source: EffectiveValueSource::Absent,
                        value_error_code: None,
                    },
                    SettingDetails {
                        declaration: retries,
                        stored_value: None,
                        effective_value: Some(SettingValue::Number(3.into())),
                        source: EffectiveValueSource::Default,
                        value_error_code: None,
                    },
                ],
                revision: 0,
                summary: ConfigurationSummary::Available {
                    completeness: ConfigurationCompleteness::Incomplete,
                },
            }
        );
        assert!(!temporary.path().join("plugins").join("data").exists());
    }

    /// Save persists a complete explicit replacement and rejects a stale editor revision.
    #[test]
    fn saves_revisioned_overrides_and_rejects_stale_replacements() {
        let temporary = TempDir::new().expect("create plugin configuration root");
        let package_root = temporary.path().join("package");
        fs::create_dir_all(package_root.join("assets")).expect("create package assets");
        fs::write(
            package_root.join("assets").join("config.json"),
            r#"{"schemaVersion":1,"settings":{"endpoint":{"type":"string","title":"Endpoint","description":"Service URL","required":true},"enabled":{"type":"boolean","title":"Enabled","description":"Use it"}}}"#,
        )
        .expect("write declaration");
        let service = ConfigurationService::new(temporary.path());
        let loaded = service
            .get("official/weather", &package_root)
            .expect("load configuration")
            .expect("declaration exists");
        let values = BTreeMap::from([
            (
                "endpoint".to_string(),
                SettingValue::String("  https://example.com  ".to_string()),
            ),
            ("enabled".to_string(), SettingValue::Boolean(false)),
        ]);

        let saved = service
            .save(
                "official/weather",
                &package_root,
                /*expected_revision*/ 0,
                &loaded.declaration.fingerprint,
                values.clone(),
            )
            .expect("save configuration");

        assert_eq!(saved.revision, 1);
        assert_eq!(
            saved.summary,
            ConfigurationSummary::Available {
                completeness: ConfigurationCompleteness::Complete,
            }
        );
        assert_eq!(
            saved
                .settings
                .iter()
                .map(|setting| (
                    setting.declaration.id.as_str(),
                    setting.stored_value.clone(),
                    setting.source,
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "enabled",
                    Some(SettingValue::Boolean(false)),
                    EffectiveValueSource::Stored,
                ),
                (
                    "endpoint",
                    Some(SettingValue::String("  https://example.com  ".to_string())),
                    EffectiveValueSource::Stored,
                ),
            ]
        );
        assert!(
            temporary
                .path()
                .join("plugins")
                .join("data")
                .join("official")
                .join("weather")
                .join("store.json")
                .is_file()
        );
        assert!(matches!(
            service.save(
                "official/weather",
                &package_root,
                /*expected_revision*/ 0,
                &loaded.declaration.fingerprint,
                values,
            ),
            Err(super::ConfigurationError::RevisionConflict {
                expected: 0,
                actual: 1,
            })
        ));
    }

    /// Plugin ids whose name segment contains `.` use the same store layout as hyphenated names.
    #[test]
    fn persists_settings_for_dotted_plugin_name_segments() {
        let temporary = TempDir::new().expect("create plugin configuration root");
        let package_root = temporary.path().join("package");
        fs::create_dir_all(package_root.join("assets")).expect("create package assets");
        fs::write(
            package_root.join("assets").join("config.json"),
            r#"{
              "schemaVersion": 1,
              "settings": {
                "apiKey": {
                  "type": "string",
                  "title": "API key",
                  "description": "Credential",
                  "required": true
                }
              }
            }"#,
        )
        .expect("write declaration");
        let service = ConfigurationService::new(temporary.path());
        let loaded = service
            .get("official/ora-space.tavily-search", &package_root)
            .expect("load dotted plugin id")
            .expect("declaration is present");
        let saved = service
            .save(
                "official/ora-space.tavily-search",
                &package_root,
                loaded.revision,
                &loaded.declaration.fingerprint,
                BTreeMap::from([(
                    "apiKey".to_string(),
                    SettingValue::String("tvly-test".to_string()),
                )]),
            )
            .expect("save dotted plugin id settings");
        assert_eq!(
            saved.summary,
            ConfigurationSummary::Available {
                completeness: ConfigurationCompleteness::Complete,
            }
        );
        assert_eq!(
            fs::read_to_string(
                temporary
                    .path()
                    .join("plugins/data/official/ora-space.tavily-search/store.json")
            )
            .expect("read store.json"),
            r#"{"schemaVersion":1,"revision":1,"values":{"apiKey":"tvly-test"}}"#
        );
    }

    /// Upgrades hide removed values, surface incompatible values, and prune both on the next save.
    #[test]
    fn upgrade_values_are_retained_until_a_successful_current_declaration_save() {
        let temporary = TempDir::new().expect("create plugin configuration root");
        let package_root = temporary.path().join("package");
        fs::create_dir_all(package_root.join("assets")).expect("create package assets");
        fs::write(
            package_root.join("assets").join("config.json"),
            r#"{"schemaVersion":1,"settings":{"endpoint":{"type":"string","title":"Endpoint","description":"Service URL","required":true}}}"#,
        )
        .expect("write upgraded declaration");
        let store_root = temporary
            .path()
            .join("plugins")
            .join("data")
            .join("official")
            .join("weather");
        fs::create_dir_all(&store_root).expect("create store root");
        fs::write(
            store_root.join("store.json"),
            r#"{"schemaVersion":1,"revision":7,"values":{"endpoint":3,"removed":true}}"#,
        )
        .expect("write values from old declaration");
        let service = ConfigurationService::new(temporary.path());

        let loaded = service
            .get("official/weather", &package_root)
            .expect("load upgraded configuration")
            .expect("declaration exists");
        assert_eq!(loaded.settings.len(), 1);
        assert_eq!(
            loaded.settings[0].value_error_code.as_deref(),
            Some("stored_value_type_mismatch")
        );
        assert_eq!(
            loaded.summary,
            ConfigurationSummary::Available {
                completeness: ConfigurationCompleteness::Incomplete,
            }
        );

        service
            .save(
                "official/weather",
                &package_root,
                /*expected_revision*/ 7,
                &loaded.declaration.fingerprint,
                BTreeMap::new(),
            )
            .expect("save current declaration values");

        assert_eq!(
            fs::read_to_string(store_root.join("store.json")).unwrap(),
            r#"{"schemaVersion":1,"revision":8,"values":{}}"#
        );
    }

    /// Per-field limits cannot allow the complete Stored Setting Value file to exceed its bound.
    #[test]
    fn rejects_a_replacement_whose_serialized_store_exceeds_the_total_limit() {
        let temporary = TempDir::new().expect("create plugin configuration root");
        let package_root = temporary.path().join("package");
        fs::create_dir_all(package_root.join("assets")).expect("create package assets");
        let settings = (0..17)
            .map(|index| {
                (
                    format!("value{index}"),
                    serde_json::json!({
                        "type": "string",
                        "title": format!("Value {index}"),
                        "description": "Large bounded value",
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        fs::write(
            package_root.join("assets").join("config.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "settings": settings,
            }))
            .unwrap(),
        )
        .expect("write declaration");
        let service = ConfigurationService::new(temporary.path());
        let loaded = service
            .get("official/weather", &package_root)
            .expect("load configuration")
            .expect("declaration exists");
        let values = (0..17)
            .map(|index| {
                (
                    format!("value{index}"),
                    SettingValue::String("x".repeat(64 * 1024)),
                )
            })
            .collect();

        let error = service
            .save(
                "official/weather",
                &package_root,
                /*expected_revision*/ 0,
                &loaded.declaration.fingerprint,
                values,
            )
            .expect_err("reject oversized replacement");

        assert!(matches!(
            error,
            super::ConfigurationError::InvalidValues { ref field_errors }
                if field_errors.len() == 1
                    && field_errors[0].error_code == "configuration_too_large"
        ));
        assert!(
            !temporary
                .path()
                .join("plugins")
                .join("data")
                .join("official")
                .join("weather")
                .join("store.json")
                .exists()
        );
    }

    /// Corrupt storage remains unavailable until confirmed recovery preserves a timestamped backup.
    #[test]
    fn backs_up_corrupt_storage_before_recovery() {
        let temporary = TempDir::new().expect("create plugin configuration root");
        let package_root = temporary.path().join("package");
        fs::create_dir_all(package_root.join("assets")).expect("create package assets");
        fs::write(
            package_root.join("assets").join("config.json"),
            r#"{"schemaVersion":1,"settings":{"endpoint":{"type":"string","title":"Endpoint","description":"Service URL","required":true}}}"#,
        )
        .expect("write declaration");
        let store_root = temporary
            .path()
            .join("plugins")
            .join("data")
            .join("official")
            .join("weather");
        fs::create_dir_all(&store_root).expect("create store root");
        fs::write(store_root.join("store.json"), "{not json").expect("write corrupt store");
        fs::write(
            store_root.join("store.json.corrupt-20260824T200000"),
            "older backup",
        )
        .expect("write colliding backup");
        let service = ConfigurationService::new(temporary.path());
        let unavailable = service
            .get("official/weather", &package_root)
            .expect("load declaration around corrupt store")
            .expect("declaration exists");
        assert_eq!(
            unavailable.summary,
            ConfigurationSummary::Unavailable {
                error_code: "configuration_load_failed".to_string(),
            }
        );

        let recovered = service
            .recover_corrupt(
                "official/weather",
                &package_root,
                &unavailable.declaration.fingerprint,
                "20260824T200000",
            )
            .expect("recover corrupt configuration");

        assert_eq!(recovered.revision, 1);
        assert_eq!(
            fs::read_to_string(store_root.join("store.json.corrupt-20260824T200000"))
                .expect("read older backup"),
            "older backup"
        );
        assert_eq!(
            fs::read_to_string(store_root.join("store.json.corrupt-20260824T200000-1"))
                .expect("read collision-free backup"),
            "{not json"
        );
        assert_eq!(
            service
                .get("official/weather", &package_root)
                .expect("load recovered configuration")
                .expect("declaration exists"),
            recovered
        );
    }

    /// A package root that cannot be traversed is treated as undeclared, not load-failed.
    ///
    /// Linux reports `NotADirectory` when `assets/config.json` is opened through a file that
    /// replaced the package tree; Windows often reports `NotFound`. List summaries must stay
    /// `NotDeclared` in both cases so uninstall staging failures do not look like corrupt values.
    #[test]
    fn treats_non_directory_package_root_as_undeclared() {
        use crate::filesystem::ConfigurationFileSystem;
        use std::path::Path;

        #[derive(Clone, Copy)]
        struct NotADirectoryFileSystem;

        impl ConfigurationFileSystem for NotADirectoryFileSystem {
            fn read_bounded(
                &self,
                _path: &Path,
                _limit: usize,
            ) -> std::io::Result<Option<Vec<u8>>> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "package root is not a directory",
                ))
            }

            fn create_dir_all(&self, _path: &Path) -> std::io::Result<()> {
                unreachable!("declaration summary must not create directories")
            }

            fn atomic_write(&self, _path: &Path, _contents: &[u8]) -> std::io::Result<()> {
                unreachable!("declaration summary must not write")
            }

            fn move_no_replace(&self, _source: &Path, _destination: &Path) -> std::io::Result<()> {
                unreachable!("declaration summary must not move files")
            }
        }

        let temporary = TempDir::new().expect("create plugin configuration root");
        let package_root = temporary.path().join("package-as-file");
        let service =
            ConfigurationService::with_file_system(temporary.path(), NotADirectoryFileSystem);

        assert_eq!(
            service.summary("official/weather", &package_root),
            ConfigurationSummary::NotDeclared
        );
        assert_eq!(
            service
                .get("official/weather", &package_root)
                .expect("non-directory package root is undeclared"),
            None
        );
    }
}
