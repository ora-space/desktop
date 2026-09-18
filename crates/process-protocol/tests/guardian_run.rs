use ora_process_protocol::{
    DescendantPolicy, RunId, RunSpec, decode_guardian_payload, encode_guardian_frame,
};
use pretty_assertions::assert_eq;

/// Run IDs use the same canonical, non-nil representation as persisted scope identities.
#[test]
fn run_identity_rejects_aliases() -> Result<(), Box<dyn std::error::Error>> {
    let run = RunId::new();
    let frame = encode_guardian_frame(&run)?;
    assert_eq!(decode_guardian_payload::<RunId>(&frame[4..])?, run);
    for invalid in [
        "00000000-0000-0000-0000-000000000000",
        "AAAAAAAA-AAAA-4AAA-AAAA-AAAAAAAAAAAA",
        "../run",
    ] {
        let frame = encode_guardian_frame(&invalid)?;
        assert!(decode_guardian_payload::<RunId>(&frame[4..]).is_err());
    }
    Ok(())
}

/// Native path, program, arguments and environment survive exact journal/wire round trips.
#[cfg(unix)]
#[test]
fn run_spec_preserves_non_utf8_os_strings() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::ffi::OsStringExt;
    let native = std::ffi::OsString::from_vec(vec![b'x', 0xff]);
    let mut spec = RunSpec::new(
        native.clone(),
        std::path::PathBuf::from(&native),
        DescendantPolicy::WaitForAll,
    );
    spec.args.push(native.clone());
    spec.env.insert(native.clone(), native);
    let frame = encode_guardian_frame(&spec)?;
    assert_eq!(decode_guardian_payload::<RunSpec>(&frame[4..])?, spec);
    Ok(())
}
