use super::tree_writer::{ByteBudgetKind, TreeWriter};
use super::{ArchiveError, ArchiveFormat, ExtractLimits, copy_tree, extract_archive};
use crate::path::{StrictRelativePath, StrictRelativePathError};
use pretty_assertions::assert_eq;
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

/// Writes one ordinary file under a source folder.
fn write_file(dir: &Path, relative: &str, content: &str) {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Builds a `.zip` archive at `destination` from the entries provided.
fn build_zip(destination: &Path, entries: &[(&str, &[u8], Option<u32>)]) {
    let file = fs::File::create(destination).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    for (name, content, unix_mode) in entries.iter().copied() {
        let mut options = zip::write::SimpleFileOptions::default();
        if let Some(mode) = unix_mode {
            options = options.unix_permissions(mode);
        }
        if content.is_empty() && name.ends_with('/') {
            writer.add_directory(name, options).unwrap();
        } else {
            writer.start_file(name, options).unwrap();
            writer.write_all(content).unwrap();
        }
    }
    writer.finish().unwrap();
}

/// Builds a `.tar.gz` archive at `destination` from the provided entries.
fn build_tar_gz(destination: &Path, entries: &[(&str, &[u8], Option<tar::EntryType>)]) {
    let file = fs::File::create(destination).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (name, content, entry_type) in entries.iter().copied() {
        match entry_type {
            Some(tar::EntryType::Symlink) => {
                let target = content.iter().map(|&byte| byte as char).collect::<String>();
                let mut header = tar::Header::new_gnu();
                header.set_path(name).unwrap();
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name(&target).unwrap();
                header.set_size(0);
                header.set_cksum();
                builder.append(&header, std::io::empty()).unwrap();
            }
            Some(tar::EntryType::Fifo) => {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Fifo);
                header.set_size(0);
                header.set_path(name).unwrap();
                header.set_cksum();
                builder.append(&header, std::io::empty()).unwrap();
            }
            _ => {
                if content.is_empty() && name.ends_with('/') {
                    builder.append_dir(name, ".").unwrap();
                } else {
                    let mut header = tar::Header::new_gnu();
                    header.set_size(content.len() as u64);
                    header.set_mode(0o644);
                    header.set_cksum();
                    builder.append_data(&mut header, name, content).unwrap();
                }
            }
        }
    }
    let encoder = builder.into_inner().unwrap();
    encoder.finish().unwrap();
}

/// Creates a flat-budget writer for direct entry-level tests.
fn flat_writer(root: &Path) -> TreeWriter {
    TreeWriter::new(
        root.to_path_buf(),
        ExtractLimits::default(),
        ExtractLimits::default().max_total_bytes,
        ByteBudgetKind::FlatTotal,
    )
    .unwrap()
}

#[test]
fn extracts_zip_archives_and_lists_files_sorted() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("bundle.zip");
    build_zip(
        &archive,
        &[
            ("pkg/b/README.md", b"docs", None),
            ("pkg/a/tool.sh", b"#!/bin/sh", None),
            ("pkg/dir/", b"", None),
        ],
    );

    let tree = extract_archive(
        ArchiveFormat::Zip,
        &archive,
        &temp.path().join("out"),
        &ExtractLimits::default(),
    )
    .unwrap();

    assert_eq!(
        tree.files()
            .iter()
            .map(|file| (file.relative_path.as_str().to_string(), file.size))
            .collect::<Vec<_>>(),
        vec![
            ("pkg/a/tool.sh".to_string(), 9),
            ("pkg/b/README.md".to_string(), 4),
        ]
    );
    assert_eq!(
        fs::read(tree.root().join("pkg/b/README.md")).unwrap(),
        b"docs"
    );
}

#[test]
fn extracts_tar_gz_archives() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("bundle.tar.gz");
    build_tar_gz(
        &archive,
        &[
            ("pkg/README.md", b"docs", None),
            ("pkg/tool.sh", b"#!/bin/sh", None),
        ],
    );

    let tree = extract_archive(
        ArchiveFormat::TarGz,
        &archive,
        &temp.path().join("out"),
        &ExtractLimits::default(),
    )
    .unwrap();

    assert_eq!(tree.files().len(), 2);
    let path = StrictRelativePath::parse("pkg/tool.sh").unwrap();
    assert_eq!(tree.read_file(&path).unwrap(), b"#!/bin/sh");
    assert!(tree.find_file(&path).is_some());
}

#[test]
fn detects_archive_format_mismatch() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("not-a-zip.zip");
    fs::write(&archive, b"this is not a zip file").unwrap();

    let result = extract_archive(
        ArchiveFormat::Zip,
        &archive,
        &temp.path().join("out"),
        &ExtractLimits::default(),
    );
    assert_eq!(result.unwrap_err(), ArchiveError::FormatMismatch);

    let gzip = temp.path().join("not-gzip.tar.gz");
    fs::write(&gzip, b"plain bytes without gzip magic").unwrap();
    let result = extract_archive(
        ArchiveFormat::TarGz,
        &gzip,
        &temp.path().join("out2"),
        &ExtractLimits::default(),
    );
    assert_eq!(result.unwrap_err(), ArchiveError::FormatMismatch);
}

#[test]
fn rejects_oversized_archives_before_reading() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("big.zip");
    build_zip(&archive, &[("a.txt", b"payload", None)]);
    let limits = ExtractLimits {
        max_archive_bytes: 8,
        ..ExtractLimits::default()
    };

    let result = extract_archive(
        ArchiveFormat::Zip,
        &archive,
        &temp.path().join("out"),
        &limits,
    );
    assert_eq!(result.unwrap_err(), ArchiveError::TooLarge);
}

#[test]
fn rejects_zip_slip_and_absolute_paths() {
    let temp = TempDir::new().unwrap();

    for entry in [
        "../escape.txt",
        "/etc/passwd",
        "..\\windows\\escape.txt",
        "a/../../escape.txt",
        "C:/Windows/win.ini",
        "a//b",
    ] {
        let archive = temp.path().join("evil.zip");
        build_zip(&archive, &[(entry, b"evil", None)]);
        let result = extract_archive(
            ArchiveFormat::Zip,
            &archive,
            &temp.path().join("out"),
            &ExtractLimits::default(),
        );
        assert_eq!(
            result.unwrap_err(),
            ArchiveError::Path(StrictRelativePathError::Unsafe),
            "expected {entry:?} to be rejected"
        );
    }
}

#[test]
fn rejects_case_conflicting_and_duplicate_paths() {
    let temp = TempDir::new().unwrap();
    let mut writer = flat_writer(&temp.path().join("out"));
    writer
        .add_file("pkg/README.md", std::io::Cursor::new(b"docs".to_vec()))
        .unwrap();
    let case_conflict = writer.add_file(
        "pkg/readme.md",
        std::io::Cursor::new(b"case clash".to_vec()),
    );
    assert_eq!(case_conflict.unwrap_err(), ArchiveError::PathCaseConflict);

    let mut duplicate = flat_writer(&temp.path().join("out2"));
    duplicate
        .add_file("a/file.txt", std::io::Cursor::new(b"one".to_vec()))
        .unwrap();
    let duplicate_error = duplicate.add_file("a/file.txt", std::io::Cursor::new(b"two".to_vec()));
    assert_eq!(duplicate_error.unwrap_err(), ArchiveError::PathCaseConflict);
}

#[test]
fn rejects_symlink_and_special_tar_entries() {
    let temp = TempDir::new().unwrap();
    let symlink = temp.path().join("symlink.tar.gz");
    build_tar_gz(
        &symlink,
        &[("link", b"target", Some(tar::EntryType::Symlink))],
    );
    let result = extract_archive(
        ArchiveFormat::TarGz,
        &symlink,
        &temp.path().join("out"),
        &ExtractLimits::default(),
    );
    assert_eq!(result.unwrap_err(), ArchiveError::SpecialEntryUnsupported);

    let fifo = temp.path().join("fifo.tar.gz");
    build_tar_gz(&fifo, &[("pipe", b"", Some(tar::EntryType::Fifo))]);
    let result = extract_archive(
        ArchiveFormat::TarGz,
        &fifo,
        &temp.path().join("out2"),
        &ExtractLimits::default(),
    );
    assert_eq!(result.unwrap_err(), ArchiveError::SpecialEntryUnsupported);
}

#[test]
fn rejects_encrypted_zip_entries() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("encrypted.zip");
    let file = fs::File::create(&archive).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .with_aes_encryption(zip::AesMode::Aes256, "secret");
    writer.start_file("pkg/README.md", options).unwrap();
    writer.write_all(b"secret docs").unwrap();
    writer.finish().unwrap();

    let result = extract_archive(
        ArchiveFormat::Zip,
        &archive,
        &temp.path().join("out"),
        &ExtractLimits::default(),
    );
    assert_eq!(result.unwrap_err(), ArchiveError::EncryptedUnsupported);
}

#[test]
fn rejects_expansion_ratio_bombs() {
    let temp = TempDir::new().unwrap();
    // 100 MiB of zeros compressed to ~100 KiB => ratio exceeds 100:1 for a small archive.
    let archive = temp.path().join("bomb.zip");
    let file = fs::File::create(&archive).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("big.bin", options).unwrap();
    let chunk = vec![0u8; 1024 * 1024];
    for _ in 0..100 {
        writer.write_all(&chunk).unwrap();
    }
    writer.finish().unwrap();

    let result = extract_archive(
        ArchiveFormat::Zip,
        &archive,
        &temp.path().join("out"),
        &ExtractLimits::default(),
    );
    assert_eq!(result.unwrap_err(), ArchiveError::ExpansionRatioExceeded);
}

#[test]
fn rejects_entry_count_exhaustion() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("many.zip");
    let file = fs::File::create(&archive).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..5001 {
        writer.start_file(format!("f{index}.txt"), options).unwrap();
        writer.write_all(b"x").unwrap();
    }
    writer.finish().unwrap();

    let result = extract_archive(
        ArchiveFormat::Zip,
        &archive,
        &temp.path().join("out"),
        &ExtractLimits::default(),
    );
    assert_eq!(
        result.unwrap_err(),
        ArchiveError::TooManyEntries {
            max_entries: ExtractLimits::default().max_entries
        }
    );
}

#[test]
fn copies_folders_without_following_links() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_file(&source, "pkg/README.md", "# docs");
    write_file(&source, "pkg/helper.md", "# helper");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", source.join("pkg").join("link")).unwrap();
    }

    let tree = copy_tree(&source, &temp.path().join("out"), &ExtractLimits::default()).unwrap();
    assert_eq!(
        tree.files()
            .iter()
            .map(|file| file.relative_path.as_str().to_string())
            .collect::<Vec<_>>(),
        vec!["pkg/README.md".to_string(), "pkg/helper.md".to_string()]
    );
    assert!(!tree.root().join("pkg").join("link").exists());
}

#[test]
fn copies_folders_within_flat_byte_budget() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_file(&source, "a.txt", "0123456789");
    let limits = ExtractLimits {
        max_total_bytes: 4,
        ..ExtractLimits::default()
    };

    let result = copy_tree(&source, &temp.path().join("out"), &limits);
    assert_eq!(result.unwrap_err(), ArchiveError::TotalBytesExceeded);
}

#[test]
fn enforces_segment_length_limits_on_writer() {
    let temp = TempDir::new().unwrap();
    let mut writer = flat_writer(&temp.path().join("out"));
    let long_name = "x".repeat(256);
    let result = writer.add_file(&long_name, std::io::Cursor::new(b"x".to_vec()));
    assert_eq!(
        result.unwrap_err(),
        ArchiveError::Path(StrictRelativePathError::SegmentTooLong)
    );
}
