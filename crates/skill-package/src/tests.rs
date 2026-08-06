use crate::manifest::{ManifestError, parse_manifest};
use crate::scan::scan_skill_boundaries;
use crate::{
    ArchiveFormat, Limits, PrepareError, RelativePath, SnapshotWriter, copy_folder_to,
    extract_archive,
};
use pretty_assertions::assert_eq;
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

/// Writes one minimal SKILL.md manifest with the given name and description.
fn write_manifest(dir: &Path, relative: &str, name: &str, description: &str) {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("---\nname: {name}\ndescription: {description}\n---\nSome body.\n"),
    )
    .unwrap();
}

/// Writes one ordinary file under a skill folder.
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

#[test]
fn extracts_zip_archives_with_multiple_skills() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("skills.zip");
    build_zip(
        &archive,
        &[
            (
                "skills/review/SKILL.md",
                b"---\nname: review\ndescription: Reviews\n---\n",
                None,
            ),
            ("skills/review/README.md", b"review docs", None),
            (
                "skills/pr-complete-read/SKILL.md",
                b"---\nname: pr-complete-read\ndescription: Reads PRs\n---\n",
                None,
            ),
        ],
    );

    let snapshot = extract_archive(
        ArchiveFormat::Zip,
        &archive,
        &temp.path().join("out"),
        &Limits::default(),
    )
    .unwrap();

    assert_eq!(snapshot.files().len(), 3);
    let boundaries = scan_skill_boundaries(&snapshot);
    assert_eq!(boundaries.len(), 2);
    assert_eq!(
        boundaries[0].manifest_path.as_str(),
        "skills/pr-complete-read/SKILL.md"
    );
    assert_eq!(
        boundaries[1].manifest_path.as_str(),
        "skills/review/SKILL.md"
    );
    assert_eq!(boundaries[1].file_count(), 2);
}

#[test]
fn extracts_tar_gz_archives() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("skills.tar.gz");
    build_tar_gz(
        &archive,
        &[
            (
                "skills/review/SKILL.md",
                b"---\nname: review\ndescription: Reviews\n---\n",
                None,
            ),
            ("skills/review/tool.sh", b"#!/bin/sh", None),
        ],
    );

    let snapshot = extract_archive(
        ArchiveFormat::TarGz,
        &archive,
        &temp.path().join("out"),
        &Limits::default(),
    )
    .unwrap();

    let boundaries = scan_skill_boundaries(&snapshot);
    assert_eq!(boundaries.len(), 1);
    assert_eq!(boundaries[0].file_count(), 2);
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
        &Limits::default(),
    );
    assert_eq!(result.unwrap_err(), PrepareError::ArchiveFormatMismatch);

    let gzip = temp.path().join("not-gzip.tar.gz");
    fs::write(&gzip, b"plain bytes without gzip magic").unwrap();
    let result = extract_archive(
        ArchiveFormat::TarGz,
        &gzip,
        &temp.path().join("out2"),
        &Limits::default(),
    );
    assert_eq!(result.unwrap_err(), PrepareError::ArchiveFormatMismatch);
}

#[test]
fn rejects_zip_slip_and_absolute_paths() {
    let temp = TempDir::new().unwrap();

    for (entry, expected) in [
        ("../escape.txt", PrepareError::UnsafePath),
        ("/etc/passwd", PrepareError::UnsafePath),
        ("..\\windows\\escape.txt", PrepareError::UnsafePath),
        ("a/../../escape.txt", PrepareError::UnsafePath),
        ("C:/Windows/win.ini", PrepareError::UnsafePath),
        ("a//b", PrepareError::UnsafePath),
    ] {
        let archive = temp.path().join("evil.zip");
        build_zip(&archive, &[(entry, b"evil", None)]);
        let result = extract_archive(
            ArchiveFormat::Zip,
            &archive,
            &temp.path().join("out"),
            &Limits::default(),
        );
        assert_eq!(
            result.unwrap_err(),
            expected,
            "expected {entry:?} to be rejected"
        );
    }
}

#[test]
fn rejects_case_conflicting_and_duplicate_paths() {
    let temp = TempDir::new().unwrap();
    let mut writer = SnapshotWriter::new(
        temp.path().join("out"),
        Limits::default(),
        Limits::default().max_snapshot_bytes,
        false,
    )
    .unwrap();
    writer
        .add_file(
            "skills/review/SKILL.md",
            std::io::Cursor::new(b"---\nname: review\ndescription: Reviews\n---\n".to_vec()),
        )
        .unwrap();
    let case_conflict = writer.add_file(
        "skills/review/skill.md",
        std::io::Cursor::new(b"case clash".to_vec()),
    );
    assert_eq!(
        case_conflict.unwrap_err(),
        PrepareError::ArchivePathCaseConflict
    );

    let mut duplicate = SnapshotWriter::new(
        temp.path().join("out2"),
        Limits::default(),
        Limits::default().max_snapshot_bytes,
        false,
    )
    .unwrap();
    duplicate
        .add_file("a/file.txt", std::io::Cursor::new(b"one".to_vec()))
        .unwrap();
    let duplicate_error = duplicate.add_file("a/file.txt", std::io::Cursor::new(b"two".to_vec()));
    assert_eq!(
        duplicate_error.unwrap_err(),
        PrepareError::ArchivePathCaseConflict
    );
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
        &Limits::default(),
    );
    assert_eq!(
        result.unwrap_err(),
        PrepareError::ArchiveSpecialEntryUnsupported
    );

    let fifo = temp.path().join("fifo.tar.gz");
    build_tar_gz(&fifo, &[("pipe", b"", Some(tar::EntryType::Fifo))]);
    let result = extract_archive(
        ArchiveFormat::TarGz,
        &fifo,
        &temp.path().join("out2"),
        &Limits::default(),
    );
    assert_eq!(
        result.unwrap_err(),
        PrepareError::ArchiveSpecialEntryUnsupported
    );
}

#[test]
fn rejects_encrypted_zip_entries() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("encrypted.zip");
    let file = fs::File::create(&archive).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .with_aes_encryption(zip::AesMode::Aes256, "secret");
    writer
        .start_file("skills/review/SKILL.md", options)
        .unwrap();
    writer
        .write_all(b"---\nname: review\ndescription: Reviews\n---\n")
        .unwrap();
    writer.finish().unwrap();

    let result = extract_archive(
        ArchiveFormat::Zip,
        &archive,
        &temp.path().join("out"),
        &Limits::default(),
    );
    assert_eq!(
        result.unwrap_err(),
        PrepareError::ArchiveEncryptedUnsupported
    );
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
        &Limits::default(),
    );
    assert_eq!(
        result.unwrap_err(),
        PrepareError::ArchiveExpansionRatioExceeded
    );
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
        &Limits::default(),
    );
    assert_eq!(
        result.unwrap_err(),
        PrepareError::TooManyEntries {
            max_entries: Limits::default().max_entries
        }
    );
}

#[test]
fn copies_folders_without_following_links() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_manifest(&source, "skills/review/SKILL.md", "review", "Reviews");
    write_file(&source, "skills/review/helper.md", "# helper");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            "/etc/passwd",
            source.join("skills").join("review").join("link"),
        )
        .unwrap();
    }

    let snapshot = copy_folder_to(&source, &temp.path().join("out"), &Limits::default()).unwrap();
    let boundaries = scan_skill_boundaries(&snapshot);
    assert_eq!(boundaries.len(), 1);
    #[cfg(unix)]
    assert_eq!(boundaries[0].file_count(), 2);
    #[cfg(not(unix))]
    assert_eq!(boundaries[0].file_count(), 2);
}

#[test]
fn scans_nested_and_invalid_manifest_boundaries() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "parent", "Parent");
    write_file(&source, "parent-file.md", "x");
    write_manifest(&source, "child/SKILL.md", "child", "Child");
    write_file(&source, "child/child-file.md", "y");
    // An invalid nested manifest still cuts the parent's scope.
    fs::create_dir_all(source.join("broken")).unwrap();
    fs::write(
        source.join("broken").join("SKILL.md"),
        "no front matter here",
    )
    .unwrap();
    write_file(&source, "broken/broken-file.md", "z");
    // A plain file inside the root skill belongs to the root boundary.
    write_file(&source, "loose.txt", "belongs-to-root");

    let snapshot = copy_folder_to(&source, &temp.path().join("out"), &Limits::default()).unwrap();
    let boundaries = scan_skill_boundaries(&snapshot);

    assert_eq!(boundaries.len(), 3);
    let parent = boundaries
        .iter()
        .find(|b| b.manifest_path.as_str() == "SKILL.md")
        .unwrap();
    // Manifest + parent-file.md + loose.txt, but NOT child/ or broken/ subtrees.
    assert_eq!(parent.file_count(), 3);
    let child = boundaries
        .iter()
        .find(|b| b.manifest_path.as_str() == "child/SKILL.md")
        .unwrap();
    assert_eq!(child.file_count(), 2);
    let broken = boundaries
        .iter()
        .find(|b| b.manifest_path.as_str() == "broken/SKILL.md")
        .unwrap();
    assert_eq!(broken.file_count(), 2);
}

#[test]
fn parses_and_validates_manifests() {
    let limits = Limits::default();

    assert_eq!(
        parse_manifest(
            b"---\nname: review\ndescription: Reviews changes\n---\nbody",
            limits.max_manifest_bytes
        )
        .unwrap(),
        crate::manifest::Manifest {
            name: "review".to_string(),
            description: "Reviews changes".to_string(),
        }
    );
    assert_eq!(
        parse_manifest(b"plain markdown", limits.max_manifest_bytes).unwrap_err(),
        ManifestError::NameMissing
    );
    assert_eq!(
        parse_manifest(b"---\nname: review\n---", limits.max_manifest_bytes).unwrap_err(),
        ManifestError::DescriptionMissing
    );
    assert_eq!(
        parse_manifest(b"---\ndescription: Reviews\n---", limits.max_manifest_bytes).unwrap_err(),
        ManifestError::NameMissing
    );
    assert_eq!(
        parse_manifest(
            b"---\nname: bad name\ndescription: Reviews\n---",
            limits.max_manifest_bytes
        )
        .unwrap_err(),
        ManifestError::NameInvalid
    );
    let oversized = format!("---\nname: review\ndescription: {}\n---", "x".repeat(4097));
    assert_eq!(
        parse_manifest(oversized.as_bytes(), limits.max_manifest_bytes).unwrap_err(),
        ManifestError::DescriptionTooLarge
    );
    assert_eq!(
        parse_manifest(b"---\nname: [unclosed", limits.max_manifest_bytes).unwrap_err(),
        ManifestError::YamlInvalid
    );

    let too_large = parse_manifest(b"---\nname: review\ndescription: d\n---", 8).unwrap_err();
    assert_eq!(too_large, ManifestError::TooLarge { max_bytes: 8 });
}

#[test]
fn rewrites_manifest_preserving_unknown_fields_and_body() {
    let rewritten = crate::manifest::rewrite_manifest(
        b"---\nname: review\ndescription: Old description\ndepth: 3\n---\n# Body line\nmore **markdown**\n",
        "reviewer",
        "New description",
    )
    .unwrap();

    assert!(rewritten.contains("name: reviewer"));
    assert!(rewritten.contains("description: New description"));
    assert!(rewritten.contains("depth: 3"));
    assert!(rewritten.contains("# Body line"));
    assert!(rewritten.contains("more **markdown**"));
    let replaced = crate::manifest::rewrite_manifest_body(
        rewritten.as_bytes(),
        "reviewer",
        "New description",
        "# Replacement\n",
    )
    .unwrap();
    assert!(replaced.contains("depth: 3"));
    assert!(replaced.ends_with("# Replacement\n"));
    assert!(!replaced.contains("more **markdown**"));

    let empty = crate::manifest::rewrite_manifest_body(
        replaced.as_bytes(),
        "reviewer",
        "New description",
        "",
    )
    .unwrap();
    assert!(empty.ends_with("---\n"));

    // A file without front matter gets a fresh block and keeps the whole body.
    let plain =
        crate::manifest::rewrite_manifest(b"just markdown\n", "reviewer", "New description")
            .unwrap();
    assert!(plain.starts_with("---\n"));
    assert!(plain.contains("just markdown\n"));
}

#[test]
fn enforces_segment_length_limits_on_writer() {
    let temp = TempDir::new().unwrap();
    let mut writer = SnapshotWriter::new(
        temp.path().join("out"),
        Limits::default(),
        Limits::default().max_snapshot_bytes,
        false,
    )
    .unwrap();
    let long_name = "x".repeat(256);
    let result = writer.add_file(&long_name, std::io::Cursor::new(b"x".to_vec()));
    assert_eq!(result.unwrap_err(), PrepareError::PathSegmentTooLong);
}

#[test]
fn root_manifest_is_recognized() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "root-skill", "Root");
    write_file(&source, "notes.md", "notes");

    let snapshot = copy_folder_to(&source, &temp.path().join("out"), &Limits::default()).unwrap();
    let boundaries = scan_skill_boundaries(&snapshot);
    assert_eq!(boundaries.len(), 1);
    assert_eq!(boundaries[0].manifest_path.as_str(), "SKILL.md");
    assert_eq!(boundaries[0].file_count(), 2);
}

#[test]
fn reads_snapshot_files_through_validated_paths() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_manifest(&source, "skills/review/SKILL.md", "review", "Reviews");

    let snapshot = copy_folder_to(&source, &temp.path().join("out"), &Limits::default()).unwrap();
    let manifest_path = RelativePath::parse("skills/review/SKILL.md").unwrap();
    let bytes = snapshot.read_file(&manifest_path).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("name: review"));

    let missing = snapshot.read_file(&RelativePath::parse("nope/SKILL.md").unwrap());
    assert!(missing.is_err());
}
