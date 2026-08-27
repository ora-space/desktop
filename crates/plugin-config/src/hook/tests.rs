use super::{
    CompileHookConfigurationError, CompiledHookConfiguration, HookCommand, HookProtocol,
    compile_hook_configuration_from_bytes,
};
use pretty_assertions::assert_eq;

/// The canonical RTK v0.1.0 Hook Configuration.
const RTK_HOOK_CONFIG: &str = r#"{
    "schemaVersion": 1,
    "hook": {
        "protocol": "rtk-rewrite-v1",
        "executable": "assets/rtk.exe",
        "command": "rtk",
        "toolVersion": "0.45.0"
    }
}"#;

/// Compiles the canonical RTK Hook Configuration into its strongly typed descriptor.
#[test]
fn compiles_the_rtk_hook_configuration() {
    let compiled = compile_hook_configuration_from_bytes(RTK_HOOK_CONFIG.as_bytes())
        .expect("valid hook configuration");

    assert_eq!(
        compiled,
        CompiledHookConfiguration {
            schema_version: 1,
            settings: None,
            hook: super::HookDescriptor {
                protocol: HookProtocol::RtkRewriteV1,
                executable: ora_utils::path::PortableRelativePath::parse("assets/rtk.exe")
                    .expect("valid path"),
                command: HookCommand::parse("rtk").expect("valid command"),
                tool_version: semver::Version::parse("0.45.0").expect("valid version"),
            },
        }
    );
}

/// An unsupported schema version fails closed.
#[test]
fn rejects_unsupported_schema_version() {
    let source = r#"{"schemaVersion": 2, "hook": {"protocol": "rtk-rewrite-v1", "executable": "assets/rtk.exe", "command": "rtk", "toolVersion": "0.45.0"}}"#;
    assert_eq!(
        compile_hook_configuration_from_bytes(source.as_bytes()),
        Err(CompileHookConfigurationError::UnsupportedSchemaVersion(2))
    );
}

/// An unknown Hook protocol fails closed.
#[test]
fn rejects_unknown_protocol() {
    let source = r#"{"schemaVersion": 1, "hook": {"protocol": "rtk-rewrite-v2", "executable": "assets/rtk.exe", "command": "rtk", "toolVersion": "0.45.0"}}"#;
    assert_eq!(
        compile_hook_configuration_from_bytes(source.as_bytes()),
        Err(CompileHookConfigurationError::UnsupportedProtocol(
            "rtk-rewrite-v2".to_string()
        ))
    );
}

/// A command alias with a path separator fails closed so PATH resolution stays deterministic.
#[test]
fn rejects_command_with_path_separator() {
    let source = r#"{"schemaVersion": 1, "hook": {"protocol": "rtk-rewrite-v1", "executable": "assets/rtk.exe", "command": "bin/rtk", "toolVersion": "0.45.0"}}"#;
    let Err(error) = compile_hook_configuration_from_bytes(source.as_bytes()) else {
        panic!("expected command separator rejection");
    };
    assert!(matches!(
        error,
        CompileHookConfigurationError::InvalidDescriptor { ref field, .. }
            if field == "hook.command"
    ));
}

/// A non-SemVer tool version fails closed with a precise field path.
#[test]
fn rejects_non_semver_tool_version() {
    let source = r#"{"schemaVersion": 1, "hook": {"protocol": "rtk-rewrite-v1", "executable": "assets/rtk.exe", "command": "rtk", "toolVersion": "not-a-version"}}"#;
    let Err(error) = compile_hook_configuration_from_bytes(source.as_bytes()) else {
        panic!("expected toolVersion rejection");
    };
    assert!(matches!(
        error,
        CompileHookConfigurationError::InvalidDescriptor { ref field, .. }
            if field == "hook.toolVersion"
    ));
}

/// A missing required descriptor field fails closed.
#[test]
fn rejects_missing_descriptor_field() {
    let source = r#"{"schemaVersion": 1, "hook": {"protocol": "rtk-rewrite-v1", "executable": "assets/rtk.exe", "command": "rtk"}}"#;
    let Err(CompileHookConfigurationError::InvalidStructure(_)) =
        compile_hook_configuration_from_bytes(source.as_bytes())
    else {
        panic!("expected structural rejection");
    };
}

/// An unknown descriptor field fails closed.
#[test]
fn rejects_unknown_descriptor_field() {
    let source = r#"{"schemaVersion": 1, "hook": {"protocol": "rtk-rewrite-v1", "executable": "assets/rtk.exe", "command": "rtk", "toolVersion": "0.45.0", "extra": true}}"#;
    let Err(CompileHookConfigurationError::InvalidStructure(_)) =
        compile_hook_configuration_from_bytes(source.as_bytes())
    else {
        panic!("expected unknown-field rejection");
    };
}

/// A Hook Configuration may declare an optional Settings subset compiled by the shared compiler.
#[test]
fn compiles_hook_configuration_with_settings_subset() {
    let source = r#"{
        "schemaVersion": 1,
        "settings": {
            "verbose": {"type": "boolean", "title": "Verbose", "description": "Verbose logging"}
        },
        "hook": {
            "protocol": "rtk-rewrite-v1",
            "executable": "assets/rtk.exe",
            "command": "rtk",
            "toolVersion": "0.45.0"
        }
    }"#;
    let compiled = compile_hook_configuration_from_bytes(source.as_bytes())
        .expect("hook configuration with settings");
    assert!(compiled.settings.is_some());
    assert_eq!(compiled.hook.protocol, HookProtocol::RtkRewriteV1);
}

/// A reserved spec Setting type fails closed with the phase-one policy message.
#[test]
fn rejects_reserved_setting_type() {
    let source = r#"{
        "schemaVersion": 1,
        "settings": {
            "apiKey": {"type": "secret", "title": "API Key", "description": "Key"}
        },
        "hook": {
            "protocol": "rtk-rewrite-v1",
            "executable": "assets/rtk.exe",
            "command": "rtk",
            "toolVersion": "0.45.0"
        }
    }"#;
    let Err(error) = compile_hook_configuration_from_bytes(source.as_bytes()) else {
        panic!("expected reserved-setting-type rejection");
    };
    assert!(matches!(
        error,
        CompileHookConfigurationError::UnsupportedSettingType { ref setting_id, ref found }
            if setting_id == "apiKey" && found == "secret"
    ));
}

/// The hook command normalizes and rejects whitespace and control characters.
#[test]
fn hook_command_rejects_whitespace_and_separators() {
    assert!(HookCommand::parse("").is_err());
    assert!(HookCommand::parse("rt k").is_err());
    assert!(HookCommand::parse("rt\nk").is_err());
    assert!(HookCommand::parse("bin/rtk").is_err());
    assert!(HookCommand::parse("bin\\rtk").is_err());
    assert_eq!(HookCommand::parse("rtk").unwrap().as_str(), "rtk");
}
