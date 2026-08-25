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
    ConfigurationCompleteness, ConfigurationDetails, ConfigurationError, ConfigurationSummary,
    EffectiveValueSource, SettingType, SettingValue,
};
use std::collections::BTreeMap;

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
        Ok(GetPluginConfigurationResponse {
            configuration: configuration_details(&request.plugin_id, details)?,
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
            .save(
                &request.plugin_id,
                &package_root,
                request.expected_revision,
                &request.declaration_fingerprint,
                values,
            )
            .map_err(configuration_error)?;
        Ok(SavePluginConfigurationResponse {
            configuration: configuration_details(&request.plugin_id, details)?,
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
        Ok(ResetPluginConfigurationResponse {
            configuration: configuration_details(&request.plugin_id, details)?,
        })
    }
}

/// Maps the configuration module's deep value model onto the transport DTO family.
fn configuration_details(
    plugin_id: &str,
    details: ConfigurationDetails,
) -> Result<PluginConfigurationDetails, BackendError> {
    let settings = details
        .settings
        .into_iter()
        .map(|setting| {
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
                    default: setting
                        .declaration
                        .default
                        .map(contract_setting_value)
                        .transpose()?,
                },
                stored_value: setting
                    .stored_value
                    .map(contract_setting_value)
                    .transpose()?,
                effective_value: setting
                    .effective_value
                    .map(contract_setting_value)
                    .transpose()?,
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
