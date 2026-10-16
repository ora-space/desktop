//! Resolves one complete Skill configuration into a collision-free process environment.

use crate::{ConfigurationCompleteness, ConfigurationDetails, ConfigurationSummary};
use std::collections::BTreeMap;

/// Resolves all values atomically; incomplete or OS-incompatible configurations yield no variables.
pub fn resolve_skill_environment(
    plugin_id: &str,
    details: &ConfigurationDetails,
) -> Option<BTreeMap<String, String>> {
    if details.summary
        != (ConfigurationSummary::Available {
            completeness: ConfigurationCompleteness::Complete,
        })
    {
        return None;
    }
    let mut environment = BTreeMap::new();
    for setting in &details.settings {
        if setting.value_error_code.is_some() {
            return None;
        }
        if let Some(value) = &setting.effective_value {
            let value = value.as_runtime_text();
            if value.contains('\0') {
                return None;
            }
            environment.insert(
                skill_environment_key(plugin_id, &setting.declaration.id),
                value,
            );
        }
    }
    Some(environment)
}

/// Encodes identities without case folding or lossy punctuation replacement, including on Windows.
pub fn skill_environment_key(plugin_id: &str, setting_id: &str) -> String {
    let mut key = String::from("ORA_SKILL_P");
    for byte in plugin_id.as_bytes() {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        key.push(char::from(HEX[usize::from(byte >> 4)]));
        key.push(char::from(HEX[usize::from(byte & 15)]));
    }
    key.push_str("_S");
    for byte in setting_id.as_bytes() {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        key.push(char::from(HEX[usize::from(byte >> 4)]));
        key.push(char::from(HEX[usize::from(byte & 15)]));
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConfigurationService;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Drives declaration parsing, defaults, completeness and serialization through the real store.
    #[test]
    fn resolves_complete_scalars_and_rejects_whole_incomplete_or_nul_packages() {
        let root = tempfile::tempdir().unwrap();
        let assets = root.path().join("package").join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let mut declaration = json!({"schemaVersion":1,"settings":{
            "token":{"type":"string","title":"Token","description":"Credential","secret":true,"default":"private"},
            "count":{"type":"number","title":"Count","description":"Count","default":3},
            "enabled":{"type":"boolean","title":"Enabled","description":"Enabled","default":false},
            "optional":{"type":"string","title":"Optional","description":"Optional"}
        }});
        let service = ConfigurationService::new(root.path());
        for invalid in [false, true] {
            declaration["settings"]["optional"]["required"] = json!(invalid);
            std::fs::write(assets.join("config.json"), declaration.to_string()).unwrap();
            let details = service
                .get("official/demo", assets.parent().unwrap())
                .unwrap()
                .unwrap();
            assert!(
                details
                    .settings
                    .iter()
                    .find(|s| s.declaration.id == "token")
                    .unwrap()
                    .declaration
                    .secret
            );
            assert_eq!(
                resolve_skill_environment("official/demo", &details),
                if invalid {
                    None
                } else {
                    Some(BTreeMap::from([
                        (
                            skill_environment_key("official/demo", "token"),
                            "private".into(),
                        ),
                        (skill_environment_key("official/demo", "count"), "3".into()),
                        (
                            skill_environment_key("official/demo", "enabled"),
                            "false".into(),
                        ),
                    ]))
                }
            );
        }
        declaration["settings"]["optional"]["required"] = json!(false);
        declaration["settings"]["token"]["default"] = json!("private\0value");
        std::fs::write(assets.join("config.json"), declaration.to_string()).unwrap();
        let details = service
            .get("official/demo", assets.parent().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(resolve_skill_environment("official/demo", &details), None);
    }

    /// Case and punctuation retain their identity even on case-insensitive environment implementations.
    #[test]
    fn names_are_injective_and_secret_changes_affect_fingerprints() {
        assert_eq!(
            skill_environment_key("a/b", "token"),
            "ORA_SKILL_P612F62_S746F6B656E"
        );
        let keys = [("a/b", "token"), ("a-b", "token"), ("a/b", "Token")]
            .map(|(plugin, setting)| skill_environment_key(plugin, setting));
        assert_eq!(
            keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
            3
        );
        let plain = br#"{"schemaVersion":1,"settings":{"token":{"type":"string","title":"Token","description":"Token"}}}"#;
        let mut secret: serde_json::Value = serde_json::from_slice(plain).unwrap();
        secret["settings"]["token"]["secret"] = json!(true);
        let old = crate::compile_declaration(plain).unwrap();
        let new = crate::compile_declaration(secret.to_string().as_bytes()).unwrap();
        assert!(!old.settings[0].secret);
        assert!(new.settings[0].secret);
        assert_ne!(old.fingerprint, new.fingerprint);
    }
}
