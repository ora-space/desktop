use crate::{
    CompiledDeclaration, ConfigurationCompleteness, ConfigurationDetails, ConfigurationFieldError,
    ConfigurationSummary, EffectiveValueSource, SettingDeclaration, SettingDetails, SettingValue,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Represents the complete host-owned override file independently of one installed version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StoredConfiguration {
    pub(super) schema_version: u32,
    pub(super) revision: u64,
    pub(super) values: BTreeMap<String, SettingValue>,
}

impl Default for StoredConfiguration {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            values: BTreeMap::new(),
        }
    }
}

/// Projects stored overrides and defaults while retaining incompatible upgrade values as errors.
pub(super) fn details_from(
    declaration: CompiledDeclaration,
    store: StoredConfiguration,
) -> ConfigurationDetails {
    let settings = declaration
        .settings
        .iter()
        .cloned()
        .map(|setting| {
            let stored = store.values.get(&setting.id).cloned();
            let compatible = stored
                .as_ref()
                .is_none_or(|value| value_matches(&setting, value));
            let (stored_value, effective_value, source, value_error_code) = if compatible {
                match stored {
                    Some(value) => (
                        Some(value.clone()),
                        Some(value),
                        EffectiveValueSource::Stored,
                        None,
                    ),
                    None => match setting.default.clone() {
                        Some(value) => (None, Some(value), EffectiveValueSource::Default, None),
                        None => (None, None, EffectiveValueSource::Absent, None),
                    },
                }
            } else {
                let (effective_value, source) = match setting.default.clone() {
                    Some(value) => (Some(value), EffectiveValueSource::Default),
                    None => (None, EffectiveValueSource::Absent),
                };
                (
                    None,
                    effective_value,
                    source,
                    Some("stored_value_type_mismatch".to_string()),
                )
            };
            SettingDetails {
                declaration: setting,
                stored_value,
                effective_value,
                source,
                value_error_code,
            }
        })
        .collect::<Vec<_>>();
    let completeness = completeness(&settings);
    ConfigurationDetails {
        declaration,
        settings,
        revision: store.revision,
        summary: ConfigurationSummary::Available { completeness },
    }
}

/// Validates that the complete submitted override set is recognized and type-correct.
pub(super) fn validate_values(
    declaration: &CompiledDeclaration,
    values: &BTreeMap<String, SettingValue>,
) -> Vec<ConfigurationFieldError> {
    let declarations = declaration
        .settings
        .iter()
        .map(|setting| (setting.id.as_str(), setting))
        .collect::<BTreeMap<_, _>>();
    values
        .iter()
        .filter_map(|(setting_id, value)| match declarations.get(setting_id.as_str()) {
            None => Some(ConfigurationFieldError {
                setting_id: setting_id.clone(),
                error_code: "setting_not_declared".to_string(),
            }),
            Some(setting) if !value_matches(setting, value) => Some(ConfigurationFieldError {
                setting_id: setting_id.clone(),
                error_code: "setting_type_mismatch".to_string(),
            }),
            Some(_)
                if matches!(value, SettingValue::String(value) if value.len() > MAX_STRING_VALUE_BYTES) =>
            {
                Some(ConfigurationFieldError {
                    setting_id: setting_id.clone(),
                    error_code: "string_value_too_large".to_string(),
                })
            }
            Some(_) => None,
        })
        .collect()
}

/// Checks one scalar against its declaration without converting between JSON types.
fn value_matches(declaration: &SettingDeclaration, value: &SettingValue) -> bool {
    matches!(
        (declaration.setting_type, value),
        (crate::SettingType::String, SettingValue::String(_))
            | (crate::SettingType::Number, SettingValue::Number(_))
            | (crate::SettingType::Boolean, SettingValue::Boolean(_))
    )
}

/// Evaluates required values and any stored type failures from the projected editor fields.
fn completeness(settings: &[SettingDetails]) -> ConfigurationCompleteness {
    let incomplete = settings.iter().any(|setting| {
        setting.value_error_code.is_some()
            || (setting.declaration.required
                && match setting.effective_value.as_ref() {
                    Some(SettingValue::String(value)) => value.trim().is_empty(),
                    Some(SettingValue::Number(_) | SettingValue::Boolean(_)) => false,
                    None => true,
                })
    });
    if incomplete {
        ConfigurationCompleteness::Incomplete
    } else {
        ConfigurationCompleteness::Complete
    }
}

const MAX_STRING_VALUE_BYTES: usize = 64 * 1024;
