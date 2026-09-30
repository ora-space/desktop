use ora_controller_proto::v1::{GrantRevisionUploadRequest, UploadGrant};
use pretty_assertions::assert_eq;
use prost::Message;
use std::collections::HashMap;

// The published request knew only fields 1 and 2. This decoder verifies unknown-field behavior.
#[derive(Clone, PartialEq, Message)]
struct LegacyGrantRequest {
    #[prost(int64, tag = "1")]
    epoch: i64,
    #[prost(string, tag = "2")]
    execution_id: String,
}

/// Existing v1 bytes retain identical meaning and serialization when checksums are omitted.
#[test]
fn legacy_upload_request_keeps_its_wire_shape() -> Result<(), prost::DecodeError> {
    let wire = [0x08, 7, 0x12, 3, b'e', b'x', b'e'];
    let decoded = GrantRevisionUploadRequest::decode(wire.as_slice())?;
    assert_eq!(
        decoded,
        GrantRevisionUploadRequest {
            epoch: 7,
            execution_id: "exe".into(),
            checksums: HashMap::new(),
        }
    );
    assert_eq!(decoded.encode_to_vec(), wire);
    Ok(())
}

/// New clients retain the optional digest map; old clients can still decode the same request.
#[test]
fn checksum_request_is_additive_and_round_trips() -> Result<(), prost::DecodeError> {
    let request = GrantRevisionUploadRequest {
        epoch: 7,
        execution_id: "delivery".into(),
        checksums: HashMap::from([("frozen/history".into(), "0".repeat(64))]),
    };
    let wire = request.encode_to_vec();
    assert_eq!(
        GrantRevisionUploadRequest::decode(wire.as_slice())?,
        request
    );
    assert_eq!(
        LegacyGrantRequest::decode(wire.as_slice())?,
        LegacyGrantRequest {
            epoch: 7,
            execution_id: "delivery".into(),
        }
    );
    Ok(())
}

/// Uploaders must forward the signed checksum header without a consumer-side translation.
#[test]
fn upload_grant_preserves_checksum_headers() -> Result<(), prost::DecodeError> {
    let grant = UploadGrant {
        object_key: "frozen/history".into(),
        method: "PUT".into(),
        headers: HashMap::from([
            ("x-amz-checksum-sha256".into(), "checksum-value".into()),
            ("x-amz-sdk-checksum-algorithm".into(), "SHA256".into()),
        ]),
        ..UploadGrant::default()
    };
    assert_eq!(
        UploadGrant::decode(grant.encode_to_vec().as_slice())?,
        grant
    );
    Ok(())
}
