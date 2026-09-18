use super::{
    CompileHookConfigurationError, CompiledHookConfiguration, HookDescriptor, HookLifecycle,
    HookLifecycleCommand, compile_hook_configuration_from_bytes,
};
use ora_utils::Slug;
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;

/// The canonical RTK Hook Configuration.
const RTK_HOOK_CONFIG: &str = r#"{
    "schemaVersion": 1,
    "hook": {
        "executable": "assets/rtk.exe",
        "supportedAgents": ["claude-code", "codex"],
        "lifecycle": {
            "init": {"args": ["--init"]},
            "deinit": {"args": ["--deinit"]}
        }
    }
}"#;

/// Builds one lifecycle command from its declared arguments.
fn lifecycle_command(args: &[&str]) -> HookLifecycleCommand {
    HookLifecycleCommand::parse(
        "hook.lifecycle",
        args.iter().map(|arg| (*arg).to_string()).collect(),
    )
    .expect("valid lifecycle arguments")
}

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
            hook: HookDescriptor {
                executable: PortableRelativePath::parse("assets/rtk.exe")
                    .expect("valid executable path"),
                supported_agents: vec![
                    Slug::parse("claude-code").expect("valid agent"),
                    Slug::parse("codex").expect("valid agent"),
                ],
                lifecycle: HookLifecycle {
                    init: lifecycle_command(&["--init"]),
                    deinit: Some(lifecycle_command(&["--deinit"])),
                },
            },
        }
    );
}

/// Only `executable` and `lifecycle.init` are required: an unsupported-Agent-free Hook may
/// declare no `supportedAgents`, no `deinit`, and no arguments at all.
#[test]
fn compiles_hook_configuration_without_optional_members() {
    let source = r#"{
        "schemaVersion": 1,
        "hook": {
            "executable": "assets/rtk.exe",
            "lifecycle": {"init": {}}
        }
    }"#;
    let compiled = compile_hook_configuration_from_bytes(source.as_bytes())
        .expect("minimal hook configuration");

    assert_eq!(
        compiled.hook,
        HookDescriptor {
            executable: PortableRelativePath::parse("assets/rtk.exe")
                .expect("valid executable path"),
            supported_agents: Vec::new(),
            lifecycle: HookLifecycle {
                init: lifecycle_command(&[]),
                deinit: None,
            },
        }
    );
}

/// An unsupported schema version fails closed.
#[test]
fn rejects_unsupported_schema_version() {
    let source = r#"{"schemaVersion": 2, "hook": {"executable": "assets/rtk.exe", "lifecycle": {"init": {}}}}"#;
    assert_eq!(
        compile_hook_configuration_from_bytes(source.as_bytes()),
        Err(CompileHookConfigurationError::UnsupportedSchemaVersion(2))
    );
}

/// Members of the replaced declaration shape are reported by name so the author learns to
/// repackage instead of reading serde's unknown-field list.
#[test]
fn rejects_descriptor_fields_removed_by_the_pack_decision() {
    let cases = [
        ("protocol", r#""rtk-rewrite-v1""#),
        ("command", r#""rtk""#),
        ("toolVersion", r#""0.45.0""#),
    ];

    for (removed, value) in cases {
        let source = format!(
            r#"{{"schemaVersion":1,"hook":{{"executable":"assets/rtk.exe","lifecycle":{{"init":{{}}}},"{removed}":{value}}}}}"#
        );
        assert_eq!(
            compile_hook_configuration_from_bytes(source.as_bytes()),
            Err(CompileHookConfigurationError::RemovedDescriptorField {
                field: format!("hook.{removed}"),
            }),
            "{removed}"
        );
    }
}

/// An unknown descriptor member that was never part of any Hook shape stays a structural error.
#[test]
fn rejects_unknown_descriptor_field() {
    let source = r#"{"schemaVersion": 1, "hook": {"executable": "assets/rtk.exe", "lifecycle": {"init": {}}, "entrypoint": "assets/main.js"}}"#;
    let Err(CompileHookConfigurationError::InvalidStructure(message)) =
        compile_hook_configuration_from_bytes(source.as_bytes())
    else {
        panic!("expected unknown-field rejection");
    };
    assert!(message.contains("entrypoint"), "{message}");
}

/// A Hook without an initialization command cannot be installed.
#[test]
fn rejects_missing_init_phase() {
    let source = r#"{"schemaVersion": 1, "hook": {"executable": "assets/rtk.exe", "lifecycle": {"deinit": {}}}}"#;
    let Err(CompileHookConfigurationError::InvalidStructure(message)) =
        compile_hook_configuration_from_bytes(source.as_bytes())
    else {
        panic!("expected missing-init rejection");
    };
    assert!(message.contains("init"), "{message}");
}

/// Advertised Agent identifiers must be lowercase slugs without duplicates, so the displayed list
/// cannot name the same Agent twice or carry a spelling no Agent could match.
#[test]
fn rejects_invalid_supported_agents() {
    let too_many = (0..17)
        .map(|index| format!(r#""agent{index}""#))
        .collect::<Vec<_>>()
        .join(",");
    let cases = [
        (r#""Claude Code""#.to_string(), "hook.supportedAgents[0]"),
        (
            r#""claude-code","claude-code""#.to_string(),
            "hook.supportedAgents[1]",
        ),
        (too_many, "hook.supportedAgents"),
    ];

    for (agents, expected_field) in cases {
        let source = format!(
            r#"{{"schemaVersion":1,"hook":{{"executable":"assets/rtk.exe","supportedAgents":[{agents}],"lifecycle":{{"init":{{}}}}}}}}"#
        );
        assert!(
            matches!(
                compile_hook_configuration_from_bytes(source.as_bytes()),
                Err(CompileHookConfigurationError::InvalidDescriptor { field, .. })
                    if field == expected_field
            ),
            "{agents}"
        );
    }
}

/// Lifecycle arguments reach the operating system verbatim, so an empty or control-bearing value
/// is rejected at compile time instead of corrupting a spawn or a diagnostic record.
#[test]
fn rejects_invalid_lifecycle_arguments() {
    let cases = [
        (r#"[""]"#, "argument must not be empty"),
        (
            r#"["--init\r\nrm -rf /"]"#,
            "argument must not contain control characters",
        ),
    ];

    for (args, expected_reason) in cases {
        let source = format!(
            r#"{{"schemaVersion":1,"hook":{{"executable":"assets/rtk.exe","lifecycle":{{"init":{{"args":{args}}}}}}}}}"#
        );
        assert_eq!(
            compile_hook_configuration_from_bytes(source.as_bytes()),
            Err(CompileHookConfigurationError::InvalidDescriptor {
                field: "hook.lifecycle.init.args[0]".to_string(),
                reason: expected_reason.to_string(),
            }),
            "{args}"
        );
    }
}

/// Both phases are validated independently, and an invalid `deinit` never masks a valid `init`.
#[test]
fn rejects_invalid_deinit_arguments_independently_of_init() {
    let source = r#"{
        "schemaVersion": 1,
        "hook": {
            "executable": "assets/rtk.exe",
            "lifecycle": {
                "init": {"args": ["--init"]},
                "deinit": {"args": ["--deinit", ""]}
            }
        }
    }"#;
    assert_eq!(
        compile_hook_configuration_from_bytes(source.as_bytes()),
        Err(CompileHookConfigurationError::InvalidDescriptor {
            field: "hook.lifecycle.deinit.args[1]".to_string(),
            reason: "argument must not be empty".to_string(),
        })
    );
}

/// Arguments beyond the supported count or byte bound are rejected before execution.
#[test]
fn rejects_oversized_and_excessive_lifecycle_arguments() {
    let oversized = format!(r#"["{}"]"#, "a".repeat(513));
    let over_count = format!(
        "[{}]",
        (0..17)
            .map(|index| format!(r#""--arg{index}""#))
            .collect::<Vec<_>>()
            .join(",")
    );
    let cases = [oversized, over_count];

    for args in cases {
        let source = format!(
            r#"{{"schemaVersion":1,"hook":{{"executable":"assets/rtk.exe","lifecycle":{{"init":{{"args":{args}}}}}}}}}"#
        );
        assert!(
            matches!(
                compile_hook_configuration_from_bytes(source.as_bytes()),
                Err(CompileHookConfigurationError::InvalidDescriptor { field, .. })
                    if field.starts_with("hook.lifecycle.init.args")
            ),
            "{args}"
        );
    }
}

/// The executable must stay a safe package-relative path.
#[test]
fn rejects_executable_path_violations() {
    let cases = [
        "../rtk.exe",
        "/usr/bin/rtk",
        "assets/../../rtk.exe",
        "C:/rtk.exe",
    ];

    for executable in cases {
        let source = format!(
            r#"{{"schemaVersion":1,"hook":{{"executable":"{executable}","lifecycle":{{"init":{{}}}}}}}}"#
        );
        assert_eq!(
            compile_hook_configuration_from_bytes(source.as_bytes()),
            Err(CompileHookConfigurationError::InvalidDescriptor {
                field: "hook.executable".to_string(),
                reason: format!(
                    "executable must be a safe relative path: {}",
                    PortableRelativePath::parse(executable).expect_err("rejected path")
                ),
            }),
            "{executable}"
        );
    }
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
            "executable": "assets/rtk.exe",
            "lifecycle": {"init": {"args": ["--init"]}}
        }
    }"#;
    let compiled = compile_hook_configuration_from_bytes(source.as_bytes())
        .expect("hook configuration with settings");

    assert_eq!(compiled.settings.is_some(), true);
    assert_eq!(compiled.hook.lifecycle.init, lifecycle_command(&["--init"]));
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
            "executable": "assets/rtk.exe",
            "lifecycle": {"init": {}}
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
