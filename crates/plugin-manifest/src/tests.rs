use crate::{
    DownloadAction, DownloadDisposition, DownloadPolicy, DownloadRule, HomepageUrl,
    HookTargetError, InvalidFieldReason, ManifestError, ManifestField, MethodName, MethodNameError,
    Origin, PageMatcher, PathPrefix, PluginDependencies, PluginHead, PluginKind, PluginManifest,
    PluginName, PluginReleaseSource, PluginWebview, PluginWorkbench, ReleaseUrl, RepositoryUrl,
    RuleField, Sha256Digest, StartUrl,
};
use ora_utils::{GitBranchName, GitBranchNameError};
use pretty_assertions::assert_eq;
use semver::{Version, VersionReq};

const DIGEST: &str = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a";

const MINIMAL_MANIFEST: &str = r#"resolver = 1
identifier = "user.ora-weather"
kind = "workbench"
version = "1.2.0"
description = "获取实时天气信息的 Ora 插件"
url = "https://example.com/ora-weather.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"
"#;

/// The installed-package spelling of the minimal manifest: it drops the download fields and
/// spells the name segment `identifier`, matching what a shipped `orax.toml` contains.
const INSTALLED_MINIMAL_MANIFEST: &str = r#"resolver = 1
identifier = "user.ora-weather"
kind = "workbench"
version = "1.2.0"
description = "????????? Ora ??"
"#;

const FULL_MANIFEST: &str = r#"resolver = 1
identifier = "user.ora-weather"
title = "Ora Weather"
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
        title: "Ora Weather".to_owned(),
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
        workbench: None,
        webview: None,
        release_source: Some(PluginReleaseSource::Universal {
            url: success(
                ReleaseUrl::parse(
                    "https://github.com/user/ora-weather/releases/download/v1.2.0/ora-weather.orax?signature=abc",
                ),
                "release URL",
            ),
            sha256: success(Sha256Digest::parse(DIGEST), "digest"),
        }),
        artifact: None,
    };

    assert_eq!(actual, expected);
}

/// Verifies every optional field can be absent without manufacturing defaults.
#[test]
fn parses_manifest_without_optional_fields() {
    let manifest = success(PluginManifest::parse(MINIMAL_MANIFEST), "minimal manifest");

    assert_eq!(manifest.resolver(), 1);
    assert_eq!(manifest.name().as_str(), "user.ora-weather");
    assert_eq!(manifest.title(), "user.ora-weather");
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

/// Verifies the Skill kind is accepted by the marketplace manifest schema.
#[test]
fn parses_skill_kind_marketplace_manifest() {
    let manifest = success(
        PluginManifest::parse(&MINIMAL_MANIFEST.replacen("workbench", "skill", 1)),
        "skill-kind marketplace manifest",
    );

    assert_eq!(manifest.kind(), PluginKind::Skill);
    assert_eq!(manifest.kind().as_str(), "skill");
}

/// Verifies the MCP kind is accepted by the marketplace manifest schema.
#[test]
fn parses_mcp_kind_marketplace_manifest() {
    let manifest = success(
        PluginManifest::parse(&MINIMAL_MANIFEST.replacen("workbench", "mcp", 1)),
        "mcp-kind marketplace manifest",
    );

    assert_eq!(manifest.kind(), PluginKind::Mcp);
    assert_eq!(manifest.kind().as_str(), "mcp");
}

/// Verifies MCP plugins cannot smuggle either process-facing kind-specific section.
#[test]
fn mcp_kind_rejects_workbench_and_webview_sections() {
    let workbench = WORKBENCH_MANIFEST.replacen("kind = \"workbench\"", "kind = \"mcp\"", 1);
    let webview = WEBVIEW_MANIFEST.replacen("kind = \"webview\"", "kind = \"mcp\"", 1);

    assert!(matches!(
        PluginManifest::parse_installed(&workbench),
        Err(ManifestError::InvalidField {
            field: ManifestField::Workbench,
            reason: InvalidFieldReason::NotAllowedForKind {
                kind: PluginKind::Mcp
            },
        })
    ));
    assert!(matches!(
        PluginManifest::parse_installed(&webview),
        Err(ManifestError::InvalidField {
            field: ManifestField::Webview,
            reason: InvalidFieldReason::NotAllowedForKind {
                kind: PluginKind::Mcp
            },
        })
    ));
}

/// Verifies a local Skill plugin needs only the installed-manifest core fields.
#[test]
fn parses_installed_skill_manifest_without_resolver_or_download_fields() {
    let installed = "identifier = \"user.skill-pack\"\nkind = \"skill\"\nversion = \"1.2.0\"\ndescription = \"A Skill package\"\n";
    let manifest = success(
        PluginManifest::parse_installed(installed),
        "installed Skill manifest",
    );

    assert_eq!(manifest.resolver(), 1);
    assert_eq!(manifest.kind(), PluginKind::Skill);
    assert_eq!(manifest.release(), None);
}

/// Verifies Skill plugins cannot smuggle either existing kind-specific section.
#[test]
fn skill_kind_rejects_workbench_and_webview_sections() {
    let workbench = WORKBENCH_MANIFEST.replacen("kind = \"workbench\"", "kind = \"skill\"", 1);
    let webview = WEBVIEW_MANIFEST.replacen("kind = \"webview\"", "kind = \"skill\"", 1);

    assert!(matches!(
        PluginManifest::parse_installed(&workbench),
        Err(ManifestError::InvalidField {
            field: ManifestField::Workbench,
            reason: InvalidFieldReason::NotAllowedForKind {
                kind: PluginKind::Skill
            },
        })
    ));
    assert!(matches!(
        PluginManifest::parse_installed(&webview),
        Err(ManifestError::InvalidField {
            field: ManifestField::Webview,
            reason: InvalidFieldReason::NotAllowedForKind {
                kind: PluginKind::Skill
            },
        })
    ));
}
/// Verifies an installed package manifest omits download-only fields and still parses.
#[test]
fn parses_installed_manifest_without_download_fields() {
    let installed = "identifier = \"user.ora-weather\"\nkind = \"workbench\"\nversion = \"1.2.0\"\ndescription = \"A test plugin\"\n";
    let manifest = success(
        PluginManifest::parse_installed(installed),
        "installed manifest",
    );

    assert_eq!(manifest.resolver(), 1);
    assert_eq!(manifest.name().as_str(), "user.ora-weather");
    assert_eq!(manifest.title(), "user.ora-weather");
    assert_eq!(manifest.kind(), PluginKind::Workbench);
    assert_eq!(manifest.url(), None);
    assert_eq!(manifest.sha256(), None);
    assert_eq!(manifest.release(), None);
}

/// Verifies the current installed-package schema, which spells the name segment `identifier` and
/// adds a human-readable `title`, parses into a manifest that exposes both.
#[test]
fn parses_new_installed_schema_with_title() {
    let source = r#"resolver = 1
title = "OpenCode"
identifier = "ora-space.opencode"
kind = "agent"
version = "0.1.2"
description = "Ora Space OpenCode Agent"
homepage = "https://github.com/ora-space/opencode-agent"
license = "Apache-2.0"
"#;
    let manifest = success(
        PluginManifest::parse_installed(source),
        "new installed schema",
    );
    assert_eq!(manifest.title(), "OpenCode");
    assert_eq!(manifest.name().as_str(), "ora-space.opencode");
    assert_eq!(manifest.kind(), PluginKind::Agent);
    assert_eq!(
        manifest.homepage().map(HomepageUrl::as_str),
        Some("https://github.com/ora-space/opencode-agent")
    );
    assert_eq!(manifest.license(), Some("Apache-2.0"));
    assert_eq!(manifest.url(), None);
    assert_eq!(manifest.sha256(), None);
}

/// Verifies the marketplace release form accepts the same new schema: `identifier`/`title` with
/// no `.orax` download fields, so a synced registry entry is no longer skipped for display.
#[test]
fn parses_new_marketplace_schema_without_download_fields() {
    let source = r#"resolver = 1
title = "OpenCode"
identifier = "ora-space.opencode"
kind = "agent"
version = "0.1.2"
description = "Ora Space OpenCode Agent"
"#;
    let manifest = success(PluginManifest::parse(source), "new marketplace schema");
    assert_eq!(manifest.title(), "OpenCode");
    assert_eq!(manifest.name().as_str(), "ora-space.opencode");
    assert_eq!(manifest.kind(), PluginKind::Agent);
    assert_eq!(manifest.url(), None);
    assert_eq!(manifest.sha256(), None);
    assert_eq!(manifest.release(), None);
}

/// Verifies the full marketplace release schema, including Markdown-linked `homepage`/`url`, parses
/// and exposes the download metadata the installer needs.
#[test]
fn parses_full_new_marketplace_manifest_with_markdown_urls() {
    let source = r#"resolver = 1
title = "OpenCode"
identifier = "ora-space.opencode"
kind = "agent"
version = "0.1.2"
description = "Ora Space OpenCode Agent"
homepage = "[https://github.com/ora-space/opencode-agent](https://github.com/ora-space/opencode-agent)"
license = "Apache-2.0"
url = "[https://github.com/ora-space/opencode-agent/releases/download/v0.1.2/ora-space.opencode-v0.1.2.orax](https://github.com/ora-space/opencode-agent/releases/download/v0.1.2/ora-space.opencode-v0.1.2.orax)"
sha256 = "18263de8e26fab1ea64d6c24913f0815d2151e0ae49cea9ef8aa46f453798558"
"#;
    let manifest = success(
        PluginManifest::parse(source),
        "full new marketplace manifest",
    );

    assert_eq!(manifest.title(), "OpenCode");
    assert_eq!(manifest.name().as_str(), "ora-space.opencode");
    assert_eq!(manifest.kind(), PluginKind::Agent);
    assert_eq!(
        manifest.homepage().map(HomepageUrl::as_str),
        Some("https://github.com/ora-space/opencode-agent")
    );
    assert_eq!(
        manifest.url().map(ReleaseUrl::as_str),
        Some(
            "https://github.com/ora-space/opencode-agent/releases/download/v0.1.2/ora-space.opencode-v0.1.2.orax"
        )
    );
    assert_eq!(
        manifest.sha256().map(ToString::to_string),
        Some("18263de8e26fab1ea64d6c24913f0815d2151e0ae49cea9ef8aa46f453798558".to_owned())
    );
    assert!(manifest.release().is_some());
}

/// Verifies unsupported resolver versions take priority over semantic field validation.
#[test]
fn rejects_unsupported_resolver_before_fields() {
    let source = MINIMAL_MANIFEST
        .replacen("resolver = 1", "resolver = 2", 1)
        .replacen(
            "identifier = \"user.ora-weather\"",
            "identifier = \"INVALID\"",
            1,
        );

    assert!(matches!(
        PluginManifest::parse(&source),
        Err(ManifestError::UnsupportedResolver { found: 2 })
    ));
}

/// Verifies missing and mistyped required fields stay structural TOML errors with spans.
#[test]
fn reports_structural_toml_errors_with_spans() {
    let cases = [
        MINIMAL_MANIFEST.replacen("resolver = 1\n", "", 1),
        MINIMAL_MANIFEST.replacen("resolver = 1", "resolver = \"one\"", 1),
    ];

    for source in cases {
        assert!(matches!(
            PluginManifest::parse(&source),
            Err(ManifestError::InvalidToml { span: Some(_), .. })
        ));
    }
}

/// Verifies an unknown field is ignored by both manifest forms rather than discarding the entry.
///
/// Manifests are written by third-party authors and read by many Ora versions at once. Rejecting
/// an unrecognized key would make every purely additive schema change delete the whole listing
/// from an older client's marketplace, so schema increments are absorbed here and only breaking
/// changes are gated by `resolver`. The residual `namespace` key a pre-derivation manifest may
/// still carry is one such unknown field: it is ignored, and it never reaches the parsed value.
#[test]
fn ignores_unknown_manifest_fields_in_both_forms() {
    let release = MINIMAL_MANIFEST.replacen(
        "kind = \"workbench\"\n",
        "kind = \"workbench\"\nnamespace = \"community\"\nfuture_optional_field = 7\n",
        1,
    );
    let installed = format!(
        "identifier = \"user.ora-weather\"\nnamespace = \"attacker\"\nkind = \"workbench\"\n\
         version = \"1.2.0\"\ndescription = \"A test plugin\"\nfuture_section_hint = [\"x\"]\n"
    );

    assert_eq!(
        (
            success(PluginManifest::parse(&release), "release manifest"),
            success(
                PluginManifest::parse_installed(&installed),
                "installed manifest",
            )
            .name()
            .as_str()
            .to_owned(),
        ),
        (
            success(PluginManifest::parse(MINIMAL_MANIFEST), "minimal manifest"),
            "user.ora-weather".to_owned(),
        ),
    );
}

/// Verifies tolerating unknown fields did not relax the required ones: a missing required field
/// is still a hard structural error that reports its TOML path.
#[test]
fn still_rejects_missing_required_fields_after_relaxing_unknown_ones() {
    let without_identifier = MINIMAL_MANIFEST.replacen(
        "identifier = \"user.ora-weather\"\n",
        "unrelated_new_key = \"user.ora-weather\"\n",
        1,
    );

    let Err(ManifestError::InvalidToml { source, path, .. }) =
        PluginManifest::parse(&without_identifier)
    else {
        panic!("expected a missing required field to stay a hard structural error");
    };
    assert_eq!(
        (source.message().contains("identifier"), path),
        (true, None),
    );
}

/// Verifies every required root field is rejected when missing or assigned the wrong TOML type.
#[test]
fn rejects_missing_and_mistyped_required_fields() {
    let fields = [
        ("resolver = 1\n", "resolver = \"one\"\n"),
        ("identifier = \"user.ora-weather\"\n", "identifier = true\n"),
        ("kind = \"workbench\"\n", "kind = true\n"),
        ("version = \"1.2.0\"\n", "version = true\n"),
        (
            "description = \"获取实时天气信息的 Ora 插件\"\n",
            "description = true\n",
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
            "identifier = \"user.ora-weather\"",
            "identifier = \"\"",
            ManifestField::Identifier,
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
        .replacen(
            "identifier = \"user.ora-weather\"",
            "identifier = \"INVALID\"",
            1,
        )
        .replacen("kind = \"workbench\"", "kind = \"gadget\"", 1);

    assert!(matches!(
        PluginManifest::parse(&source),
        Err(ManifestError::InvalidField {
            field: ManifestField::Identifier,
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

/// Verifies a nested section keeps its own required members while tolerating unknown ones.
///
/// Unknown-field tolerance is a whole-schema property, not a root-table one: a new optional key
/// inside `[head]` must not delete the listing from an older client either.
#[test]
fn rejects_incomplete_head_but_ignores_unknown_head_fields() {
    let missing_branch =
        format!("{MINIMAL_MANIFEST}\n[head]\nrepository = \"https://example.com/repo.git\"\n");
    let unknown = format!(
        "{MINIMAL_MANIFEST}\n[head]\nrepository = \"https://example.com/repo.git\"\nbranch = \"main\"\nother = true\n"
    );

    assert!(matches!(
        PluginManifest::parse(&missing_branch),
        Err(ManifestError::InvalidToml { .. })
    ));
    assert_eq!(
        success(PluginManifest::parse(&unknown), "unknown head field")
            .head()
            .map(|head| head.branch().as_str().to_owned()),
        Some("main".to_owned()),
    );
}

/// Verifies dependency parsing accepts SemVer requirement composition and reads only `ora`.
///
/// A dependency key this Ora does not know is ignored like any other unknown field, so the
/// manifest declares no host requirement rather than failing to parse.
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
    assert_eq!(
        success(PluginManifest::parse(&unknown), "unknown dependency key").dependencies(),
        None,
    );
}

/// Verifies field paths have stable dotted representations for programmatic diagnostics.
#[test]
fn formats_structured_manifest_fields() {
    assert_eq!(ManifestField::HeadRepository.to_string(), "head.repository");
    assert_eq!(
        ManifestField::WebviewDownloadRule {
            index: 2,
            field: RuleField::PagePathPrefix,
        }
        .to_string(),
        "webview.downloads.rules[2].page.path_prefix"
    );
    assert_eq!(
        ManifestField::DependenciesOra.to_string(),
        "dependencies.ora"
    );
}

const WORKBENCH_MANIFEST: &str = r#"resolver = 1
identifier = "user.ora-weather"
kind = "workbench"
version = "1.2.0"
description = "Weather panel"

[workbench]
methods = ["weather/get_current", "weather/search_city"]
"#;

const WEBVIEW_MANIFEST: &str = r#"resolver = 1
identifier = "acme.hub"
kind = "webview"
version = "1.0.0"
description = "An example marketplace surface"

[webview]
start_url = "https://www.example.com"
allowed_origins = ["https://www.example.com", "https://example.com"]

[webview.downloads]
fallback = { prompt = ["save_as"] }

[[webview.downloads.rules]]
page = { origin = "https://www.example.com", path_prefix = "/skills/" }
action = { auto = "import_skill" }

[[webview.downloads.rules]]
page = { origin = "https://www.example.com", path_prefix = "/downloads/" }
action = { prompt = ["import_skill", "save_as"] }
"#;

/// Verifies a workbench manifest keeps its page-visible methods in declaration order.
#[test]
fn parses_workbench_section_into_method_list() {
    let manifest = success(
        PluginManifest::parse_installed(WORKBENCH_MANIFEST),
        "workbench manifest",
    );

    assert_eq!(manifest.kind(), PluginKind::Workbench);
    assert_eq!(
        (manifest.workbench(), manifest.webview()),
        (
            Some(&PluginWorkbench {
                methods: vec![
                    success(MethodName::parse("weather/get_current"), "method"),
                    success(MethodName::parse("weather/search_city"), "method"),
                ],
            }),
            None,
        )
    );
}

/// Verifies a workbench plugin may omit `[workbench]` (a static page) while other kinds may
/// not declare it.
#[test]
fn workbench_section_is_optional_and_kind_exclusive() {
    let static_page = success(
        PluginManifest::parse_installed(INSTALLED_MINIMAL_MANIFEST),
        "static workbench",
    );
    assert_eq!(static_page.workbench(), None);

    let agent = WORKBENCH_MANIFEST.replacen("kind = \"workbench\"", "kind = \"agent\"", 1);
    assert!(matches!(
        PluginManifest::parse_installed(&agent),
        Err(ManifestError::InvalidField {
            field: ManifestField::Workbench,
            reason: InvalidFieldReason::NotAllowedForKind {
                kind: PluginKind::Agent
            },
        })
    ));
}

/// Verifies method names are validated per entry and duplicates are refused.
#[test]
fn rejects_invalid_and_duplicate_workbench_methods() {
    let cases = [
        (
            "methods = [\"ora/storage/read\"]",
            ManifestField::WorkbenchMethod { index: 0 },
        ),
        (
            "methods = [\"weather/get_current\", \"Weather/Get\"]",
            ManifestField::WorkbenchMethod { index: 1 },
        ),
        (
            "methods = [\"weather/get_current\", \"weather/get_current\"]",
            ManifestField::WorkbenchMethod { index: 1 },
        ),
        ("methods = []", ManifestField::WorkbenchMethods),
    ];
    for (broken, expected_field) in cases {
        let source = WORKBENCH_MANIFEST.replacen(
            "methods = [\"weather/get_current\", \"weather/search_city\"]",
            broken,
            1,
        );
        let Err(ManifestError::InvalidField { field, .. }) =
            PluginManifest::parse_installed(&source)
        else {
            panic!("expected {broken} to produce a semantic field error");
        };
        assert_eq!(field, expected_field, "{broken}");
    }

    assert!(matches!(
        MethodName::parse("ora/anything"),
        Err(MethodNameError::ReservedPrefix)
    ));
    assert!(matches!(
        MethodName::parse("weather//get"),
        Err(MethodNameError::EmptySegment)
    ));
}

/// Verifies a webview manifest maps to normalized origins, ordered rules, and its fallback.
#[test]
fn parses_webview_section_into_download_policy() {
    let manifest = success(
        PluginManifest::parse_installed(WEBVIEW_MANIFEST),
        "webview manifest",
    );
    let origin = |value: &str| success(Origin::parse(value), "origin");
    let rule = |prefix: &str, disposition: DownloadDisposition| DownloadRule {
        page: PageMatcher {
            origin: origin("https://www.example.com"),
            path_prefix: success(PathPrefix::parse(prefix), "prefix"),
        },
        disposition,
    };

    assert_eq!(manifest.kind(), PluginKind::Webview);
    assert_eq!(
        manifest.webview(),
        Some(&PluginWebview {
            start_url: success(StartUrl::parse("https://www.example.com"), "start url"),
            allowed_origins: vec![
                origin("https://www.example.com"),
                origin("https://example.com"),
            ],
            downloads: DownloadPolicy {
                rules: vec![
                    rule(
                        "/skills/",
                        DownloadDisposition::Auto {
                            action: DownloadAction::ImportSkill,
                        },
                    ),
                    rule(
                        "/downloads/",
                        DownloadDisposition::Prompt {
                            actions: vec![DownloadAction::ImportSkill, DownloadAction::SaveAs],
                        },
                    ),
                ],
                fallback: DownloadDisposition::Prompt {
                    actions: vec![DownloadAction::SaveAs],
                },
            },
        })
    );
}

/// Verifies omitting `[webview.downloads]` rejects every download rather than allowing any.
#[test]
fn webview_downloads_default_to_reject() {
    let source = WEBVIEW_MANIFEST
        .split("\n[webview.downloads]")
        .next()
        .unwrap_or_default();
    let manifest = success(PluginManifest::parse_installed(source), "webview manifest");

    assert_eq!(
        manifest.webview().map(PluginWebview::downloads),
        Some(&DownloadPolicy::default())
    );
}

/// Verifies `[webview]` is required for webview plugins and refused for every other kind.
#[test]
fn pairs_webview_section_with_kind() {
    let missing = WEBVIEW_MANIFEST
        .split("\n[webview]")
        .next()
        .unwrap_or_default();
    assert!(matches!(
        PluginManifest::parse_installed(missing),
        Err(ManifestError::InvalidField {
            field: ManifestField::Webview,
            reason: InvalidFieldReason::MissingForKind {
                kind: PluginKind::Webview
            },
        })
    ));

    let workbench = WEBVIEW_MANIFEST.replacen("kind = \"webview\"", "kind = \"workbench\"", 1);
    assert!(matches!(
        PluginManifest::parse_installed(&workbench),
        Err(ManifestError::InvalidField {
            field: ManifestField::Webview,
            reason: InvalidFieldReason::NotAllowedForKind {
                kind: PluginKind::Workbench
            },
        })
    ));
}

/// Verifies each webview value rule is attributed to its indexed field.
#[test]
fn rejects_invalid_webview_fields_with_index() {
    let cases = [
        (
            (
                "start_url = \"https://www.example.com\"",
                "start_url = \"http://www.example.com\"",
            ),
            ManifestField::WebviewStartUrl,
        ),
        (
            (
                "start_url = \"https://www.example.com\"",
                "start_url = \"https://www.example.com/#top\"",
            ),
            ManifestField::WebviewStartUrl,
        ),
        (
            (
                "\"https://example.com\"]",
                "\"https://example.com/skills\"]",
            ),
            ManifestField::WebviewAllowedOrigin { index: 1 },
        ),
        (
            (
                "allowed_origins = [\"https://www.example.com\", \"https://example.com\"]",
                "allowed_origins = []",
            ),
            ManifestField::WebviewAllowedOrigins,
        ),
        (
            (
                "fallback = { prompt = [\"save_as\"] }",
                "fallback = { prompt = [] }",
            ),
            ManifestField::WebviewDownloadsFallback,
        ),
        (
            (
                "fallback = { prompt = [\"save_as\"] }",
                "fallback = { auto = \"import_skill\", prompt = [\"save_as\"] }",
            ),
            ManifestField::WebviewDownloadsFallback,
        ),
        (
            (
                "action = { auto = \"import_skill\" }",
                "action = { auto = \"run_binary\" }",
            ),
            ManifestField::WebviewDownloadRule {
                index: 0,
                field: RuleField::Action,
            },
        ),
        // `save_as` needs a user-chosen destination, so it can never run automatically.
        (
            (
                "action = { auto = \"import_skill\" }",
                "action = { auto = \"save_as\" }",
            ),
            ManifestField::WebviewDownloadRule {
                index: 0,
                field: RuleField::Action,
            },
        ),
        (
            (
                "action = { prompt = [\"import_skill\", \"save_as\"] }",
                "action = { prompt = [\"save_as\", \"save_as\"] }",
            ),
            ManifestField::WebviewDownloadRule {
                index: 1,
                field: RuleField::Action,
            },
        ),
        (
            (
                "path_prefix = \"/downloads/\"",
                "path_prefix = \"downloads/\"",
            ),
            ManifestField::WebviewDownloadRule {
                index: 1,
                field: RuleField::PagePathPrefix,
            },
        ),
        (
            (
                "page = { origin = \"https://www.example.com\", path_prefix = \"/downloads/\" }",
                "page = { origin = \"https://www.example.com/x\", path_prefix = \"/downloads/\" }",
            ),
            ManifestField::WebviewDownloadRule {
                index: 1,
                field: RuleField::PageOrigin,
            },
        ),
    ];
    for ((valid, broken), expected_field) in cases {
        let source = WEBVIEW_MANIFEST.replacen(valid, broken, 1);
        assert_ne!(source, WEBVIEW_MANIFEST, "{broken} did not apply");
        let Err(ManifestError::InvalidField { field, .. }) =
            PluginManifest::parse_installed(&source)
        else {
            panic!("expected {broken} to produce a semantic field error");
        };
        assert_eq!(field, expected_field, "{broken}");
    }
}

/// Verifies a missing nested member still fails structurally and names its TOML path, while an
/// unknown key at any nesting depth is ignored.
///
/// The nested paths are what make this worth asserting separately: `serde_path_to_error` is the
/// only reason a failure inside `[[webview.downloads.rules]]` says which entry broke.
#[test]
fn reports_structural_webview_errors_with_paths() {
    let missing_page_member = WEBVIEW_MANIFEST.replacen(
        "page = { origin = \"https://www.example.com\", path_prefix = \"/skills/\" }",
        "page = { origin = \"https://www.example.com\" }",
        1,
    );
    assert_ne!(missing_page_member, WEBVIEW_MANIFEST);
    let Err(ManifestError::InvalidToml { path, .. }) =
        PluginManifest::parse_installed(&missing_page_member)
    else {
        panic!("expected a missing rule member to fail structurally");
    };
    assert_eq!(path.as_deref(), Some("webview.downloads.rules[0].page"));

    let unknown_keys = [
        WEBVIEW_MANIFEST.replacen(
            "allowed_origins = [",
            "user_agent = \"x\"\nallowed_origins = [",
            1,
        ),
        WEBVIEW_MANIFEST.replacen(
            "path_prefix = \"/skills/\" }",
            "path_prefix = \"/skills/\", query = \"a\" }",
            1,
        ),
    ];
    for source in unknown_keys {
        assert_ne!(source, WEBVIEW_MANIFEST);
        assert_eq!(
            success(
                PluginManifest::parse_installed(&source),
                "webview manifest with an unknown key",
            ),
            success(
                PluginManifest::parse_installed(WEBVIEW_MANIFEST),
                "webview manifest",
            ),
        );
    }
}

/// Verifies an action table naming only keys this Ora does not know is a semantic ambiguity
/// rather than a parse failure.
///
/// Ignoring unknown fields moves this case from "unknown key" to "no action declared", which is
/// exactly the tradeoff D1 accepts: the misspelling surfaces one step later, as the rule failing
/// to declare what to do, and never as the whole listing disappearing.
#[test]
fn reports_an_action_without_a_known_key_as_ambiguous() {
    let source = WEBVIEW_MANIFEST.replacen(
        "action = { auto = \"import_skill\" }",
        "action = { run = \"x\" }",
        1,
    );
    assert_ne!(source, WEBVIEW_MANIFEST);

    assert!(matches!(
        PluginManifest::parse_installed(&source),
        Err(ManifestError::InvalidField {
            reason: InvalidFieldReason::AmbiguousDownloadAction,
            ..
        })
    ));
}

const TARGETED_HOOK_MANIFEST: &str = r#"resolver = 1
identifier = "rtk-ai.rtk"
kind = "hook"
version = "0.1.0"
description = "RTK command rewrite hook"
homepage = "https://github.com/rtk-ai/rtk"
license = "Apache-2.0"

[[targets]]
target = "x86_64-pc-windows-msvc"
url = "https://example.com/rtk.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"
"#;

/// A hook kind manifest is parsed and carries its targeted release source.
#[test]
fn parses_targeted_hook_manifest() {
    let manifest = success(
        PluginManifest::parse(TARGETED_HOOK_MANIFEST),
        "targeted hook manifest",
    );

    assert_eq!(manifest.kind(), PluginKind::Hook);
    assert!(manifest.workbench().is_none());
    assert!(manifest.webview().is_none());
    let Some(PluginReleaseSource::Targets(targets)) = manifest.release_source() else {
        panic!("expected a targeted release source");
    };
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].target().as_str(), "x86_64-pc-windows-msvc");
    assert_eq!(targets[0].url().as_str(), "https://example.com/rtk.orax");
    assert_eq!(manifest.artifact(), None);
}

/// Unknown rustc triples never enter the domain model, so a packaging typo cannot look like a
/// distinct installable architecture.
#[test]
fn rejects_unsupported_target_triples() {
    let source = TARGETED_HOOK_MANIFEST.replace("x86_64-pc-windows-msvc", "not-a-real-triple");
    let Err(ManifestError::InvalidField { field, reason }) = PluginManifest::parse(&source) else {
        panic!("expected unsupported triple rejection");
    };
    assert_eq!(field, ManifestField::ReleaseTargetTarget { index: 0 });
    assert!(matches!(
        reason,
        InvalidFieldReason::InvalidHookTarget(HookTargetError::Unsupported { ref found })
            if found == "not-a-real-triple"
    ));
}

/// A universal hook manifest is parsed and carries a universal release source.
#[test]
fn parses_universal_hook_manifest() {
    let source = r#"resolver = 1
identifier = "rtk-ai.rtk"
kind = "hook"
version = "0.1.0"
description = "RTK command rewrite hook"
url = "https://example.com/rtk.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"
"#;
    let manifest = success(PluginManifest::parse(source), "universal hook manifest");

    assert!(matches!(
        manifest.release_source(),
        Some(PluginReleaseSource::Universal { .. })
    ));
}

/// Declaring both a universal release and targeted artifacts is rejected.
#[test]
fn rejects_both_universal_and_targeted_release_sources() {
    // The universal `url`/`sha256` fields must appear before the `[[targets]]` array; placing them
    // after would attach them to the last target entry instead of the top level.
    let source = r#"resolver = 1
identifier = "rtk-ai.rtk"
kind = "hook"
version = "0.1.0"
description = "RTK command rewrite hook"
url = "https://example.com/rtk.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"

[[targets]]
target = "x86_64-pc-windows-msvc"
url = "https://example.com/rtk.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"
"#;
    let Err(ManifestError::InvalidField { field, reason }) = PluginManifest::parse(source) else {
        panic!("expected duplicate release source rejection");
    };
    assert_eq!(field, ManifestField::Targets);
    assert!(matches!(reason, InvalidFieldReason::DuplicateReleaseSource));
}

/// Duplicate target triples within the targeted form are rejected with an index path.
#[test]
fn rejects_duplicate_target_triples() {
    let source = TARGETED_HOOK_MANIFEST.to_owned()
        + "\n[[targets]]\ntarget = \"x86_64-pc-windows-msvc\"\nurl = \"https://example.com/rtk2.orax\"\nsha256 = \"feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a\"\n";
    let Err(ManifestError::InvalidField { field, .. }) = PluginManifest::parse(&source) else {
        panic!("expected duplicate target rejection");
    };
    assert_eq!(field, ManifestField::ReleaseTargetTarget { index: 1 });
}

/// An agent that bundles its CLI declares the targeted form the same way a hook does.
#[test]
fn parses_targeted_release_for_the_agent_kind() {
    let source = r#"resolver = 1
identifier = "weather"
kind = "agent"
version = "1.0.0"
description = "Weather agent"

[[targets]]
target = "x86_64-pc-windows-msvc"
url = "https://example.com/weather.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"
"#;
    let manifest = PluginManifest::parse(source).expect("targeted agent release parses");
    let Some(PluginReleaseSource::Targets(targets)) = manifest.release_source() else {
        panic!("expected the targeted release form");
    };
    assert_eq!(
        targets
            .iter()
            .map(|target| target.target().to_string())
            .collect::<Vec<_>>(),
        vec!["x86_64-pc-windows-msvc".to_string()]
    );
}

/// The targeted form stays closed to kinds that ship no native binary of their own.
#[test]
fn rejects_targeted_release_for_kinds_without_native_binaries() {
    let source = r#"resolver = 1
identifier = "weather"
kind = "workbench"
version = "1.0.0"
description = "Weather workbench"

[[targets]]
target = "x86_64-pc-windows-msvc"
url = "https://example.com/weather.orax"
sha256 = "feab001d7e9ff4ce66011ebd70791de93eb1554d34d3ea44c33d102a25c1be0a"
"#;
    let Err(ManifestError::InvalidField { field, .. }) = PluginManifest::parse(source) else {
        panic!("expected targeted release rejection for a kind with no native binary");
    };
    assert_eq!(field, ManifestField::Targets);
}

/// An installed targeted package carries an artifact section, not a URL or sha256.
#[test]
fn parses_installed_hook_manifest_with_artifact() {
    let source = r#"resolver = 1
identifier = "rtk-ai.rtk"
kind = "hook"
version = "0.1.0"
description = "RTK command rewrite hook"

[artifact]
target = "x86_64-pc-windows-msvc"
"#;
    let manifest = success(
        PluginManifest::parse_installed(source),
        "installed hook manifest",
    );

    assert_eq!(manifest.kind(), PluginKind::Hook);
    assert_eq!(manifest.release_source(), None);
    let Some(artifact) = manifest.artifact() else {
        panic!("expected an installed artifact target");
    };
    assert_eq!(artifact.target().as_str(), "x86_64-pc-windows-msvc");
}

/// A hook manifest rejects workbench and webview sections.
#[test]
fn hook_kind_rejects_workbench_and_webview_sections() {
    let workbench = TARGETED_HOOK_MANIFEST.to_owned() + "\n[workbench]\nmethods = [\"hello\"]\n";
    let Err(ManifestError::InvalidField { field, .. }) = PluginManifest::parse(&workbench) else {
        panic!("expected workbench rejection for hook kind");
    };
    assert_eq!(field, ManifestField::Workbench);

    let webview = TARGETED_HOOK_MANIFEST.to_owned()
        + "\n[webview]\nstart_url = \"https://a.example\"\nallowed_origins = [\"https://a.example\"]\n";
    let Err(ManifestError::InvalidField { field, .. }) = PluginManifest::parse(&webview) else {
        panic!("expected webview rejection for hook kind");
    };
    assert_eq!(field, ManifestField::Webview);
}

/// A release manifest cannot carry the installed `[artifact]` self-declaration.
#[test]
fn rejects_artifact_section_on_release_manifest() {
    let source =
        TARGETED_HOOK_MANIFEST.to_owned() + "\n[artifact]\ntarget = \"x86_64-pc-windows-msvc\"\n";
    let Err(ManifestError::InvalidField { field, reason }) = PluginManifest::parse(&source) else {
        panic!("expected artifact rejection on release form");
    };
    assert_eq!(field, ManifestField::Artifact);
    assert!(matches!(
        reason,
        InvalidFieldReason::ArtifactNotAllowedOnRelease
    ));
}

/// An installed manifest cannot carry the marketplace `[[targets]]` download list.
#[test]
fn rejects_targets_section_on_installed_manifest() {
    let Err(ManifestError::InvalidField { field, reason }) =
        PluginManifest::parse_installed(TARGETED_HOOK_MANIFEST)
    else {
        panic!("expected targets rejection on installed form");
    };
    assert_eq!(field, ManifestField::Targets);
    assert!(matches!(
        reason,
        InvalidFieldReason::TargetsNotAllowedOnInstalled
    ));
}

/// An installed agent package that bundles its CLI self-declares the target it was built for.
#[test]
fn parses_installed_agent_manifest_with_artifact() {
    let source = r#"resolver = 1
identifier = "weather"
kind = "agent"
version = "1.0.0"
description = "Weather agent"

[artifact]
target = "x86_64-pc-windows-msvc"
"#;
    let manifest = PluginManifest::parse_installed(source).expect("installed agent parses");
    assert_eq!(
        manifest
            .artifact()
            .map(|artifact| artifact.target().to_string()),
        Some("x86_64-pc-windows-msvc".to_string())
    );
}

/// An agent that resolves its CLI from PATH bundles nothing, so it declares no target either.
#[test]
fn parses_installed_agent_manifest_without_artifact() {
    let source = r#"resolver = 1
identifier = "weather"
kind = "agent"
version = "1.0.0"
description = "Weather agent"
"#;
    let manifest = PluginManifest::parse_installed(source).expect("installed agent parses");
    assert_eq!(manifest.artifact(), None);
}

/// `[artifact]` stays closed to kinds that ship no native binary of their own.
#[test]
fn rejects_artifact_section_for_kinds_without_native_binaries() {
    let source = r#"resolver = 1
identifier = "weather"
kind = "workbench"
version = "1.0.0"
description = "Weather workbench"

[artifact]
target = "x86_64-pc-windows-msvc"
"#;
    let Err(ManifestError::InvalidField { field, reason }) =
        PluginManifest::parse_installed(source)
    else {
        panic!("expected artifact rejection for a kind with no native binary");
    };
    assert_eq!(field, ManifestField::Artifact);
    assert!(matches!(
        reason,
        InvalidFieldReason::NotAllowedForKind {
            kind: PluginKind::Workbench
        }
    ));
}

/// An installed hook package must self-declare `[artifact]`.
#[test]
fn rejects_installed_hook_without_artifact() {
    let source = r#"resolver = 1
identifier = "rtk-ai.rtk"
kind = "hook"
version = "0.1.0"
description = "RTK command rewrite hook"
"#;
    let Err(ManifestError::InvalidField { field, reason }) =
        PluginManifest::parse_installed(source)
    else {
        panic!("expected missing artifact rejection");
    };
    assert_eq!(field, ManifestField::Artifact);
    assert!(matches!(
        reason,
        InvalidFieldReason::MissingForKind {
            kind: PluginKind::Hook
        }
    ));
}
