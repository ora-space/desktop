//! Random values for one loopback sign-in: the `state` and the PKCE verifier and challenge.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// 32 random bytes as base64url: 43 characters, as RFC 7636 requires of a verifier.
///
/// A sign-in cannot proceed safely without randomness, so a failing system source panics like
/// `std`'s own randomness does rather than producing a guessable value.
pub(crate) fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("operating system randomness is available");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The S256 challenge of a verifier (RFC 7636 §4.2).
pub(crate) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::{challenge, random_token};
    use pretty_assertions::assert_eq;

    /// RFC 7636 appendix B.
    #[test]
    fn challenge_matches_the_rfc_example() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    /// Tokens have the verifier length and do not repeat.
    #[test]
    fn tokens_are_verifier_sized_and_distinct() {
        let (first, second) = (random_token(), random_token());
        assert_eq!((first.len(), first == second), (43, false));
    }
}
