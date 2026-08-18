use crate::manifest::{ManifestError, parse_manifest};
use crate::scan::scan_skill_boundaries;
use crate::{Limits, SkillSource, materialize_source};
use ora_utils::archive::ExtractedTree;
use ora_utils::path::StrictRelativePath;
use pretty_assertions::assert_eq;
use std::fs;
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

/// Materializes a local folder source with the default limits.
fn materialize_folder(source: &Path, destination: &Path) -> ExtractedTree {
    materialize_source(
        SkillSource::Folder { path: source },
        destination,
        &Limits::default(),
    )
    .unwrap()
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

    let snapshot = materialize_folder(&source, &temp.path().join("out"));
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
fn root_manifest_is_recognized() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "root-skill", "Root");
    write_file(&source, "notes.md", "notes");

    let snapshot = materialize_folder(&source, &temp.path().join("out"));
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

    let snapshot = materialize_folder(&source, &temp.path().join("out"));
    let manifest_path = StrictRelativePath::parse("skills/review/SKILL.md").unwrap();
    let bytes = snapshot.read_file(&manifest_path).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("name: review"));

    let missing = snapshot.read_file(&StrictRelativePath::parse("nope/SKILL.md").unwrap());
    assert!(missing.is_err());
}
