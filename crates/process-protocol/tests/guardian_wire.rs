use std::num::NonZeroU64;

use ora_process_protocol::{
    GUARDIAN_MAX_FRAME, GUARDIAN_WIRE_VERSION, GuardianChannel, GuardianInstanceId, GuardianReady,
    HostBinding, HostInstanceId, ScopeCreationIntent, ScopeId, decode_guardian_payload,
    encode_guardian_frame,
};
use pretty_assertions::assert_eq;
use serde::Serialize;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Management is additive: legacy readiness decoders reject it rather than interpreting a grant.
#[test]
fn management_messages_cannot_downgrade_to_legacy_readiness() -> TestResult {
    use ora_process_protocol::{
        GuardianManagementOperation, GuardianManagementRequest, GuardianReadyRequest,
        GuardianRequest,
    };
    let host = HostBinding {
        epoch: NonZeroU64::MIN,
        instance: HostInstanceId::new(),
    };
    let request = GuardianManagementRequest {
        version: GUARDIAN_WIRE_VERSION,
        intent: ScopeCreationIntent {
            scope: ScopeId::new(),
            guardian: GuardianInstanceId::new(),
            created_by: host,
        },
        channel: GuardianChannel::Control,
        operation: GuardianManagementOperation::Bind { host },
    };
    let frame = encode_guardian_frame(&request)?;
    assert!(matches!(
        decode_guardian_payload::<GuardianRequest>(&frame[4..])?,
        GuardianRequest::Management(_)
    ));
    assert!(decode_guardian_payload::<GuardianReadyRequest>(&frame[4..]).is_err());
    let legacy = GuardianReadyRequest {
        version: request.version,
        intent: request.intent,
        channel: request.channel,
        session: [3; 16],
    };
    let frame = encode_guardian_frame(&legacy)?;
    assert!(matches!(
        decode_guardian_payload::<GuardianRequest>(&frame[4..])?,
        GuardianRequest::Ready(_)
    ));
    assert!(decode_guardian_payload::<GuardianManagementRequest>(&frame[4..]).is_err());
    Ok(())
}

/// Round trips the public wire value and rejects concatenated, truncated and oversized payloads.
#[test]
fn framing_requires_one_complete_bounded_message() -> TestResult {
    let ready = GuardianReady {
        version: GUARDIAN_WIRE_VERSION,
        intent: ScopeCreationIntent {
            scope: ScopeId::new(),
            guardian: GuardianInstanceId::new(),
            created_by: HostBinding {
                epoch: NonZeroU64::MIN,
                instance: HostInstanceId::new(),
            },
        },
        channel: GuardianChannel::Control,
        session: [7; 16],
    };
    let frame = encode_guardian_frame(&ready)?;
    assert_eq!(
        u32::from_be_bytes(frame[..4].try_into()?) as usize,
        frame.len() - 4
    );
    assert_eq!(
        decode_guardian_payload::<GuardianReady>(&frame[4..])?,
        ready
    );
    let mut trailing = frame[4..].to_vec();
    trailing.push(0xc0);
    assert!(decode_guardian_payload::<GuardianReady>(&trailing).is_err());
    assert!(decode_guardian_payload::<GuardianReady>(&frame[4..frame.len() - 1]).is_err());
    assert!(decode_guardian_payload::<GuardianReady>(&vec![0; GUARDIAN_MAX_FRAME + 1]).is_err());
    assert!(encode_guardian_frame(&"x".repeat(GUARDIAN_MAX_FRAME)).is_err());
    // Nested single-element arrays exercise the decoder's recursion limit without large frames.
    let mut nested = vec![0x91; 32];
    nested.push(0xc0);
    assert!(decode_guardian_payload::<serde::de::IgnoredAny>(&nested).is_err());
    Ok(())
}

/// An expanded message cannot silently bypass the agreed protocol or canonical identity parser.
#[test]
fn unknown_fields_and_noncanonical_identities_are_rejected() -> TestResult {
    #[derive(Serialize)]
    struct ExpandedBinding {
        epoch: u64,
        instance: HostInstanceId,
        ignored_authority: u64,
    }
    let frame = encode_guardian_frame(&ExpandedBinding {
        epoch: 1,
        instance: HostInstanceId::new(),
        ignored_authority: 2,
    })?;
    assert!(decode_guardian_payload::<HostBinding>(&frame[4..]).is_err());
    for invalid in [
        "00000000-0000-0000-0000-000000000000",
        "../scope",
        "AAAAAAAA-AAAA-4AAA-AAAA-AAAAAAAAAAAA",
    ] {
        let frame = encode_guardian_frame(&invalid)?;
        assert!(decode_guardian_payload::<ScopeId>(&frame[4..]).is_err());
    }
    Ok(())
}
