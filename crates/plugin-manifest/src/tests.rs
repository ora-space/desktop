use crate::{
    HomepageUrl, InvalidFieldReason, ManifestError, ManifestField, PluginDependencies, PluginHead,
    PluginKind, PluginManifest, PluginName, PluginNamespace, PluginUi, ReleaseUrl, RepositoryUrl,
    Sha256Digest, SurfaceDeclaration, SurfaceField, SurfaceInstances, SurfaceSource, WebDataMode,
};
use ora_utils::{GitBranchName, GitBranchNameError, Slug, SlugError};
use pretty_assertions::assert_eq;
use semver::{Version, VersionReq};

const DIGEST: &str = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a";

const MINIMAL_MANIFEST: &str = r#"resolver = 1
name = "user.ora-weather"
namespace = "official"
kind = "workbench"
version = "1.2.0"
description = "获取实时天气信息的 Ora 插件"
url = "https://example.com/ora-weather.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"
"#;

const FULL_MANIFEST: &str = r#"resolver = 1
name = "user.ora-weather"
namespace = "official"
kind = "workbench"
version = "1.2.0"
description = "获取实时天气信息的 Ora 插件"
homepage = "https://github.com/user/ora-weather"
license = "MIT"
url = "https://github.com/user/ora-weather/releases/download/v1.2.0/ora-weather.orax?signature=abc"
sha256 = "FEAB001D7E9FF4CE66011EBD70791DE93EB1554D34D3EA44C33D102A25C1BE0A"

[head]
repository = "https://github.com/user/ora-weather.git"
branch = "main"

[dependencies]
ora = ">= 0.8.0"
"#;

/// Extracts an expected successful result without using `unwrap` in tests.
fn success<T, E>(result: Result<T, E>, label: &str) -> T {
    match result {
        Ok(value) => value,
        Err(_) => panic!("expected {label} to succeed"),
    }
}

/// Verifies the complete example maps to the intended immutable domain object.
#[test]
fn parses_complete_manifest_into_full_domain_object() {
    let actual = success(PluginManifest::parse(FULL_MANIFEST), "complete manifest");
    let expected = PluginManifest {
        resolver: 1,
        name: success(PluginName::parse("user.ora-weather"), "plugin name"),
        namespace: PluginNamespace::Official,
        kind: PluginKind::Workbench,
        version: success(Version::parse("1.2.0"), "version"),
        description: "获取实时天气信息的 Ora 插件".to_owned(),
        homepage: Some(success(
            HomepageUrl::parse("https://github.com/user/ora-weather"),
            "homepage",
        )),
        license: Some("MIT".to_owned()),
        url: Some(success(
            ReleaseUrl::parse(
                "https://github.com/user/ora-weather/releases/download/v1.2.0/ora-weather.orax?signature=abc",
            ),
            "release URL",
        )),
        sha256: Some(success(Sha256Digest::parse(DIGEST), "digest")),
        head: Some(PluginHead {
            repository: success(
                RepositoryUrl::parse("https://github.com/user/ora-weather.git"),
                "repository URL",
            ),
            branch: success(GitBranchName::parse("main"), "branch"),
        }),
        dependencies: Some(PluginDependencies {
            ora: success(VersionReq::parse(">= 0.8.0"), "Ora requirement"),
        }),
        ui: None,
    };

    assert_eq!(actual, expected);
}

/// Verifies every optional field can be absent without manufacturing defaults.
#[test]
fn parses_manifest_without_optional_fields() {
    let manifest = success(PluginManifest::parse(MINIMAL_MANIFEST), "minimal manifest");

    assert_eq!(manifest.resolver(), 1);
    assert_eq!(manifest.name().as_str(), "user.ora-weather");
    assert_eq!(manifest.namespace(), PluginNamespace::Official);
    assert_eq!(manifest.kind(), PluginKind::Workbench);
    assert_eq!(
        manifest.version(),
        &success(Version::parse("1.2.0"), "version")
    );
    assert_eq!(manifest.description(), "获取实时天气信息的 Ora 插件");
    assert_eq!(manifest.homepage(), None);
    assert_eq!(manifest.license(), None);
    assert_eq!(manifest.head(), None);
    assert_eq!(manifest.dependencies(), None);
}

/// Verifies an explicitly empty dependency table normalizes to an undeclared dependency.
#[test]
fn normalizes_empty_dependencies_table() {
    let source = format!("{MINIMAL_MANIFEST}\n[dependencies]\n");
    let manifest = success(PluginManifest::parse(&source), "empty dependencies");

    assert_eq!(manifest.dependencies(), None);
}

/// Verifies the agent kind is accepted and round-trips to its manifest spelling.
#[test]
fn parses_agent_kind_manifest() {
    let manifest = success(
        PluginManifest::parse(&MINIMAL_MANIFEST.replacen("workbench", "agent", 1)),
        "agent-kind manifest",
    );
    assert_eq!(manifest.kind(), PluginKind::Agent);
    assert_eq!(manifest.kind().as_str(), "agent");
}

/// Verifies an installed package manifest omits download-only fields and still parses.
#[test]
fn parses_installed_manifest_without_download_fields() {
    let installed = "name = \"user.ora-weather\"\nnamespace = \"official\"\nkind = \"workbench\"\nversion = \"1.2.0\"\ndescription = \"A test plugin\"\n";
    let manifest = success(
        PluginManifest::parse_installed(installed),
        "installed manifest",
    );

    assert_eq!(manifest.resolver(), 1);
    assert_eq!(manifest.name().as_str(), "user.ora-weather");
    assert_eq!(manifest.kind(), PluginKind::Workbench);
    assert_eq!(manifest.url(), None);
    assert_eq!(manifest.sha256(), None);
    assert_eq!(manifest.release(), None);
}

/// Verifies unsupported resolver versions take priority over semantic field validation.
#[test]
fn rejects_unsupported_resolver_before_fields() {
    let source = MINIMAL_MANIFEST
        .replacen("resolver = 1", "resolver = 2", 1)
        .replacen("name = \"user.ora-weather\"", "name = \"INVALID\"", 1);

    assert!(matches!(
        PluginManifest::parse(&source),
        Err(ManifestError::UnsupportedResolver { found: 2 })
    ));
}

/// Verifies missing, mistyped, and unknown fields stay structural TOML errors with spans.
#[test]
fn reports_structural_toml_errors_with_spans() {
    let cases = [
        MINIMAL_MANIFEST.replacen("resolver = 1\n", "", 1),
        MINIMAL_MANIFEST.replacen("resolver = 1", "resolver = \"one\"", 1),
        format!("{MINIMAL_MANIFEST}unknown = true\n"),
    ];

    for source in cases {
        assert!(matches!(
            PluginManifest::parse(&source),
            Err(ManifestError::InvalidToml { span: Some(_), .. })
        ));
    }
}

/// Verifies every required root field is rejected when missing or assigned the wrong TOML type.
#[test]
fn rejects_missing_and_mistyped_required_fields() {
    let fields = [
        ("resolver = 1\n", "resolver = \"one\"\n"),
        ("name = \"user.ora-weather\"\n", "name = true\n"),
        ("namespace = \"official\"\n", "namespace = true\n"),
        ("kind = \"workbench\"\n", "kind = true\n"),
        ("version = \"1.2.0\"\n", "version = true\n"),
        (
            "description = \"获取实时天气信息的 Ora 插件\"\n",
            "description = true\n",
        ),
        (
            "url = \"https://example.com/ora-weather.orax\"\n",
            "url = true\n",
        ),
        (
            "sha256 = \"feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a\"\n",
            "sha256 = true\n",
        ),
    ];

    for (valid_line, mistyped_line) in fields {
        let missing = MINIMAL_MANIFEST.replacen(valid_line, "", 1);
        let mistyped = MINIMAL_MANIFEST.replacen(valid_line, mistyped_line, 1);
        for source in [missing, mistyped] {
            assert!(matches!(
                PluginManifest::parse(&source),
                Err(ManifestError::InvalidToml { .. })
            ));
        }
    }
}

/// Verifies empty required strings are attributed to their declared structured field.
#[test]
fn rejects_empty_required_strings() {
    let fields = [
        (
            "name = \"user.ora-weather\"",
            "name = \"\"",
            ManifestField::Name,
        ),
        (
            "namespace = \"official\"",
            "namespace = \"\"",
            ManifestField::Namespace,
        ),
        ("kind = \"workbench\"", "kind = \"\"", ManifestField::Kind),
        (
            "version = \"1.2.0\"",
            "version = \"\"",
            ManifestField::Version,
        ),
        (
            "description = \"获取实时天气信息的 Ora 插件\"",
            "description = \"\"",
            ManifestField::Description,
        ),
        (
            "url = \"https://example.com/ora-weather.orax\"",
            "url = \"\"",
            ManifestField::Url,
        ),
        (
            "sha256 = \"feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a\"",
            "sha256 = \"\"",
            ManifestField::Sha256,
        ),
    ];

    for (valid, empty, expected_field) in fields {
        let source = MINIMAL_MANIFEST.replacen(valid, empty, 1);
        let Err(ManifestError::InvalidField { field, .. }) = PluginManifest::parse(&source) else {
            panic!("expected empty value to produce a semantic field error");
        };
        assert_eq!(field, expected_field);
    }
}

/// Verifies root semantic fields are validated in schema declaration order.
#[test]
fn returns_first_root_field_error_deterministically() {
    let source = MINIMAL_MANIFEST
        .replacen("name = \"user.ora-weather\"", "name = \"INVALID\"", 1)
        .replacen("namespace = \"official\"", "namespace = \"community\"", 1);

    assert!(matches!(
        PluginManifest::parse(&source),
        Err(ManifestError::InvalidField {
            field: ManifestField::Name,
            reason: InvalidFieldReason::InvalidPluginName(_),
        })
    ));
}

/// Verifies descriptions reject rather than trim leading, trailing, and all-whitespace values.
#[test]
fn rejects_description_outer_whitespace() {
    for description in [" weather", "weather ", "   "] {
        let source = MINIMAL_MANIFEST.replacen(
            "description = \"获取实时天气信息的 Ora 插件\"",
            &format!("description = {description:?}"),
            1,
        );

        assert!(matches!(
            PluginManifest::parse(&source),
            Err(ManifestError::InvalidField {
                field: ManifestField::Description,
                reason: InvalidFieldReason::LeadingOrTrailingWhitespace,
            })
        ));
    }
}

/// Verifies Unicode descriptions and ordinary internal spaces remain valid.
#[test]
fn accepts_unicode_description_with_internal_spaces() {
    let source = MINIMAL_MANIFEST.replacen(
        "description = \"获取实时天气信息的 Ora 插件\"",
        "description = \"实时 天气插件\"",
        1,
    );
    let manifest = success(PluginManifest::parse(&source), "Unicode description");

    assert_eq!(manifest.description(), "实时 天气插件");
}

/// Verifies description and license byte limits accept their boundary and reject one byte above it.
#[test]
fn enforces_text_byte_limits() {
    let description_boundary = "a".repeat(1000);
    let description_over_limit = format!("{description_boundary}a");
    let valid_description = MINIMAL_MANIFEST.replacen(
        "description = \"获取实时天气信息的 Ora 插件\"",
        &format!("description = {description_boundary:?}"),
        1,
    );
    assert!(PluginManifest::parse(&valid_description).is_ok());

    let invalid_description = MINIMAL_MANIFEST.replacen(
        "description = \"获取实时天气信息的 Ora 插件\"",
        &format!("description = {description_over_limit:?}"),
        1,
    );
    assert!(matches!(
        PluginManifest::parse(&invalid_description),
        Err(ManifestError::InvalidField {
            field: ManifestField::Description,
            reason: InvalidFieldReason::TooLong {
                max_bytes: 1000,
                actual_bytes: 1001,
            },
        })
    ));

    let license_boundary = "a".repeat(256);
    let valid_license = format!("{MINIMAL_MANIFEST}license = {license_boundary:?}\n");
    assert!(PluginManifest::parse(&valid_license).is_ok());

    let license_over_limit = format!("{license_boundary}a");
    let invalid_license = format!("{MINIMAL_MANIFEST}license = {license_over_limit:?}\n");
    assert!(matches!(
        PluginManifest::parse(&invalid_license),
        Err(ManifestError::InvalidField {
            field: ManifestField::License,
            reason: InvalidFieldReason::TooLong {
                max_bytes: 256,
                actual_bytes: 257,
            },
        })
    ));
}

/// Verifies complete SemVer prerelease and build metadata are retained.
#[test]
fn parses_full_semantic_version_syntax() {
    let source = MINIMAL_MANIFEST.replacen(
        "version = \"1.2.0\"",
        "version = \"1.2.0-beta.1+build.7\"",
        1,
    );
    let manifest = success(PluginManifest::parse(&source), "full semantic version");
    let expected = success(
        Version::parse("1.2.0-beta.1+build.7"),
        "full semantic version",
    );

    assert_eq!(manifest.version(), &expected);
}

/// Verifies license text remains ASCII and is not silently normalized.
#[test]
fn rejects_invalid_license_text() {
    for (license, expected) in [
        (" MIT", InvalidFieldReason::LeadingOrTrailingWhitespace),
        ("许可证", InvalidFieldReason::NonAscii),
    ] {
        let source = format!("{MINIMAL_MANIFEST}license = {license:?}\n");
        let result = PluginManifest::parse(&source);

        match (result, expected) {
            (
                Err(ManifestError::InvalidField {
                    field: ManifestField::License,
                    reason: InvalidFieldReason::LeadingOrTrailingWhitespace,
                }),
                InvalidFieldReason::LeadingOrTrailingWhitespace,
            )
            | (
                Err(ManifestError::InvalidField {
                    field: ManifestField::License,
                    reason: InvalidFieldReason::NonAscii,
                }),
                InvalidFieldReason::NonAscii,
            ) => {}
            _ => panic!("unexpected license validation result"),
        }
    }
}

/// Verifies head branch errors retain the shared structured validation error.
#[test]
fn preserves_git_branch_error_in_head_field() {
    let source = format!(
        "{MINIMAL_MANIFEST}\n[head]\nrepository = \"https://example.com/repo.git\"\nbranch = \"feature//api\"\n"
    );

    assert!(matches!(
        PluginManifest::parse(&source),
        Err(ManifestError::InvalidField {
            field: ManifestField::HeadBranch,
            reason: InvalidFieldReason::InvalidGitBranch(GitBranchNameError::ConsecutiveSlashes),
        })
    ));
}

/// Verifies missing and unknown head members remain structural TOML errors.
#[test]
fn rejects_incomplete_or_unknown_head_fields() {
    let missing_branch =
        format!("{MINIMAL_MANIFEST}\n[head]\nrepository = \"https://example.com/repo.git\"\n");
    let unknown = format!(
        "{MINIMAL_MANIFEST}\n[head]\nrepository = \"https://example.com/repo.git\"\nbranch = \"main\"\nother = true\n"
    );

    for source in [missing_branch, unknown] {
        assert!(matches!(
            PluginManifest::parse(&source),
            Err(ManifestError::InvalidToml { .. })
        ));
    }
}

/// Verifies dependency parsing accepts SemVer requirement composition and rejects unknown keys.
#[test]
fn parses_only_the_ora_dependency() {
    let valid = format!("{MINIMAL_MANIFEST}\n[dependencies]\nora = \"^1.2, <2\"\n");
    let manifest = success(PluginManifest::parse(&valid), "Ora dependency");
    let expected = success(VersionReq::parse("^1.2, <2"), "Ora requirement");
    assert_eq!(
        manifest.dependencies().map(PluginDependencies::ora),
        Some(&expected)
    );

    let unknown = format!("{MINIMAL_MANIFEST}\n[dependencies]\nother = \"1\"\n");
    assert!(matches!(
        PluginManifest::parse(&unknown),
        Err(ManifestError::InvalidToml { .. })
    ));
}

/// Verifies field paths have stable dotted representations for programmatic diagnostics.
#[test]
fn formats_structured_manifest_fields() {
    assert_eq!(ManifestField::HeadRepository.to_string(), "head.repository");
    assert_eq!(
        ManifestField::UiSurface {
            index: 2,
            field: SurfaceField::WebDataMode,
        }
        .to_string(),
        "ui.surfaces[2].web_data.mode"
    );
    assert_eq!(
        ManifestField::DependenciesOra.to_string(),
        "dependencies.ora"
    );
}

const UI_MANIFEST: &str = r#"name = "ora-space.skillhub"
namespace = "official"
kind = "ui"
version = "0.1.0"
description = "Ora Space SkillHub surface"

[[ui.surfaces]]
id = "market"
title = "SkillHub"
instances = "singleton"

[ui.surfaces.source]
kind = "remote_site"
entry = "https://www.skillhub.cn"
hosts = ["skillhub.cn", "www.skillhub.cn"]

[ui.surfaces.web_data]
mode = "persistent"

[[ui.surfaces]]
id = "counter"
title = "Hello Panel"

[ui.surfaces.source]
kind = "panel"
root = "ui"
entry = "index.html"
"#;

/// Verifies a ui manifest keeps every surface verbatim, defaulting only `instances`.
#[test]
fn parses_ui_section_into_surface_declarations() {
    let manifest = success(PluginManifest::parse_installed(UI_MANIFEST), "ui manifest");

    assert_eq!(manifest.kind(), PluginKind::Ui);
    assert_eq!(
        manifest.ui(),
        Some(&PluginUi {
            surfaces: vec![
                SurfaceDeclaration {
                    id: success(Slug::parse("market"), "surface id"),
                    title: "SkillHub".to_owned(),
                    instances: SurfaceInstances::Singleton,
                    source: SurfaceSource::RemoteSite {
                        entry: "https://www.skillhub.cn".to_owned(),
                        hosts: vec!["skillhub.cn".to_owned(), "www.skillhub.cn".to_owned()],
                        host_suffixes: vec![],
                    },
                    web_data: Some(WebDataMode::Persistent),
                },
                SurfaceDeclaration {
                    id: success(Slug::parse("counter"), "surface id"),
                    title: "Hello Panel".to_owned(),
                    instances: SurfaceInstances::Singleton,
                    source: SurfaceSource::Panel {
                        root: "ui".to_owned(),
                        entry: "index.html".to_owned(),
                    },
                    web_data: None,
                },
            ],
        })
    );
}

/// Verifies the release form accepts a `[ui]` section too, so marketplace listings can show
/// what a ui plugin contributes before it is installed.
#[test]
fn release_manifest_accepts_ui_section() {
    let source = format!(
        "resolver = 1\nurl = \"https://example.com/skillhub.orax\"\nsha256 = \"{DIGEST}\"\n{UI_MANIFEST}"
    );
    let manifest = success(PluginManifest::parse(&source), "ui release manifest");

    assert_eq!(manifest.ui().map(|ui| ui.surfaces().len()), Some(2));
}

/// Verifies `[ui]` is required for ui plugins and refused for every other kind.
#[test]
fn pairs_ui_section_with_kind() {
    let missing = UI_MANIFEST
        .split("\n[[ui.surfaces]]")
        .next()
        .unwrap_or_default();
    assert!(matches!(
        PluginManifest::parse_installed(missing),
        Err(ManifestError::InvalidField {
            field: ManifestField::Ui,
            reason: InvalidFieldReason::MissingForKind {
                kind: PluginKind::Ui
            },
        })
    ));

    let agent = UI_MANIFEST.replacen("kind = \"ui\"", "kind = \"agent\"", 1);
    assert!(matches!(
        PluginManifest::parse_installed(&agent),
        Err(ManifestError::InvalidField {
            field: ManifestField::Ui,
            reason: InvalidFieldReason::NotAllowedForKind {
                kind: PluginKind::Agent
            },
        })
    ));
}

/// Verifies each surface shape rule is attributed to the indexed surface field.
#[test]
fn rejects_invalid_surface_fields_with_index() {
    let empty = UI_MANIFEST.replacen("[[ui.surfaces]]\nid = \"market\"\ntitle = \"SkillHub\"\ninstances = \"singleton\"\n\n[ui.surfaces.source]\nkind = \"remote_site\"\nentry = \"https://www.skillhub.cn\"\nhosts = [\"skillhub.cn\", \"www.skillhub.cn\"]\n\n[ui.surfaces.web_data]\nmode = \"persistent\"\n\n[[ui.surfaces]]\nid = \"counter\"\ntitle = \"Hello Panel\"\n\n[ui.surfaces.source]\nkind = \"panel\"\nroot = \"ui\"\nentry = \"index.html\"\n", "[ui]\nsurfaces = []\n", 1);
    assert!(matches!(
        PluginManifest::parse_installed(&empty),
        Err(ManifestError::InvalidField {
            field: ManifestField::UiSurfaces,
            reason: InvalidFieldReason::Empty,
        })
    ));

    let cases = [
        (
            ("id = \"counter\"", "id = \"Counter\""),
            ManifestField::UiSurface {
                index: 1,
                field: SurfaceField::Id,
            },
        ),
        (
            ("title = \"SkillHub\"", "title = \"   \""),
            ManifestField::UiSurface {
                index: 0,
                field: SurfaceField::Title,
            },
        ),
        (
            ("instances = \"singleton\"", "instances = \"many\""),
            ManifestField::UiSurface {
                index: 0,
                field: SurfaceField::Instances,
            },
        ),
        (
            ("mode = \"persistent\"", "mode = \"persistent_profile\""),
            ManifestField::UiSurface {
                index: 0,
                field: SurfaceField::WebDataMode,
            },
        ),
    ];
    for ((valid, broken), expected_field) in cases {
        let source = UI_MANIFEST.replacen(valid, broken, 1);
        let Err(ManifestError::InvalidField { field, .. }) =
            PluginManifest::parse_installed(&source)
        else {
            panic!("expected {broken} to produce a semantic field error");
        };
        assert_eq!(field, expected_field);
    }

    let uppercase = UI_MANIFEST.replacen("id = \"counter\"", "id = \"Counter\"", 1);
    assert!(matches!(
        PluginManifest::parse_installed(&uppercase),
        Err(ManifestError::InvalidField {
            reason: InvalidFieldReason::InvalidSlug(SlugError::InvalidCharacter { .. }),
            ..
        })
    ));
}

/// Verifies an unknown source kind, a field of the other source form, and an unknown surface
/// field stay structural errors that name the offending TOML path.
#[test]
fn reports_structural_surface_errors_with_paths() {
    let cases = [
        (
            UI_MANIFEST.replacen("kind = \"panel\"", "kind = \"native_view\"", 1),
            "ui.surfaces[1].source.kind",
        ),
        (
            UI_MANIFEST.replacen("root = \"ui\"\n", "root = \"ui\"\nhosts = []\n", 1),
            "ui.surfaces[1].source",
        ),
        (
            UI_MANIFEST.replacen(
                "title = \"SkillHub\"\n",
                "title = \"SkillHub\"\nicon = \"x\"\n",
                1,
            ),
            "ui.surfaces[0].icon",
        ),
    ];
    for (source, expected_path) in cases {
        let Err(ManifestError::InvalidToml { path, .. }) = PluginManifest::parse_installed(&source)
        else {
            panic!("expected {expected_path} to fail structurally");
        };
        assert_eq!(path.as_deref(), Some(expected_path));
    }
}
