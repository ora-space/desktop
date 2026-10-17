use super::{CompileAgentConfigurationError, compile_agent_configuration};
use crate::{MAX_DECLARATION_BYTES, MAX_SETTINGS, compile_configuration_file, compile_declaration};
use pretty_assertions::assert_eq;
use serde::Deserialize;
use serde_json::{Value, json};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

const MINIMAL: &[u8] = br#"{"schemaVersion":2,"agent":{"modelAccess":{"contractVersion":1,"protocols":["anthropic-messages"],"modelCardinality":"single"}}}"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractCase {
    name: String,
    source: String,
    expected: Value,
}

/// The fixture is portable input/output data for other implementations, not Rust-specific assertions.
#[test]
fn matches_shared_contract_vectors() -> TestResult {
    let cases: Vec<ContractCase> = serde_json::from_str(include_str!(
        "../../tests/fixtures/agent-configuration-contract.json"
    ))?;
    assert!(!cases.is_empty());
    for case in cases {
        let actual = match compile_agent_configuration(case.source.as_bytes()) {
            Ok(compiled) => json!({"ok": compiled}),
            Err(error) => json!({"error": error_code(&error)}),
        };
        assert_eq!(actual, case.expected, "{}", case.name);
    }
    Ok(())
}

/// The fixture vocabulary describes compiler outcomes without freezing Rust error display wording.
fn error_code(error: &CompileAgentConfigurationError) -> &'static str {
    match error {
        CompileAgentConfigurationError::TooLarge => "too_large",
        CompileAgentConfigurationError::DuplicateKey => "duplicate_key",
        CompileAgentConfigurationError::InvalidJson => "invalid_json",
        CompileAgentConfigurationError::InvalidStructure => "invalid_structure",
        CompileAgentConfigurationError::UnsupportedSchemaVersion(_) => "unsupported_schema_version",
        CompileAgentConfigurationError::UnsupportedContractVersion(_) => {
            "unsupported_contract_version"
        }
        CompileAgentConfigurationError::EmptyProtocols => "empty_protocols",
        CompileAgentConfigurationError::DuplicateProtocol => "duplicate_protocol",
        CompileAgentConfigurationError::UnsupportedProtocol => "unsupported_protocol",
        CompileAgentConfigurationError::InvalidSettings => "invalid_settings",
    }
}

/// Reformatting changes the release byte identity without changing the compiled capability values.
#[test]
fn fingerprints_exact_bytes_and_preserves_legacy_fingerprints() -> TestResult {
    let value: Value = serde_json::from_slice(MINIMAL)?;
    let pretty = serde_json::to_vec_pretty(&value)?;
    let compact = compile_agent_configuration(MINIMAL)?;
    let mut formatted = compile_agent_configuration(&pretty)?;
    assert_ne!(compact.fingerprint, formatted.fingerprint);
    formatted.fingerprint.clone_from(&compact.fingerprint);
    assert_eq!(formatted, compact);

    let legacy = json!({"schemaVersion": 1, "settings": {
        "enabled": {"type": "boolean", "title": "Enabled", "description": "Enable feature"}
    }});
    assert_eq!(
        compile_declaration(&serde_json::to_vec(&legacy)?),
        compile_declaration(&serde_json::to_vec_pretty(&legacy)?),
    );
    Ok(())
}

/// Declaration recognition must not enable installation or runtime use before those consumers exist.
#[test]
fn legacy_dispatcher_still_rejects_the_new_shape() {
    assert!(compile_agent_configuration(MINIMAL).is_ok());
    assert!(compile_configuration_file(MINIMAL).is_err());
    for source in [
        br#"{"schemaVersion":1,"settings":{"enabled":{"type":"boolean","title":"Enabled","description":"Enable feature"}}}"#.as_slice(),
        br#"{"schemaVersion":1,"transport":{"type":"http","url":"https://mcp.example.invalid/server"}}"#.as_slice(),
    ] {
        assert!(compile_configuration_file(source).is_ok());
        assert!(compile_agent_configuration(source).is_err());
    }
}

/// Input size is measured on raw bytes, including formatting that would disappear during parsing.
#[test]
fn enforces_the_source_byte_limit_before_parsing() {
    let mut boundary = MINIMAL.to_vec();
    boundary.resize(MAX_DECLARATION_BYTES, b' ');
    assert!(compile_agent_configuration(&boundary).is_ok());
    boundary.push(b' ');
    assert_eq!(
        compile_agent_configuration(&boundary),
        Err(CompileAgentConfigurationError::TooLarge)
    );
    assert_eq!(
        compile_agent_configuration(&vec![b'!'; MAX_DECLARATION_BYTES + 1]),
        Err(CompileAgentConfigurationError::TooLarge)
    );
}

/// The existing Settings owner, rather than a second parser, enforces the shared maximum count.
#[test]
fn inherits_the_settings_count_limit() -> TestResult {
    let mut declaration: Value = serde_json::from_slice(MINIMAL)?;
    let mut settings = serde_json::Map::new();
    for index in 0..MAX_SETTINGS {
        settings.insert(
            format!("option{index}"),
            json!({"type": "boolean", "title": "Option", "description": "Enable option"}),
        );
    }
    declaration["settings"] = Value::Object(settings);
    assert!(compile_agent_configuration(&serde_json::to_vec(&declaration)?).is_ok());
    declaration["settings"]["extra"] =
        json!({"type": "boolean", "title": "Extra", "description": "Extra option"});
    assert_eq!(
        compile_agent_configuration(&serde_json::to_vec(&declaration)?),
        Err(CompileAgentConfigurationError::InvalidSettings)
    );
    Ok(())
}

/// Nested duplicate keys must not be collapsed before the model-access or Setting validators run.
#[test]
fn rejects_duplicate_keys_at_each_declaration_level() {
    for source in [
        r#"{"schemaVersion":2,"schemaVersion":1}"#,
        r#"{"schemaVersion":2,"agent":{"modelAccess":{},"modelAccess":{}}}"#,
        r#"{"schemaVersion":2,"agent":{"modelAccess":{"contractVersion":1,"contractVersion":2}}}"#,
        r#"{"schemaVersion":2,"settings":{"enabled":{},"enabled":{}}}"#,
        r#"{"schemaVersion":2,"settings":{"enabled":{"default":true,"default":false}}}"#,
    ] {
        assert_eq!(
            compile_agent_configuration(source.as_bytes()),
            Err(CompileAgentConfigurationError::DuplicateKey)
        );
    }
}

/// Malformed encodings and concatenated documents cannot acquire a valid release fingerprint.
#[test]
fn rejects_invalid_utf8_and_trailing_json() {
    assert_eq!(
        compile_agent_configuration(&[0xff, 0xfe]),
        Err(CompileAgentConfigurationError::InvalidJson)
    );
    let mut source = MINIMAL.to_vec();
    source.extend_from_slice(b" {}");
    assert_eq!(
        compile_agent_configuration(&source),
        Err(CompileAgentConfigurationError::InvalidJson)
    );
}

/// Publisher errors must be safe even when a malformed static file contains an accidental secret.
#[test]
fn diagnostics_do_not_echo_rejected_values() -> TestResult {
    let private_input = "private-input-must-not-appear";
    for pointer in [
        "/agent/modelAccess/protocols/0",
        "/agent/modelAccess/modelCardinality",
        "/agent/modelAccess/contractVersion",
    ] {
        let mut declaration: Value = serde_json::from_slice(MINIMAL)?;
        *declaration
            .pointer_mut(pointer)
            .ok_or("missing test field")? = json!(private_input);
        let Err(error) = compile_agent_configuration(&serde_json::to_vec(&declaration)?) else {
            return Err("unsupported values were accepted".into());
        };
        assert!(!error.to_string().contains(private_input));
    }
    Ok(())
}
