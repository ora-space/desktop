/// Supported archive container formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

impl ArchiveFormat {
    /// Derives the allowed format from a file name extension, case-insensitively.
    ///
    /// `.skill` files are ZIP archives by convention.
    pub fn from_extension(file_name: &str) -> Option<ArchiveFormat> {
        let lower = file_name.to_ascii_lowercase();
        if lower.ends_with(".zip") || lower.ends_with(".skill") {
            Some(ArchiveFormat::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(ArchiveFormat::TarGz)
        } else {
            None
        }
    }

    /// Returns the allowed file extensions surfaced in unsupported-format errors.
    pub fn supported_extensions() -> &'static [&'static str] {
        &["zip", "skill", "tar.gz", "tgz"]
    }
}

#[cfg(test)]
mod tests {
    use super::ArchiveFormat;
    use pretty_assertions::assert_eq;

    #[test]
    fn derives_archive_formats_from_extensions_case_insensitively() {
        for name in ["skill.zip", "bundle.SKILL", "Skill.skill", "A.ZIP"] {
            assert_eq!(
                ArchiveFormat::from_extension(name),
                Some(ArchiveFormat::Zip)
            );
        }
        for name in ["skills.tar.gz", "bundle.TGZ", "Skills.tar.GZ"] {
            assert_eq!(
                ArchiveFormat::from_extension(name),
                Some(ArchiveFormat::TarGz)
            );
        }
        for name in ["skills.rar", "skills.gz", "skills", ".zip.bak"] {
            assert_eq!(ArchiveFormat::from_extension(name), None);
        }
    }
}
