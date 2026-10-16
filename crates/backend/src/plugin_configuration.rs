use crate::error::{BackendError, ErrorClassification};
use crate::plugin::PluginApi;
use ora_contracts::{
    EmptyErrorParams, GetPluginConfigurationRequest, GetPluginConfigurationResponse,
    PluginConfigurationCompleteness, PluginConfigurationDetails, PluginConfigurationFieldError,
    PluginConfigurationSummary, PluginConfigurationValidationParams, PluginSettingDeclaration,
    PluginSettingDetails, PluginSettingType, PluginSettingValue, PluginSettingValueSource,
    PublicError, ResetPluginConfigurationMode, ResetPluginConfigurationRequest,
    ResetPluginConfigurationResponse, SavePluginConfigurationRequest,
    SavePluginConfigurationResponse,
};
use ora_plugin_config::{
    CompiledConfigurationFile, ConfigurationCompleteness, ConfigurationDetails, ConfigurationError,
    ConfigurationSummary, EffectiveValueSource, SettingType, SettingValue,
};
use std::collections::{BTreeMap, BTreeSet};

impl PluginApi {
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
        let redacted = mcp_bound_settings(&request.plugin_id, &package_root, details.revision)?;
        Ok(GetPluginConfigurationResponse {
            configuration: configuration_details(
                &request.plugin_id,
                details,
                redacted.as_ref().unwrap_or(&BTreeSet::new()),
            )?,
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
        let mut values = BTreeMap::new();
        let mut field_errors = Vec::new();
        for (setting_id, value) in request.values {
            if let Some(value) = setting_value(value) {
                values.insert(setting_id, value);
            } else {
                field_errors.push(PluginConfigurationFieldError {
                    setting_id,
                    error_code: "number_must_be_finite".to_string(),
                });
            }
        }
        if !field_errors.is_empty() {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::PluginConfigurationValidation(PluginConfigurationValidationParams {
                    field_errors,
                }),
                "plugin configuration contains non-finite numbers",
            ));
        }
        let details = self
            .configuration
            .save_preserving(
                &request.plugin_id,
                &package_root,
                request.expected_revision,
                &request.declaration_fingerprint,
                values,
                &request.preserve_setting_ids.into_iter().collect(),
            )
            .map_err(configuration_error)?;
        self.notify_mcp_desired_changed();
        self.notify_effect_reconcile();
        let redacted = mcp_bound_settings(&request.plugin_id, &package_root, details.revision)?;
        Ok(SavePluginConfigurationResponse {
            configuration: configuration_details(
                &request.plugin_id,
                details,
                redacted.as_ref().unwrap_or(&BTreeSet::new()),
            )?,
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
                self.configuration.recover_corrupt(
                    &request.plugin_id,
                    &package_root,
                    &request.declaration_fingerprint,
                    &ora_plugin_config::recovery_backup_label(
                        now.year(),
                        u8::from(now.month()),
                        now.day(),
                        now.hour(),
                        now.minute(),
                        now.second(),
                    ),
                )
            }
        }
        .map_err(configuration_error)?;
        self.notify_mcp_desired_changed();
        self.notify_effect_reconcile();
        let redacted = mcp_bound_settings(&request.plugin_id, &package_root, details.revision)?;
        Ok(ResetPluginConfigurationResponse {
            configuration: configuration_details(
                &request.plugin_id,
                details,
                redacted.as_ref().unwrap_or(&BTreeSet::new()),
            )?,
        })
    }
}

/// Maps the configuration module's deep value model onto the transport DTO family.
fn configuration_details(
    plugin_id: &str,
    details: ConfigurationDetails,
    redacted_setting_ids: &BTreeSet<String>,
) -> Result<PluginConfigurationDetails, BackendError> {
    let settings = details
        .settings
        .into_iter()
        .map(|setting| {
            let redacted = setting.declaration.secret
                || redacted_setting_ids.contains(&setting.declaration.id);
            Ok(PluginSettingDetails {
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
                    default: if redacted {
                        None
                    } else {
                        setting
                            .declaration
                            .default
                            .map(contract_setting_value)
                            .transpose()?
                    },
                },
                stored_value: if redacted {
                    None
                } else {
                    setting
                        .stored_value
                        .map(contract_setting_value)
                        .transpose()?
                },
                effective_value: if redacted {
                    None
                } else {
                    setting
                        .effective_value
                        .map(contract_setting_value)
                        .transpose()?
                },
                redacted,
                source: match setting.source {
                    EffectiveValueSource::Stored => PluginSettingValueSource::Stored,
                    EffectiveValueSource::Default => PluginSettingValueSource::Default,
                    EffectiveValueSource::Absent => PluginSettingValueSource::Absent,
                },
                value_error_code: setting.value_error_code,
            })
        })
        .collect::<Result<Vec<_>, BackendError>>()?;
    Ok(PluginConfigurationDetails {
        plugin_id: plugin_id.to_string(),
        schema_version: details.declaration.schema_version,
        revision: details.revision,
        declaration_fingerprint: details.declaration.fingerprint,
        settings,
        summary: contract_configuration_summary(details.summary),
    })
}

/// Identifies every MCP Setting so the editor DTO never carries those values.
fn mcp_bound_settings(
    _plugin_id: &str,
    package_root: &std::path::Path,
    _configuration_revision: u64,
) -> Result<Option<BTreeSet<String>>, BackendError> {
    let configuration =
        ora_plugin_config::ConfigurationService::configuration_file_from_package(package_root)
            .map_err(configuration_error)?;
    let Some(CompiledConfigurationFile::Mcp(configuration)) = configuration else {
        return Ok(None);
    };
    Ok(Some(
        configuration
            .settings
            .as_ref()
            .map(|declaration| {
                declaration
                    .settings
                    .iter()
                    .map(|setting| setting.id.clone())
                    .collect()
            })
            .unwrap_or_default(),
    ))
}

/// Maps one valid domain scalar into its JSON-compatible contract value.
fn contract_setting_value(value: SettingValue) -> Result<PluginSettingValue, BackendError> {
    match value {
        SettingValue::String(value) => Ok(PluginSettingValue::String(value)),
        SettingValue::Number(value) => value.as_f64().map(PluginSettingValue::Number).ok_or_else(
            || {
                BackendError::new(
                    ErrorClassification::Internal,
                    PublicError::ConfigurationLoadFailed(EmptyErrorParams {}),
                    "plugin configuration number cannot be represented by the transport contract",
                )
            },
        ),
        SettingValue::Boolean(value) => Ok(PluginSettingValue::Boolean(value)),
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
        ConfigurationError::InvalidPluginId { .. } => (
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            "plugin identifier is invalid",
        ),
        ConfigurationError::Io { .. }
        | ConfigurationError::LoadFailed { .. }
        | ConfigurationError::PersistFailed { .. }
        | ConfigurationError::RecoveryRestoreFailed { .. }
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
    use super::*;

    /// User-facing reads, CAS saves and reset all keep credentials out of every response.
    #[tokio::test]
    async fn skill_configuration_public_api_preserves_and_resets_secrets() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("plugins/installed/official/demo/1.0.0");
        std::fs::create_dir_all(package.join("assets/demo")).unwrap();
        std::fs::write(package.join("orax.toml"), "identifier = \"demo\"\nnamespace = \"official\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Demo\"\n").unwrap();
        std::fs::write(
            package.join("assets/demo/SKILL.md"),
            "---\nname: demo\ndescription: Demo\n---\n",
        )
        .unwrap();
        std::fs::write(package.join("assets/config.json"), r#"{"schemaVersion":1,"settings":{"token":{"type":"string","title":"Token","description":"Token","required":true,"secret":true}}}"#).unwrap();
        let backend =
            crate::Backend::open(crate::test_backend::backend_paths(root.path(), root.path()))
                .unwrap();
        let plugins = backend.plugins();
        let initial = plugins
            .get_configuration(GetPluginConfigurationRequest {
                plugin_id: "official/demo".into(),
            })
            .unwrap()
            .configuration;
        let request = SavePluginConfigurationRequest {
            plugin_id: "official/demo".into(),
            expected_revision: initial.revision,
            declaration_fingerprint: initial.declaration_fingerprint.clone(),
            values: BTreeMap::from([(
                "token".into(),
                PluginSettingValue::String("test-secret-value".into()),
            )]),
            preserve_setting_ids: Vec::new(),
        };
        let saved = plugins
            .save_configuration(request.clone())
            .unwrap()
            .configuration;
        assert!(
            !serde_json::to_string(&saved)
                .unwrap()
                .contains("test-secret-value")
        );
        assert!(plugins.save_configuration(request).is_err());
        let preserved = plugins
            .save_configuration(SavePluginConfigurationRequest {
                plugin_id: "official/demo".into(),
                expected_revision: saved.revision,
                declaration_fingerprint: saved.declaration_fingerprint.clone(),
                values: BTreeMap::new(),
                preserve_setting_ids: vec!["token".into()],
            })
            .unwrap()
            .configuration;
        assert!(preserved.settings[0].redacted);
        assert!(matches!(
            preserved.settings[0].source,
            PluginSettingValueSource::Stored
        ));
        let reset = plugins
            .reset_configuration(ResetPluginConfigurationRequest {
                plugin_id: "official/demo".into(),
                declaration_fingerprint: preserved.declaration_fingerprint,
                reset: ResetPluginConfigurationMode::ResetAll {
                    expected_revision: preserved.revision,
                },
            })
            .unwrap()
            .configuration;
        assert!(matches!(
            reset.settings[0].source,
            PluginSettingValueSource::Absent
        ));
        assert!(
            !serde_json::to_string(&reset)
                .unwrap()
                .contains("test-secret-value")
        );
        let read = plugins
            .get_configuration(GetPluginConfigurationRequest {
                plugin_id: "official/demo".into(),
            })
            .unwrap();
        assert!(
            !serde_json::to_string(&read)
                .unwrap()
                .contains("test-secret-value")
        );
        assert!(
            !std::fs::read_to_string(package.join("assets/demo/SKILL.md"))
                .unwrap()
                .contains("test-secret-value")
        );
    }

    /// All secret scalar types hide defaults and values while public fields remain readable.
    #[test]
    fn secret_settings_are_redacted_in_editor_projection() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("assets")).unwrap();
        std::fs::write(root.path().join("assets/config.json"), r#"{"schemaVersion":1,"settings":{
            "token":{"type":"string","title":"Token","description":"Token","secret":true,"default":"private"},
            "number":{"type":"number","title":"Number","description":"Number","secret":true,"default":42},
            "flag":{"type":"boolean","title":"Flag","description":"Flag","secret":true,"default":true},
            "url":{"type":"string","title":"URL","description":"URL","default":"https://example.test"}
        }}"#).unwrap();
        let service = ora_plugin_config::ConfigurationService::new(root.path());
        let details = service.get("official/demo", root.path()).unwrap().unwrap();
        let dto = configuration_details("official/demo", details, &BTreeSet::new()).unwrap();
        for field in dto.settings {
            if field.declaration.id == "url" {
                assert!(!field.redacted);
                assert!(field.effective_value.is_some());
            } else {
                assert!(field.redacted);
                assert!(field.declaration.default.is_none());
                assert!(field.stored_value.is_none());
                assert!(field.effective_value.is_none());
            }
        }
    }
}
