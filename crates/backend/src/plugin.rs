use crate::app_event::AppEventPublisher;
use crate::clock::SystemClock;
use crate::error::{BackendError, ErrorClassification};
use gitlancer::{BranchName, CliGitRunner, Git};
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, DisablePluginRequest, DisablePluginResponse,
    EmptyErrorParams, EnablePluginRequest, EnablePluginResponse, GetPluginConfigurationRequest,
    GetPluginConfigurationResponse, InstallPluginRequest, InstallPluginResponse,
    ListAvailablePluginsRequest, ListAvailablePluginsResponse, ListInstalledPluginsRequest,
    ListInstalledPluginsResponse, PluginConfigurationCompleteness, PluginConfigurationDetails,
    PluginConfigurationFieldError, PluginConfigurationSummary, PluginConfigurationValidationParams,
    PluginSettingDeclaration, PluginSettingDetails, PluginSettingType, PluginSettingValue,
    PluginSettingValueSource, PublicError, ResetPluginConfigurationMode,
    ResetPluginConfigurationRequest, ResetPluginConfigurationResponse,
    SavePluginConfigurationRequest, SavePluginConfigurationResponse, ScanPluginsRequest,
    ScanPluginsResponse, StopPluginRequest, StopPluginResponse, SyncAvailablePluginsRequest,
    SyncAvailablePluginsResponse, UninstallPluginRequest, UninstallPluginResponse,
};
use ora_db::{RepositoryPool, SqlitePluginStateRepository};
use ora_domain::PluginId;
use ora_logging::{ora_info, ora_warn};
use ora_plugin_config::{
    ConfigurationCompleteness, ConfigurationDetails, ConfigurationError, ConfigurationService,
    ConfigurationSummary, EffectiveValueSource, SettingType, SettingValue,
};
use ora_plugin_lifecycle::{
    DenoPluginRuntime, DenoPluginRuntimeLauncher, PluginAttachment, PluginLifecycle,
    PluginLifecycleConfig, PluginLifecycleError, PluginRuntimeTimeouts,
};
use ora_plugin_manager::Installer;
use ora_plugin_registry::{
    RegistryEntry, RegistryError, RegistryIndex, RegistrySource, RegistrySync,
};
use ora_utils::http::{DownloadSource, ProxyConfig, ReqwestDownloader};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The marketplace repository mirrored into the local registry source checkout.
const MARKETPLACE_REPOSITORY_URL: &str = "https://github.com/ora-space/marketplace";
/// The branch tracked by the marketplace registry source.
const MARKETPLACE_REPOSITORY_BRANCH: &str = "main";

/// Groups plugin discovery and lifecycle operations behind the backend's plugin interface.
pub(crate) struct PluginApi {
    lifecycle: PluginLifecycle<
        SqlitePluginStateRepository,
        SystemClock,
        DenoPluginRuntimeLauncher,
        AppEventPublisher,
    >,
    registry_source: RegistrySource,
    registry_index_path: PathBuf,
    data_directory: PathBuf,
    installer: Installer<ReqwestDownloader>,
    configuration: ConfigurationService,
}

impl PluginApi {
    /// Opens plugin lifecycle state with the concrete backend adapters.
    pub(crate) fn open(
        pool: RepositoryPool,
        data_directory: PathBuf,
        deno_path: PathBuf,
        clock: SystemClock,
        publisher: AppEventPublisher,
    ) -> Result<Self, PluginLifecycleError> {
        let plugins_directory = data_directory.join("plugins");
        let configuration = ConfigurationService::new(data_directory.clone());
        let registry_source = RegistrySource::new(
            MARKETPLACE_REPOSITORY_URL,
            BranchName::new(MARKETPLACE_REPOSITORY_BRANCH),
            plugins_directory
                .join("sources")
                .join("github.com")
                .join("ora-space")
                .join("marketplace"),
        );
        let registry_index_path = plugins_directory.join("cache").join("registry_index.json");
        let installer = Installer::new(ReqwestDownloader::new(ProxyConfig::default()));
        let lifecycle = PluginLifecycle::open(
            PluginLifecycleConfig {
                data_directory: data_directory.clone(),
                deno_path,
            },
            SqlitePluginStateRepository::new(pool),
            clock,
            DenoPluginRuntimeLauncher::new(PluginRuntimeTimeouts::default()),
            publisher,
        )?;

        Ok(Self {
            lifecycle,
            registry_source,
            registry_index_path,
            data_directory,
            installer,
            configuration,
        })
    }

    /// Returns the cached marketplace registry index, or an empty catalog when absent.
    pub(crate) fn list_available_plugins(
        &self,
        _request: ListAvailablePluginsRequest,
    ) -> Result<ListAvailablePluginsResponse, RegistryError> {
        match RegistryIndex::load(&self.registry_index_path) {
            Ok(index) => Ok(ListAvailablePluginsResponse {
                updated_at: index.updated_at(),
                plugins: index.plugins().iter().map(available_plugin).collect(),
            }),
            Err(RegistryError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ListAvailablePluginsResponse {
                    updated_at: 0,
                    plugins: Vec::new(),
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Pulls the marketplace source, rebuilds its registry index, and atomically replaces the cache.
    pub(crate) fn sync_available_plugins(
        &self,
        _request: SyncAvailablePluginsRequest,
    ) -> Result<SyncAvailablePluginsResponse, RegistryError> {
        let checkout_directory =
            RegistrySync::sync(&Git::new(CliGitRunner), &self.registry_source)?;
        let build = RegistryIndex::build(
            &checkout_directory.join("registry"),
            ora_logging::clock::now_local().unix_timestamp(),
        );
        if let Some(cache_directory) = self.registry_index_path.parent() {
            std::fs::create_dir_all(cache_directory)?;
        }
        build.index().write(&self.registry_index_path)?;
        Ok(SyncAvailablePluginsResponse {
            updated_at: build.index().updated_at(),
            plugins: build
                .index()
                .plugins()
                .iter()
                .map(available_plugin)
                .collect(),
        })
    }

    /// Returns the cached installed-plugin snapshot without rescanning the filesystem.
    pub(crate) fn list(
        &self,
        _request: ListInstalledPluginsRequest,
    ) -> ListInstalledPluginsResponse {
        self.lifecycle.list_installed_plugins()
    }

    /// Rescans packages and reconciles durable and runtime state.
    pub(crate) async fn scan(
        &self,
        request: ScanPluginsRequest,
    ) -> Result<ScanPluginsResponse, PluginLifecycleError> {
        self.lifecycle.scan_plugins(request).await
    }

    /// Persists plugin eligibility and starts the runtime it implies.
    pub(crate) async fn enable(
        &self,
        request: EnablePluginRequest,
    ) -> Result<EnablePluginResponse, PluginLifecycleError> {
        self.lifecycle.enable_plugin(request).await
    }

    /// Stops a plugin when necessary before persisting ineligibility.
    pub(crate) async fn disable(
        &self,
        request: DisablePluginRequest,
    ) -> Result<DisablePluginResponse, PluginLifecycleError> {
        self.lifecycle.disable_plugin(request).await
    }

    /// Starts one enabled plugin and returns its immediate starting state.
    pub(crate) async fn activate(
        &self,
        request: ActivatePluginRequest,
    ) -> Result<ActivatePluginResponse, PluginLifecycleError> {
        self.lifecycle.activate_plugin(request).await
    }

    /// Returns a running plugin runtime plus the unclaimed notification stream of that launch.
    ///
    /// This is the single seam through which the agent runtime reaches a plugin process. The
    /// process stays owned by the lifecycle, so an agent connection can never leave one running
    /// that the settings surface reports as stopped.
    pub(crate) async fn attach_runtime(
        &self,
        plugin_id: &PluginId,
    ) -> Result<PluginAttachment<DenoPluginRuntime>, PluginLifecycleError> {
        self.lifecycle.attach_runtime(plugin_id).await
    }

    /// Stops one plugin process without changing durable eligibility.
    pub(crate) async fn stop(
        &self,
        request: StopPluginRequest,
    ) -> Result<StopPluginResponse, PluginLifecycleError> {
        self.lifecycle.stop_plugin(request).await
    }

    /// Stops and removes one plugin package plus its durable state.
    pub(crate) async fn uninstall(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, PluginLifecycleError> {
        self.lifecycle.uninstall_plugin(request).await
    }

    /// Returns one typed Plugin Configuration editor snapshot.
    pub(crate) fn get_configuration(
        &self,
        request: GetPluginConfigurationRequest,
    ) -> Result<GetPluginConfigurationResponse, BackendError> {
        let package_root = self
            .lifecycle
            .installed_package_root(&request.plugin_id)
            .map_err(BackendError::from)?;
        let details = self
            .configuration
            .get(&request.plugin_id, &package_root)
            .map_err(configuration_error)?
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::InvalidRequest,
                    PublicError::PluginConfigurationNotDeclared(EmptyErrorParams {}),
                    "plugin does not declare configuration",
                )
            })?;
        Ok(GetPluginConfigurationResponse {
            configuration: configuration_details(&request.plugin_id, details),
        })
    }

    /// Validates and persists a complete explicit override replacement.
    pub(crate) fn save_configuration(
        &self,
        request: SavePluginConfigurationRequest,
    ) -> Result<SavePluginConfigurationResponse, BackendError> {
        let package_root = self
            .lifecycle
            .installed_package_root(&request.plugin_id)
            .map_err(BackendError::from)?;
        let values = request
            .values
            .into_iter()
            .map(|(setting_id, value)| match setting_value(value) {
                Some(value) => Ok((setting_id, value)),
                None => Err(setting_id),
            })
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|setting_id| {
                BackendError::new(
                    ErrorClassification::InvalidRequest,
                    PublicError::PluginConfigurationValidation(
                        PluginConfigurationValidationParams {
                            field_errors: vec![PluginConfigurationFieldError {
                                setting_id,
                                error_code: "number_must_be_finite".to_string(),
                            }],
                        },
                    ),
                    "plugin configuration contains a non-finite number",
                )
            })?;
        let details = self
            .configuration
            .save(
                &request.plugin_id,
                &package_root,
                request.expected_revision,
                &request.declaration_fingerprint,
                values,
            )
            .map_err(configuration_error)?;
        Ok(SavePluginConfigurationResponse {
            configuration: configuration_details(&request.plugin_id, details),
        })
    }

    /// Executes Reset All or confirmed damaged-data recovery as an explicit domain operation.
    pub(crate) fn reset_configuration(
        &self,
        request: ResetPluginConfigurationRequest,
    ) -> Result<ResetPluginConfigurationResponse, BackendError> {
        let package_root = self
            .lifecycle
            .installed_package_root(&request.plugin_id)
            .map_err(BackendError::from)?;
        let details = match request.reset {
            ResetPluginConfigurationMode::ResetAll { expected_revision } => {
                self.configuration.reset_all(
                    &request.plugin_id,
                    &package_root,
                    expected_revision,
                    &request.declaration_fingerprint,
                )
            }
            ResetPluginConfigurationMode::RecoverCorrupt => {
                let now = ora_logging::clock::now_local();
                let timestamp = format!(
                    "{:04}{:02}{:02}T{:02}{:02}{:02}",
                    now.year(),
                    u8::from(now.month()),
                    now.day(),
                    now.hour(),
                    now.minute(),
                    now.second(),
                );
                self.configuration.recover_corrupt(
                    &request.plugin_id,
                    &package_root,
                    &request.declaration_fingerprint,
                    &timestamp,
                )
            }
        }
        .map_err(configuration_error)?;
        Ok(ResetPluginConfigurationResponse {
            configuration: configuration_details(&request.plugin_id, details),
        })
    }
    /// Installs a marketplace plugin by resolving its release manifest from the synced source and
    /// downloading, verifying, and extracting its package through the network-backed installer.
    ///
    /// The source registry is read only for the release `url`/`sha256` (the cached index carries
    /// display fields only), so this returns NotFound when the identifier is not in the checkout.
    pub(crate) async fn install(
        &self,
        request: InstallPluginRequest,
    ) -> Result<InstallPluginResponse, BackendError> {
        let registry_directory = self.registry_source.checkout_dir().join("registry");
        let manifest = RegistryIndex::resolve_manifest(&registry_directory, &request.plugin_id)
            .map_err(|error| {
                BackendError::internal("failed to resolve plugin release manifest", error)
            })?
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::NotFound,
                    PublicError::PluginNotFound(EmptyErrorParams {}),
                    "marketplace plugin was not found in the registry",
                )
            })?;
        let release_url = manifest
            .url()
            .ok_or_else(|| {
                BackendError::internal(
                    "marketplace plugin manifest is missing its release url",
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "missing release url"),
                )
            })?
            .as_url();
        ora_info!(plugin_id = %request.plugin_id, url = %release_url, "installing marketplace plugin");
        self.installer
            .install(
                &manifest,
                DownloadSource::Url(release_url.clone()),
                &self.data_directory,
            )
            .await
            .map_err(|error| BackendError::internal("failed to install plugin", error))?;
        // The installed snapshot is built once at startup, so a fresh install must re-scan for the
        // new package to appear in the installed list without restarting the backend.
        if let Err(error) = self.lifecycle.scan_plugins(ScanPluginsRequest {}).await {
            ora_warn!(plugin_id = %request.plugin_id, %error, "installed the package but failed to refresh the installed-plugin snapshot");
        }
        ora_info!(plugin_id = %request.plugin_id, "installed marketplace plugin");
        Ok(InstallPluginResponse {
            plugin_id: request.plugin_id,
        })
    }
}

/// Converts one registry entry into the frontend-facing marketplace summary.
fn available_plugin(entry: &RegistryEntry) -> ora_contracts::AvailablePlugin {
    ora_contracts::AvailablePlugin {
        id: entry.id().to_owned(),
        name: entry.name().to_owned(),
        namespace: entry.namespace().to_owned(),
        version: entry.version().to_string(),
        description: entry.description().to_owned(),
        logo: entry.logo().map(str::to_owned),
    }
}

/// Maps the configuration module's deep value model onto the transport DTO family.
fn configuration_details(
    plugin_id: &str,
    details: ConfigurationDetails,
) -> PluginConfigurationDetails {
    PluginConfigurationDetails {
        plugin_id: plugin_id.to_string(),
        schema_version: details.declaration.schema_version,
        revision: details.revision,
        declaration_fingerprint: details.declaration.fingerprint,
        settings: details
            .settings
            .into_iter()
            .map(|setting| PluginSettingDetails {
                declaration: PluginSettingDeclaration {
                    id: setting.declaration.id,
                    title: setting.declaration.title,
                    description: setting.declaration.description,
                    setting_type: match setting.declaration.setting_type {
                        SettingType::String => PluginSettingType::String,
                        SettingType::Number => PluginSettingType::Number,
                        SettingType::Boolean => PluginSettingType::Boolean,
                    },
                    required: setting.declaration.required,
                    order: setting.declaration.order,
                    default: setting.declaration.default.map(contract_setting_value),
                },
                stored_value: setting.stored_value.map(contract_setting_value),
                effective_value: setting.effective_value.map(contract_setting_value),
                source: match setting.source {
                    EffectiveValueSource::Stored => PluginSettingValueSource::Stored,
                    EffectiveValueSource::Default => PluginSettingValueSource::Default,
                    EffectiveValueSource::Absent => PluginSettingValueSource::Absent,
                },
                value_error_code: setting.value_error_code,
            })
            .collect(),
        summary: contract_configuration_summary(details.summary),
    }
}

/// Converts one core scalar into its frontend-facing number representation.
fn contract_setting_value(value: SettingValue) -> PluginSettingValue {
    match value {
        SettingValue::String(value) => PluginSettingValue::String(value),
        SettingValue::Number(value) => PluginSettingValue::Number(
            value
                .as_f64()
                .unwrap_or_else(|| panic!("a JSON number must have an f64 representation")),
        ),
        SettingValue::Boolean(value) => PluginSettingValue::Boolean(value),
    }
}

/// Converts one transport scalar without permitting non-finite numbers into storage.
fn setting_value(value: PluginSettingValue) -> Option<SettingValue> {
    match value {
        PluginSettingValue::String(value) => Some(SettingValue::String(value)),
        PluginSettingValue::Number(value) => {
            serde_json::Number::from_f64(value).map(SettingValue::Number)
        }
        PluginSettingValue::Boolean(value) => Some(SettingValue::Boolean(value)),
    }
}

/// Maps the exclusive configuration summary without manufacturing boolean combinations.
fn contract_configuration_summary(summary: ConfigurationSummary) -> PluginConfigurationSummary {
    match summary {
        ConfigurationSummary::NotDeclared => PluginConfigurationSummary::NotDeclared,
        ConfigurationSummary::Available { completeness } => PluginConfigurationSummary::Available {
            completeness: match completeness {
                ConfigurationCompleteness::Complete => PluginConfigurationCompleteness::Complete,
                ConfigurationCompleteness::Incomplete => {
                    PluginConfigurationCompleteness::Incomplete
                }
            },
        },
        ConfigurationSummary::Unavailable { error_code } => {
            PluginConfigurationSummary::Unavailable { error_code }
        }
    }
}

/// Preserves stable Plugin Configuration failures and Setting-addressed validation details.
fn configuration_error(error: ConfigurationError) -> BackendError {
    let (classification, public_error, context) = match &error {
        ConfigurationError::InvalidDeclaration(_) => (
            ErrorClassification::InvalidRequest,
            PublicError::PluginConfigurationDeclarationInvalid(EmptyErrorParams {}),
            "plugin configuration declaration is invalid",
        ),
        ConfigurationError::NotDeclared => (
            ErrorClassification::InvalidRequest,
            PublicError::PluginConfigurationNotDeclared(EmptyErrorParams {}),
            "plugin does not declare configuration",
        ),
        ConfigurationError::DeclarationChanged => (
            ErrorClassification::Conflict,
            PublicError::PluginConfigurationDeclarationChanged(EmptyErrorParams {}),
            "plugin configuration declaration changed",
        ),
        ConfigurationError::RevisionConflict { .. } => (
            ErrorClassification::Conflict,
            PublicError::ConfigurationRevisionConflict(EmptyErrorParams {}),
            "plugin configuration revision conflict",
        ),
        ConfigurationError::InvalidValues { field_errors } => (
            ErrorClassification::InvalidRequest,
            PublicError::PluginConfigurationValidation(PluginConfigurationValidationParams {
                field_errors: field_errors
                    .iter()
                    .map(|field| PluginConfigurationFieldError {
                        setting_id: field.setting_id.clone(),
                        error_code: field.error_code.clone(),
                    })
                    .collect(),
            }),
            "plugin configuration values are invalid",
        ),
        ConfigurationError::RecoveryNotRequired => (
            ErrorClassification::InvalidRequest,
            PublicError::PluginConfigurationRecoveryNotRequired(EmptyErrorParams {}),
            "plugin configuration recovery is not required",
        ),
        ConfigurationError::Io { .. }
        | ConfigurationError::LoadFailed { .. }
        | ConfigurationError::RevisionExhausted
        | ConfigurationError::LockUnavailable => (
            ErrorClassification::Unprocessable,
            PublicError::ConfigurationLoadFailed(EmptyErrorParams {}),
            "plugin configuration could not be loaded or persisted",
        ),
    };
    BackendError::with_source(classification, public_error, context, error)
}

#[cfg(test)]
mod tests {
    use super::PluginApi;
    use crate::app_event::AppEventHub;
    use crate::clock::SystemClock;
    use ora_contracts::{
        EmptyErrorParams, GetPluginConfigurationRequest, PluginConfigurationCompleteness,
        PluginConfigurationDetails, PluginConfigurationSummary, PluginSettingDeclaration,
        PluginSettingDetails, PluginSettingType, PluginSettingValue, PluginSettingValueSource,
        PublicError, SavePluginConfigurationRequest,
    };
    use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// The public PluginApi joins package declarations and plugin-global values for editor callers.
    #[test]
    fn returns_configuration_details_through_the_plugin_api() {
        let temporary = TempDir::new().expect("create plugin API root");
        let package_root = write_plugin_package(temporary.path());
        fs::create_dir_all(package_root.join("assets")).expect("create package assets");
        fs::write(
            package_root.join("assets").join("config.json"),
            r#"{"schemaVersion":1,"settings":{"endpoint":{"type":"string","title":"Endpoint","description":"Service URL","required":true},"retries":{"type":"number","title":"Retries","description":"Attempts","default":3}}}"#,
        )
        .expect("write configuration declaration");
        let api = open_plugin_api(temporary.path());

        let response = api
            .get_configuration(GetPluginConfigurationRequest {
                plugin_id: "official/weather".to_string(),
            })
            .expect("load Plugin Configuration");

        assert_eq!(
            response.configuration,
            PluginConfigurationDetails {
                plugin_id: "official/weather".to_string(),
                schema_version: 1,
                revision: 0,
                declaration_fingerprint:
                    "ed254f53f4f9ff2e8e008641b3502d6f60dcb9ac77e9839fc524a940227833dd".to_string(),
                settings: vec![
                    PluginSettingDetails {
                        declaration: PluginSettingDeclaration {
                            id: "endpoint".to_string(),
                            title: "Endpoint".to_string(),
                            description: "Service URL".to_string(),
                            setting_type: PluginSettingType::String,
                            required: true,
                            order: None,
                            default: None,
                        },
                        stored_value: None,
                        effective_value: None,
                        source: PluginSettingValueSource::Absent,
                        value_error_code: None,
                    },
                    PluginSettingDetails {
                        declaration: PluginSettingDeclaration {
                            id: "retries".to_string(),
                            title: "Retries".to_string(),
                            description: "Attempts".to_string(),
                            setting_type: PluginSettingType::Number,
                            required: false,
                            order: None,
                            default: Some(PluginSettingValue::Number(3.0)),
                        },
                        stored_value: None,
                        effective_value: Some(PluginSettingValue::Number(3.0)),
                        source: PluginSettingValueSource::Default,
                        value_error_code: None,
                    },
                ],
                summary: PluginConfigurationSummary::Available {
                    completeness: PluginConfigurationCompleteness::Incomplete,
                },
            }
        );
    }

    /// The PluginApi persists whole replacements and exposes stale revisions as stable conflicts.
    #[test]
    fn saves_configuration_and_rejects_a_stale_plugin_api_editor() {
        let temporary = TempDir::new().expect("create plugin API root");
        let package_root = write_plugin_package(temporary.path());
        fs::create_dir_all(package_root.join("assets")).expect("create package assets");
        fs::write(
            package_root.join("assets").join("config.json"),
            r#"{"schemaVersion":1,"settings":{"endpoint":{"type":"string","title":"Endpoint","description":"Service URL","required":true}}}"#,
        )
        .expect("write configuration declaration");
        let api = open_plugin_api(temporary.path());
        let loaded = api
            .get_configuration(GetPluginConfigurationRequest {
                plugin_id: "official/weather".to_string(),
            })
            .expect("load Plugin Configuration")
            .configuration;
        let request = SavePluginConfigurationRequest {
            plugin_id: "official/weather".to_string(),
            expected_revision: 0,
            declaration_fingerprint: loaded.declaration_fingerprint,
            values: BTreeMap::from([(
                "endpoint".to_string(),
                PluginSettingValue::String(" https://api.test ".to_string()),
            )]),
        };

        let saved = api
            .save_configuration(request.clone())
            .expect("save Plugin Configuration");
        assert_eq!(saved.configuration.revision, 1);
        assert_eq!(
            saved.configuration.summary,
            PluginConfigurationSummary::Available {
                completeness: PluginConfigurationCompleteness::Complete,
            }
        );
        let conflict = api
            .save_configuration(request)
            .expect_err("stale editor must conflict");
        assert_eq!(
            conflict.public_error(),
            &PublicError::ConfigurationRevisionConflict(EmptyErrorParams {})
        );
    }

    /// Opens the concrete plugin API over one isolated migrated data root.
    fn open_plugin_api(root: &Path) -> PluginApi {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(root.join("test.sqlite")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap plugin API database");
        PluginApi::open(
            pool,
            root.to_path_buf(),
            PathBuf::from("deno"),
            SystemClock,
            AppEventHub::new().publisher(),
        )
        .expect("open plugin API")
    }

    /// Writes one fully discoverable Agent Plugin package and returns its immutable root.
    fn write_plugin_package(data_root: &Path) -> PathBuf {
        let package_root = data_root
            .join("plugins")
            .join("installed")
            .join("official")
            .join("weather")
            .join("1.0.0");
        fs::create_dir_all(&package_root).expect("create installed package");
        fs::write(package_root.join("main.js"), "export {};\n").expect("write plugin entrypoint");
        fs::write(
            package_root.join("orax.toml"),
            "resolver = 1\nname = \"weather\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Weather\"\n",
        )
        .expect("write plugin manifest");
        package_root
    }
}
