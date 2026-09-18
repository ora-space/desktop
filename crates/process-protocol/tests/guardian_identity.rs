use ora_process_protocol::{GuardianInstanceId, HostInstanceId, ScopeId};
use pretty_assertions::assert_eq;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Each role round-trips only canonical identities; arbitrary path components never enter ScopeId.
#[test]
fn identities_require_canonical_non_nil_uuid() -> TestResult {
    let scope = ScopeId::new();
    let host = HostInstanceId::new();
    let guardian = GuardianInstanceId::new();
    assert_eq!(
        (
            scope.to_string().parse::<ScopeId>()?,
            host.to_string().parse::<HostInstanceId>()?,
            guardian.to_string().parse::<GuardianInstanceId>()?
        ),
        (scope, host, guardian),
    );
    for invalid in [
        "../scope",
        "",
        "00000000-0000-0000-0000-000000000000",
        "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
        "aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa",
    ] {
        assert!(invalid.parse::<ScopeId>().is_err());
        assert!(invalid.parse::<HostInstanceId>().is_err());
        assert!(invalid.parse::<GuardianInstanceId>().is_err());
    }
    Ok(())
}
