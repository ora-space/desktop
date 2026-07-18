use crate::{ALL_AGENT_METHODS, FrameType, MAX_FRAME_BYTES, encode_frame};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

/// The checked-in canonical Frame v1 fixture schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameGoldenFixture {
    pub fixture_version: u32,
    pub wire_version: u32,
    pub maximum_payload_bytes: usize,
    pub valid: Vec<FrameGoldenVector>,
    pub invalid: Vec<InvalidFrameGoldenVector>,
}

/// One exact valid Frame vector shared by Rust and TypeScript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameGoldenVector {
    pub frame_type: String,
    pub payload_utf8: String,
    pub payload_len: usize,
    pub header_hex: String,
    pub frame_hex: String,
}

/// One invalid byte vector and its stable rejection category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidFrameGoldenVector {
    pub frame_hex: String,
    pub reason: String,
}

/// A generated projection of the closed Agent method registry and representative DTO shapes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContractGoldenFixture {
    pub fixture_version: u32,
    pub contract_version: u32,
    pub methods: Vec<AgentMethodGolden>,
    pub representative_values: Vec<NamedGoldenValue>,
}

/// Method metadata that SDK behavior interfaces must match at compile time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMethodGolden {
    pub method: String,
    pub semantics: String,
    pub streaming: bool,
    pub safety_control: bool,
}

/// A named JSON value used for cross-language encode/decode checks.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedGoldenValue {
    pub name: String,
    pub value: Value,
}

/// Builds the canonical valid and invalid Frame vectors from the production encoder.
pub fn canonical_frame_golden_fixture() -> FrameGoldenFixture {
    let payloads = [
        r#"{"jsonrpc":"2.0","id":"h:1","method":"ping","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":"h:1","result":"ok"}"#,
        r#"{"jsonrpc":"2.0","method":"$/exit"}"#,
        r#"{"jsonrpc":"2.0","method":"$/stream","params":{"id":"h:1","seq":1,"value":{"kind":"textDelta","text":"你好"}}}"#,
    ];
    let valid = payloads
        .into_iter()
        .map(|payload| {
            let encoded = encode_frame(FrameType::Json, payload.as_bytes(), MAX_FRAME_BYTES)
                .unwrap_or_else(|error| panic!("canonical frame must encode: {error}"));
            FrameGoldenVector {
                frame_type: "json".to_string(),
                payload_utf8: payload.to_string(),
                payload_len: payload.len(),
                header_hex: encode_hex(&encoded[..5]),
                frame_hex: encode_hex(&encoded),
            }
        })
        .collect();

    FrameGoldenFixture {
        fixture_version: 1,
        wire_version: 1,
        maximum_payload_bytes: MAX_FRAME_BYTES,
        valid,
        invalid: vec![
            // [type=json][length=0]
            invalid_vector([0x01, 0x00, 0x00, 0x00, 0x00], "zeroLength"),
            // [type=json][length=-1]
            invalid_vector([0x01, 0xff, 0xff, 0xff, 0xff], "negativeLength"),
            // [type=json][length=0x800001] exceeds MAX_FRAME_BYTES
            invalid_vector([0x01, 0x00, 0x80, 0x00, 0x01], "payloadTooLarge"),
            // [type=0x7f][length=2]
            invalid_vector([0x7f, 0x00, 0x00, 0x00, 0x02], "unknownType"),
        ],
    }
}

/// Builds deterministic Agent registry metadata plus representative discriminated unions.
pub fn canonical_agent_contract_golden_fixture() -> AgentContractGoldenFixture {
    let methods = ALL_AGENT_METHODS
        .into_iter()
        .map(|method| {
            let metadata = method.metadata();
            AgentMethodGolden {
                method: method.as_str().to_string(),
                semantics: match metadata.semantics {
                    crate::InvocationSemantics::Idempotent => "idempotent",
                    crate::InvocationSemantics::NonIdempotent => "nonIdempotent",
                }
                .to_string(),
                streaming: metadata.streaming,
                safety_control: metadata.safety_control,
            }
        })
        .collect();

    AgentContractGoldenFixture {
        fixture_version: 1,
        contract_version: 1,
        methods,
        representative_values: vec![
            named("agentScope.global", json!({"type": "global"})),
            named(
                "agentScope.project",
                json!({
                    "type": "project",
                    "projectHandle": "project-1",
                    "workingDirectory": "D:\\work"
                }),
            ),
            named(
                "agentEvent.textDelta",
                json!({"kind": "textDelta", "channel": "assistant", "text": "你好"}),
            ),
            named(
                "cancelConversation.accepted",
                json!({"disposition": "accepted"}),
            ),
            named(
                "businessError.authenticationRequired",
                json!({"kind": "authenticationRequired", "retryable": false}),
            ),
        ],
    }
}

/// Writes both canonical fixture files for Rust, TypeScript, and E2E consumers.
pub fn export_protocol_fixtures_to(
    output_directory: impl AsRef<Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let output_directory = output_directory.as_ref();
    std::fs::create_dir_all(output_directory)?;
    let frame = serde_json::to_vec_pretty(&canonical_frame_golden_fixture())?;
    let agent = serde_json::to_vec_pretty(&canonical_agent_contract_golden_fixture())?;
    std::fs::write(output_directory.join("frame-golden.json"), frame)?;
    std::fs::write(output_directory.join("agent-contract-golden.json"), agent)?;
    Ok(())
}

/// Converts a fixed invalid header vector into the checked-in hex representation.
fn invalid_vector<const N: usize>(bytes: [u8; N], reason: &str) -> InvalidFrameGoldenVector {
    InvalidFrameGoldenVector {
        frame_hex: encode_hex(&bytes),
        reason: reason.to_string(),
    }
}

/// Encodes bytes without separators so concatenation remains machine-checkable.
fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Builds one named representative JSON entry.
fn named(name: &str, value: Value) -> NamedGoldenValue {
    NamedGoldenValue {
        name: name.to_string(),
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::canonical_agent_contract_golden_fixture;
    use super::canonical_frame_golden_fixture;
    use pretty_assertions::assert_eq;

    /// Freezes payload lengths and exact type-first five-byte headers.
    #[test]
    fn canonical_frame_vectors_match_design() {
        let fixture = canonical_frame_golden_fixture();
        assert_eq!(
            fixture
                .valid
                .iter()
                .map(|vector| (vector.payload_len, vector.header_hex.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (56, "0100000038"),
                (42, "010000002a"),
                (35, "0100000023"),
                (112, "0100000070"),
            ]
        );
    }

    /// Keeps every closed Rust Agent method in the shared fixture.
    #[test]
    fn canonical_agent_fixture_contains_all_methods() {
        assert_eq!(canonical_agent_contract_golden_fixture().methods.len(), 8);
    }
}
