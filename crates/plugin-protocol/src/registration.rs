use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use ts_rs::TS;

/// Immutable capabilities published by a plugin after its registration is validated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginRegistration {
    pub methods: HashSet<String>,
    pub emits: HashSet<String>,
}

/// Wire payload of the `ora/register` notification.
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "registration.ts")]
pub struct PluginRegistrationParams {
    pub methods: Vec<String>,
    #[serde(default)]
    pub emits: Vec<String>,
}

/// Parses a strict registration payload without losing duplicate method declarations.
///
/// Unknown legacy fields such as `effectResources` are ignored so plugins compiled against older
/// SDK versions still connect cleanly.
pub fn parse_registration(params: Option<&Value>) -> Result<PluginRegistration, String> {
    let methods = parse_method_list(params, "methods")?
        .ok_or_else(|| "plugin registration is missing a methods array".to_string())?;
    let emits = parse_method_list(params, "emits")?.unwrap_or_default();
    Ok(PluginRegistration { methods, emits })
}

/// Reads one optional registration array into a duplicate-free method set.
fn parse_method_list(
    params: Option<&Value>,
    field: &str,
) -> Result<Option<HashSet<String>>, String> {
    let Some(value) = params.and_then(|params| params.get(field)) else {
        return Ok(None);
    };
    let entries = value
        .as_array()
        .ok_or_else(|| format!("plugin registration field {field} must be an array"))?;
    let mut parsed = HashSet::with_capacity(entries.len());
    for entry in entries {
        let entry = entry
            .as_str()
            .filter(|entry| !entry.is_empty())
            .ok_or_else(|| {
                format!("plugin registration field {field} contains an invalid entry")
            })?;
        if !parsed.insert(entry.to_string()) {
            return Err(format!("plugin registered duplicate {field} entry {entry}"));
        }
    }
    Ok(Some(parsed))
}

/// Exports the registration DTO family into one TypeScript module.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    PluginRegistrationParams::export(config)?;
    Ok(())
}
