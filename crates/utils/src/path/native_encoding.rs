use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Encodes paths as platform-tagged native OS strings instead of serde's UTF-8-only path format.
/// This preserves bytes/code units but performs no validation, normalization or authorization.
pub fn serialize_native_path<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    path.as_os_str().serialize(serializer)
}

/// Rejects foreign-platform representations rather than silently converting an executable path.
pub fn deserialize_native_path<'de, D: Deserializer<'de>>(decoder: D) -> Result<PathBuf, D::Error> {
    OsString::deserialize(decoder).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Neither Unix bytes nor Windows unpaired UTF-16 units are lost at the serialization boundary.
    #[test]
    fn native_paths_round_trip_without_unicode_normalization()
    -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(OsString::from_vec(vec![b'x', 0xff]))
        };
        #[cfg(windows)]
        let path = {
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(OsString::from_wide(&[120, 0xd800]))
        };
        let mut bytes = Vec::new();
        serialize_native_path(&path, &mut serde_json::Serializer::new(&mut bytes))?;
        assert_eq!(
            deserialize_native_path(&mut serde_json::Deserializer::from_slice(&bytes))?,
            path
        );
        #[cfg(unix)]
        let foreign = r#"{"Windows":[120]}"#;
        #[cfg(windows)]
        let foreign = r#"{"Unix":[120]}"#;
        assert!(deserialize_native_path(&mut serde_json::Deserializer::from_str(foreign)).is_err());
        Ok(())
    }
}
