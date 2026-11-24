//! Unit tests for Skill catalog rendering, path-based deltas, placement, and ACP setup fields.
//!
//! Covers obligations in `specs/test-cases/desktop/core/agent/session/skill/catalog-delivery.md`
//! and `specs/test-cases/desktop/core/agent/session/skill/session-setup-and-hosts.md`.

use super::{
    AnnouncedSkills, SKILL_DIRECTORIES_META_KEY, apply_skill_setup_to_load_session,
    apply_skill_setup_to_new_session, insert_catalog_block, prepare_skill_catalog_injection,
    skill_directories_meta,
};
use crate::host::SkillCatalogEntry;
use agent_client_protocol_schema::v1::{
    ContentBlock, ImageContent, LoadSessionRequest, NewSessionRequest, TextContent,
};
use pretty_assertions::assert_eq;
use std::path::PathBuf;

fn entry(name: &str, description: &str, skill_md: &str, package_dir: &str) -> SkillCatalogEntry {
    SkillCatalogEntry {
        name: name.to_string(),
        description: description.to_string(),
        skill_md_path: PathBuf::from(skill_md),
        package_dir: PathBuf::from(package_dir),
    }
}

/// First prompt lists all available entries sorted by ASCII-lowercased name then path, keeping
/// duplicate names separate by path and preserving unescaped spaces / non-ASCII paths
/// (`catalog-delivery.md#first-prompt-must-carry-the-full-catalog-of-available-skills`).
#[test]
fn full_catalog_sorts_deterministically_keeps_duplicate_names_and_preserves_native_paths() {
    let entries = vec![
        entry(
            "zeta",
            "Last alphabetically.",
            "/home/user name/技能目录/zeta/SKILL.md",
            "/home/user name/技能目录/zeta",
        ),
        entry(
            "Review",
            "Security review.",
            "/opt/ora/plugins/installed/acme/sec/1.0.0/assets/review/SKILL.md",
            "/opt/ora/plugins/installed/acme/sec/1.0.0/assets/review",
        ),
        entry(
            "review",
            "Checklist\n  review.",
            "/home/user name/atoms/skills/review/SKILL.md",
            "/home/user name/atoms/skills/review",
        ),
    ];

    let injection = prepare_skill_catalog_injection(/*announced*/ None, &entries)
        .expect("non-empty catalog must inject");

    assert_eq!(
        injection.block_text,
        "<available_skills>\n\
Available skills are listed below. When a task matches a skill's description, read its SKILL.md first and follow its instructions. Relative paths inside SKILL.md resolve against its directory.\n\
- review: Checklist review.\n  /home/user name/atoms/skills/review/SKILL.md\n\
- Review: Security review.\n  /opt/ora/plugins/installed/acme/sec/1.0.0/assets/review/SKILL.md\n\
- zeta: Last alphabetically.\n  /home/user name/技能目录/zeta/SKILL.md\n\
</available_skills>"
    );
}

/// Empty catalog emits no prompt block and no `_meta` field
/// (`catalog-delivery.md#first-prompt-must-carry-the-full-catalog-of-available-skills`).
#[test]
fn empty_catalog_emits_no_block_and_no_meta() {
    assert_eq!(
        prepare_skill_catalog_injection(/*announced*/ None, &[]),
        None
    );
    assert_eq!(skill_directories_meta(&[]), None);
}

/// Unchanged paths emit nothing even if name or description text changed; additions, removals,
/// and upgrades emit only deltas (`catalog-delivery.md#later-prompts-must-carry-only-path-deltas`).
#[test]
fn delta_tracks_path_identity_across_unchanged_add_remove_and_upgrade() {
    let local = entry(
        "local-lint",
        "Original description.",
        "/data/atoms/skills/local-lint/SKILL.md",
        "/data/atoms/skills/local-lint",
    );
    let plugin_v1 = entry(
        "sec-audit",
        "Audit v1.",
        "/home/plugins/installed/acme.1/sec/1.0.0/assets/sec-audit/SKILL.md",
        "/home/plugins/installed/acme.1/sec/1.0.0/assets/sec-audit",
    );
    let initial =
        prepare_skill_catalog_injection(/*announced*/ None, &[local.clone(), plugin_v1])
            .expect("initial full");
    let announced = initial.next_announced;

    let same_path_updated = entry(
        "local-lint-renamed-meta",
        "Updated description.",
        "/data/atoms/skills/local-lint/SKILL.md",
        "/data/atoms/skills/local-lint",
    );
    let plugin_v1_same = entry(
        "sec-audit",
        "Audit v1.",
        "/home/plugins/installed/acme.1/sec/1.0.0/assets/sec-audit/SKILL.md",
        "/home/plugins/installed/acme.1/sec/1.0.0/assets/sec-audit",
    );
    assert_eq!(
        prepare_skill_catalog_injection(Some(&announced), &[same_path_updated, plugin_v1_same]),
        None
    );

    // Upgrade plugin 1.0.0 -> 1.1.0 and install a second local skill.
    let plugin_v2 = entry(
        "sec-audit",
        "Audit v2.",
        "/home/plugins/installed/acme.1/sec/1.1.0/assets/sec-audit/SKILL.md",
        "/home/plugins/installed/acme.1/sec/1.1.0/assets/sec-audit",
    );
    let added = entry(
        "deploy",
        "Deploy helper.",
        "/data/atoms/skills/deploy/SKILL.md",
        "/data/atoms/skills/deploy",
    );
    let delta = prepare_skill_catalog_injection(
        Some(&announced),
        &[local, plugin_v2.clone(), added.clone()],
    )
    .expect("upgrade + add must emit delta");
    assert_eq!(
        delta.block_text,
        "<available_skills_update>\n\
Available skills changed during this session.\n\
Added skills (read a skill's SKILL.md before using it; relative paths inside SKILL.md resolve against its directory):\n\
- deploy: Deploy helper.\n  /data/atoms/skills/deploy/SKILL.md\n\
- sec-audit: Audit v2.\n  /home/plugins/installed/acme.1/sec/1.1.0/assets/sec-audit/SKILL.md\n\
Removed skills (no longer available; do not use):\n\
- sec-audit\n  /home/plugins/installed/acme.1/sec/1.0.0/assets/sec-audit/SKILL.md\n\
</available_skills_update>"
    );

    // Uninstalling everything still emits removals and resets cumulative paths to empty.
    let cleared = prepare_skill_catalog_injection(Some(&delta.next_announced), &[])
        .expect("removing all skills emits removal delta");
    assert!(cleared.block_text.contains("Removed skills"));
    assert!(!cleared.block_text.contains("Added skills"));
    assert_eq!(
        prepare_skill_catalog_injection(Some(&cleared.next_announced), &[]),
        None
    );

    // Explicitly verify forgetting announced state yields full catalog on next turn.
    let reset: Option<&AnnouncedSkills> = None;
    let resent = prepare_skill_catalog_injection(reset, &[plugin_v2, added])
        .expect("reset resends full catalog");
    assert!(resent.block_text.starts_with("<available_skills>\n"));
}

/// Leading slash commands stay first while ordinary prompts receive the block before user content
/// (`catalog-delivery.md#catalog-blocks-must-follow-a-leading-slash-command-and-stay-out-of-user-input`).
#[test]
fn catalog_block_follows_leading_slash_command_and_precedes_ordinary_prompt() {
    let ordinary = insert_catalog_block(
        vec![ContentBlock::Text(TextContent::new("fix the tests"))],
        "<available_skills>catalog</available_skills>".to_string(),
        /*user_start_index*/ 0,
    );
    assert_eq!(
        ordinary,
        vec![
            ContentBlock::Text(TextContent::new(
                "<available_skills>catalog</available_skills>"
            )),
            ContentBlock::Text(TextContent::new("fix the tests")),
        ]
    );

    let slash = insert_catalog_block(
        vec![
            ContentBlock::Text(TextContent::new("/review src/lib.rs")),
            ContentBlock::Text(TextContent::new("extra context")),
        ],
        "<available_skills>catalog</available_skills>".to_string(),
        /*user_start_index*/ 0,
    );
    assert_eq!(
        slash,
        vec![
            ContentBlock::Text(TextContent::new("/review src/lib.rs")),
            ContentBlock::Text(TextContent::new(
                "<available_skills>catalog</available_skills>"
            )),
            ContentBlock::Text(TextContent::new("extra context")),
        ]
    );

    let with_handoff = insert_catalog_block(
        vec![
            ContentBlock::Text(TextContent::new("# Transcript")),
            ContentBlock::Image(ImageContent::new("img", "image/png")),
            ContentBlock::Text(TextContent::new("/deploy")),
        ],
        "<available_skills_update>delta</available_skills_update>".to_string(),
        /*user_start_index*/ 1,
    );
    assert_eq!(
        with_handoff,
        vec![
            ContentBlock::Text(TextContent::new("# Transcript")),
            ContentBlock::Image(ImageContent::new("img", "image/png")),
            ContentBlock::Text(TextContent::new("/deploy")),
            ContentBlock::Text(TextContent::new(
                "<available_skills_update>delta</available_skills_update>"
            )),
        ]
    );
}

/// ACP `session/new` and `session/load` include `additionalDirectories` only when the agent
/// declared support, and omit `_meta` when the catalog is empty
/// (`session-setup-and-hosts.md#readable-roots-must-follow-the-declared-capability-without-failing-setup`).
#[test]
fn setup_requests_follow_capability_negotiation_and_preserve_empty_wire_shape() {
    let roots = vec![
        PathBuf::from("/app-data/atoms/skills"),
        PathBuf::from("/ora-home/plugins/installed"),
    ];
    let entries = vec![entry(
        "review",
        "Review changes.",
        "/app-data/atoms/skills/review/SKILL.md",
        "/app-data/atoms/skills/review",
    )];

    let with_cap_new = apply_skill_setup_to_new_session(
        NewSessionRequest::new("/workspace"),
        /*additional_directories_supported*/ true,
        roots.clone(),
        &entries,
    );
    assert_eq!(with_cap_new.additional_directories, roots);
    assert_eq!(
        with_cap_new
            .meta
            .as_ref()
            .and_then(|meta| meta.get(SKILL_DIRECTORIES_META_KEY)),
        Some(&serde_json::json!(["/app-data/atoms/skills/review"]))
    );

    let without_cap_load = apply_skill_setup_to_load_session(
        LoadSessionRequest::new("provider-1", "/workspace"),
        /*additional_directories_supported*/ false,
        roots,
        &entries,
    );
    assert!(without_cap_load.additional_directories.is_empty());
    assert!(without_cap_load.meta.is_some());

    // Empty host (Node) produces the exact same JSON as bare NewSessionRequest / LoadSessionRequest.
    let node_new = apply_skill_setup_to_new_session(
        NewSessionRequest::new("/workspace"),
        /*additional_directories_supported*/ true,
        Vec::new(),
        &[],
    );
    assert_eq!(
        serde_json::to_value(&node_new).unwrap(),
        serde_json::to_value(NewSessionRequest::new("/workspace")).unwrap()
    );
    let node_load = apply_skill_setup_to_load_session(
        LoadSessionRequest::new("provider-1", "/workspace"),
        /*additional_directories_supported*/ true,
        Vec::new(),
        &[],
    );
    assert_eq!(
        serde_json::to_value(&node_load).unwrap(),
        serde_json::to_value(LoadSessionRequest::new("provider-1", "/workspace")).unwrap()
    );
}
