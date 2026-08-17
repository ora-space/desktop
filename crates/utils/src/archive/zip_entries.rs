use super::error::ArchiveError;
use super::tree_writer::TreeWriter;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Extracts a ZIP archive, rejecting encrypted and special entries before writing.
pub(super) fn extract_zip(mut file: File, writer: &mut TreeWriter) -> Result<(), ArchiveError> {
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|_| ArchiveError::Corrupt)?;
    if magic != *b"PK\x03\x04" {
        return Err(ArchiveError::FormatMismatch);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| ArchiveError::Corrupt)?;

    let mut archive = zip::ZipArchive::new(file).map_err(|_| ArchiveError::Corrupt)?;
    for index in 0..archive.len() {
        // An AES entry without a password surfaces as an unsupported-archive or invalid-
        // password error; ZipCrypto entries are detected below through the flag bit.
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(error) if requires_password(&error) => {
                return Err(ArchiveError::EncryptedUnsupported);
            }
            Err(_) => return Err(ArchiveError::Corrupt),
        };
        if entry.encrypted() {
            return Err(ArchiveError::EncryptedUnsupported);
        }
        if is_special_zip_entry(&entry) {
            return Err(ArchiveError::SpecialEntryUnsupported);
        }
        let name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| ArchiveError::PathEncodingInvalid)?
            .to_string();
        if entry.is_dir() {
            writer.add_directory(&name)?;
        } else {
            writer.add_file(&name, &mut entry)?;
        }
    }
    Ok(())
}

/// Rejects ZIP entries that are not plain files or directories (symlinks, devices, FIFOs).
fn is_special_zip_entry(entry: &zip::read::ZipFile<'_>) -> bool {
    if entry.is_symlink() {
        return true;
    }
    if let Some(mode) = entry.unix_mode() {
        let file_type = mode & 0o170_000;
        if matches!(
            file_type,
            0o120_000 | 0o020_000 | 0o060_000 | 0o010_000 | 0o140_000
        ) {
            return true;
        }
    }
    false
}

/// Classifies zip entry-open failures that mean the entry requires a password.
fn requires_password(error: &zip::result::ZipError) -> bool {
    match error {
        zip::result::ZipError::InvalidPassword => true,
        zip::result::ZipError::UnsupportedArchive(message) => {
            message.contains("Password required to decrypt")
        }
        _ => false,
    }
}
