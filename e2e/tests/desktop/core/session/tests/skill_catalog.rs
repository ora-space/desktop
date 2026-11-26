//! End-to-end verification of desktop storage-path Skill catalog delivery and legacy Effect compat.
//!
//! Covers `specs/test-cases/desktop/core/agent/session/skill/`:
//! - `catalog-delivery.md` (first-prompt full `<available_skills>`, `<available_skills_update>` deltas,
//!   leading slash ordering, post-stop full resend, hidden host block in user history, untouched
//!   Workspace tree)
//! - `session-setup-and-hosts.md` (`sessionCapabilities.additionalDirectories`, `_meta["ora/skillDirectories"]`,
//!   and ignoring legacy `effectResources` / never calling `effect/*`)

use super::{
    agent_ref, collect_load, current_thread_runtime, drain_prompt, install_fake_opencode_plugin,
    install_skill_plugin, main_workspace_id, open_ready_backend,
};
use crate::setup::DesktopTestSetup;
use agent_client_protocol_schema::v1::{ContentBlock, SessionUpdate, TextContent};
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const PROMPTS_JOURNAL: &str = "acp_prompts.jsonl";
const SETUPS_JOURNAL: &str = "acp_setups.jsonl";
const LEGACY_EFFECT_MARKER: &str = "register_legacy_effect_resources";
const EFFECT_CALLS_JOURNAL: &str = "effect_calls.txt";
const ADDITIONAL_DIRS_MARKER: &str = "advertise_additional_directories";

fn run_case(test: impl std::future::Future<Output = TestResult>) -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?
            .block_on(async { tokio::time::timeout(Duration::from_secs(20), test).await? })
    })
}

fn read_jsonl(path: &Path) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string(path)?;
    raw.lines()
        .map(|line| serde_json::from_str::<Value>(line).map_err(Into::into))
        .collect()
}

fn workspace_entries(workspace: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut names = fs::read_dir(workspace)?
        .map(|entry| entry.map(|item| item.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn user_messages(updates: &[SessionUpdate]) -> Vec<String> {
    updates
        .iter()
        .filter_map(|update| match update {
            SessionUpdate::UserMessageChunk(chunk) => match &chunk.content {
                ContentBlock::Text(text) => Some(text.text.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn text_prompt(session_id: &str, text: &str) -> PromptSessionRequest {
    PromptSessionRequest {
        session_id: session_id.to_string(),
        prompt: vec![ContentBlock::Text(TextContent::new(text))],
        record_prompt: None,
        model: None,
    }
}

/// Covers `specs/test-cases/desktop/core/agent/session/skill/catalog-delivery.md` &
/// `session-setup-and-hosts.md` end-to-end on the desktop host and fake ACP agent.
#[test]
fn desktop_session_delivers_storage_path_catalog_deltas_and_leaves_workspace_untouched()
-> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let agent_root = install_fake_opencode_plugin(home)?;
        fs::write(agent_root.join(ADDITIONAL_DIRS_MARKER), "")?;
        let plugin_pkg_root = install_skill_plugin(home, "review-pack", "plugin-review")?;

        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        fs::write(workspace.join("README.md"), "# User repo\n")?;

        let backend = open_ready_backend(&setup)?;
        backend.projects().create(CreateProjectRequest {
            name: "Skill Catalog E2E".to_string(),
            main_workspace_path: normalized(&workspace),
        })?;
        let workspace_id = main_workspace_id(&backend)?;

        let local_skill = backend
            .skills()
            .create(CreateSkillRequest {
                name: "local-check".to_string(),
                description: "Local checklist".to_string(),
                content: Some("local check instructions".to_string()),
            })?
            .skill;

        let local_root = setup
            .backend_paths()
            .app_data_directory
            .join("atoms")
            .join("skills");
        let plugins_root = home.join("plugins").join("installed");
        let local_skill_md = local_root.join("local-check").join("SKILL.md");
        let plugin_skill_dir = plugin_pkg_root.join("assets").join("plugin-review");
        let plugin_skill_md = plugin_skill_dir.join("SKILL.md");

        let session_id = backend
            .sessions()
            .start(StartSessionRequest {
                workspace_id,
                agent_ref: agent_ref(),
                model: None,
            })
            .await?
            .session
            .id;

        // Turn 1: full catalog + clean Workspace + readable SKILL.md on disk.
        drain_prompt(backend.sessions().prompt(text_prompt(&session_id, "hello"))).await?;
        assert_eq!(workspace_entries(&workspace)?, vec!["README.md"]);

        let setups = read_jsonl(&agent_root.join(SETUPS_JOURNAL))?;
        assert_eq!(setups[0]["method"], "session/new");
        let setup_dirs: Vec<PathBuf> =
            serde_json::from_value(setups[0]["additionalDirectories"].clone())?;
        assert_eq!(setup_dirs, vec![local_root.clone(), plugins_root.clone()]);
        let meta_dirs: Vec<PathBuf> =
            serde_json::from_value(setups[0]["skillDirectories"].clone())?;
        assert!(meta_dirs.contains(&local_root.join("local-check")));
        assert!(meta_dirs.contains(&plugin_skill_dir));

        let prompts = read_jsonl(&agent_root.join(PROMPTS_JOURNAL))?;
        let p1 = prompts[0]["prompt"].as_str().ok_or("missing prompt 1")?;
        assert!(p1.starts_with("<available_skills>"), "prompt 1: {p1}");
        assert!(p1.contains(&normalized(&local_skill_md)));
        assert!(p1.contains(&normalized(&plugin_skill_md)));
        assert!(p1.ends_with("\nhello"));
        assert!(fs::read_to_string(&local_skill_md)?.contains("local check instructions"));
        assert!(fs::read_to_string(&plugin_skill_md)?.contains("Follow the review checklist."));

        let replayed = collect_load(backend.sessions().load(LoadSessionRequest {
            session_id: session_id.clone(),
        }))
        .await?;
        assert_eq!(user_messages(&replayed), vec!["hello".to_string()]);

        // Turn 2: unchanged catalog injects nothing.
        drain_prompt(
            backend
                .sessions()
                .prompt(text_prompt(&session_id, "unchanged")),
        )
        .await?;
        let prompts = read_jsonl(&agent_root.join(PROMPTS_JOURNAL))?;
        assert_eq!(prompts[1]["prompt"], "unchanged");

        // Turn 3: mid-session skill addition + leading slash command ordering.
        backend.skills().create(CreateSkillRequest {
            name: "slash-helper".to_string(),
            description: "Slash helper".to_string(),
            content: Some("slash helper instructions".to_string()),
        })?;
        let slash_skill_md = local_root.join("slash-helper").join("SKILL.md");
        drain_prompt(
            backend
                .sessions()
                .prompt(text_prompt(&session_id, "/slash-helper run")),
        )
        .await?;
        let prompts = read_jsonl(&agent_root.join(PROMPTS_JOURNAL))?;
        let p3 = prompts[2]["prompt"].as_str().ok_or("missing prompt 3")?;
        assert!(
            p3.starts_with("/slash-helper run\n<available_skills_update>"),
            "prompt 3: {p3}"
        );
        assert!(p3.contains(&normalized(&slash_skill_md)));
        assert!(!p3.contains("local-check"));

        // Turn 4: mid-session skill deletion produces removal delta without touching Workspace.
        backend.skills().delete(DeleteSkillRequest {
            skill_id: local_skill.id,
        })?;
        drain_prompt(
            backend
                .sessions()
                .prompt(text_prompt(&session_id, "after delete")),
        )
        .await?;
        let prompts = read_jsonl(&agent_root.join(PROMPTS_JOURNAL))?;
        let p4 = prompts[3]["prompt"].as_str().ok_or("missing prompt 4")?;
        assert!(p4.contains("<available_skills_update>"), "prompt 4: {p4}");
        assert!(p4.contains(&normalized(&local_skill_md)));
        assert_eq!(workspace_entries(&workspace)?, vec!["README.md"]);

        // Turn 5: stopping drops the actor; reopening resends full current catalog & load roots.
        backend
            .sessions()
            .stop(StopSessionRequest {
                session_id: session_id.clone(),
            })
            .await?;
        drain_prompt(
            backend
                .sessions()
                .prompt(text_prompt(&session_id, "after reopen")),
        )
        .await?;
        let setups = read_jsonl(&agent_root.join(SETUPS_JOURNAL))?;
        assert_eq!(
            setups.last().and_then(|v| v["method"].as_str()),
            Some("session/load")
        );
        let prompts = read_jsonl(&agent_root.join(PROMPTS_JOURNAL))?;
        let p5 = prompts[4]["prompt"].as_str().ok_or("missing prompt 5")?;
        assert!(p5.starts_with("<available_skills>"), "prompt 5: {p5}");
        assert!(p5.contains(&normalized(&plugin_skill_md)));
        assert!(p5.contains(&normalized(&slash_skill_md)));
        assert!(!p5.contains(&normalized(&local_skill_md)));
        assert_eq!(workspace_entries(&workspace)?, vec!["README.md"]);
        Ok(())
    })
}

/// Covers `specs/test-cases/desktop/core/agent/session/skill/session-setup-and-hosts.md`:
/// legacy `effectResources` in `ora/register` is ignored, `effect/*` is never called, and
/// Workspace stays clean while prompts receive `<available_skills>`.
#[test]
fn desktop_ignores_legacy_effect_resources_registration_without_failing_or_calling_effects()
-> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let agent_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        fs::write(agent_root.join(LEGACY_EFFECT_MARKER), "")?;

        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        fs::write(workspace.join("README.md"), "# Compat check\n")?;

        let backend = open_ready_backend(&setup)?;
        backend.projects().create(CreateProjectRequest {
            name: "Legacy Compat E2E".to_string(),
            main_workspace_path: normalized(&workspace),
        })?;
        let workspace_id = main_workspace_id(&backend)?;

        let created = backend
            .skills()
            .create(CreateSkillRequest {
                name: "compat-skill".to_string(),
                description: "Compat check".to_string(),
                content: Some("compat body".to_string()),
            })?
            .skill;

        let session_id = backend
            .sessions()
            .start(StartSessionRequest {
                workspace_id,
                agent_ref: agent_ref(),
                model: None,
            })
            .await?
            .session
            .id;
        drain_prompt(
            backend
                .sessions()
                .prompt(text_prompt(&session_id, "use compat")),
        )
        .await?;
        backend.skills().delete(DeleteSkillRequest {
            skill_id: created.id,
        })?;
        drain_prompt(
            backend
                .sessions()
                .prompt(text_prompt(&session_id, "after compat delete")),
        )
        .await?;

        let prompts = read_jsonl(&agent_root.join(PROMPTS_JOURNAL))?;
        assert!(
            prompts[0]["prompt"]
                .as_str()
                .is_some_and(|p| p.contains("<available_skills>"))
        );
        assert!(!agent_root.join(EFFECT_CALLS_JOURNAL).exists());
        assert_eq!(workspace_entries(&workspace)?, vec!["README.md"]);
        Ok(())
    })
}
