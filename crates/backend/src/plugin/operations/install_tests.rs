//! Covers Hook and pack install outcomes after the host dropped plugin enablement.

use super::Plugins;
use crate::agent_runtime::{AgentRuntimeSetup, open_agent_runtime};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::error::ErrorClassification;
use crate::plugin::PluginApi;
use crate::plugin::hook_lifecycle::{
    HookCommandError, HookCommandExecution, HookCommandFinished, HookCommandOutput,
    HookCommandRunner, HookCommandSpec,
};
use crate::plugin::pack_reconcile::PackMemberReconciliation;
use crate::settings::Settings;
use ora_contracts::{
    HookLifecycleOutcome, HookLifecyclePhase, ImportPluginRequest, ImportedWorkflowOutcome,
    InitializeHookRequest, InstallOutcome, InstallPluginRequest, ListHookLifecycleReportsRequest,
    ListInstalledPluginsRequest, ListPackInstallationsRequest, MarketplaceWorkflowOutcome,
    PackInstallFailure, PackUninstallPlanRequest, PluginDataDisposition,
    PluginPackageInvalidParams, PublicError, RetryWorkflowImportRequest, UninstallPluginRequest,
    UpdatePluginRequest,
};
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqlitePackInstallationRepository,
    default_migration_catalog,
};
use ora_logging::with_trace_logging;
use ora_plugin_manager::{Installer, ResolvedReleaseSource};
use ora_plugin_manifest::{PluginKind, PluginManifest};
use ora_plugin_registry::RegistrySource;
use ora_scheduler::Scheduler;
use pretty_assertions::assert_eq;
use std::collections::VecDeque;
use std::fs;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(windows)]
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const SOURCE_URL: &str = "https://github.com/ora-space/marketplace";
const PACK_ID: &str = "official/ora-space.python-extension-pack";
const PACK_IDENTIFIER: &str = "ora-space.python-extension-pack";

/// Opens a throwaway SQLite pool under `root` for PluginApi tests.
fn test_pool(root: &Path) -> RepositoryPool {
    ora_logging::initialize_test_clock();
    DatabaseBootstrapper::new(crate::test_clock::TestClock)
        .bootstrap_repository_pool(
            &DatabaseLocation::path(root.join("test.sqlite")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("create repository pool")
}

/// Exercises the public plugin interface with its real host and shared runtime coordinator.
fn test_plugin_api(root: &Path, pool: &RepositoryPool) -> Plugins {
    let events = AppEventHub::new();
    let host = Arc::new(
        PluginApi::open(
            pool.clone(),
            root.to_path_buf(),
            std::path::PathBuf::from("deno"),
            SystemClock,
            events.publisher(),
            Arc::new(Settings::new(pool.clone())),
        )
        .expect("open plugin host"),
    );
    let runtime = Arc::new(
        open_agent_runtime(AgentRuntimeSetup {
            plugin_host: host.clone(),
            pool: pool.clone(),
            home_directory: root.to_path_buf(),
            relative_path_base: root.to_path_buf(),
            sessions_root: root.join("sessions"),
            scheduler: Scheduler::new(chrono_tz::Asia::Shanghai),
            app_events: events.publisher(),
        })
        .expect("agent runtime"),
    );
    Plugins::new(
        host,
        runtime,
        Arc::new(crate::workflow::workflow_import(pool.clone(), SystemClock)),
        Arc::new(crate::workflow::marketplace_workflow_import(
            pool.clone(),
            SystemClock,
        )),
    )
}

/// Writes a Hook `.orax` that ships `assets/rtk.exe` and declares both lifecycle commands.
///
/// A local import refuses a Hook package that declares no artifact target, so `target` is that
/// path's requirement; a marketplace release is universal and the package carries none. Returns
/// the artifact's lowercase hex SHA-256 so a listing can declare it.
fn write_hook_orax(path: &Path, identifier: &str, target: Option<&str>) -> String {
    let artifact = target
        .map(|target| format!("\n[artifact]\ntarget = \"{target}\"\n"))
        .unwrap_or_default();
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"Hook command rewrite\"\n{artifact}"
    );
    let config = br#"{"schemaVersion":1,"hook":{"executable":"assets/rtk.exe","lifecycle":{"init":{"args":["--init"]},"deinit":{"args":["--deinit"]}}}}"#;
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default();
    writer.start_file("orax.toml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    writer.start_file("assets/config.json", options).unwrap();
    writer.write_all(config).unwrap();
    writer.start_file("assets/rtk.exe", options).unwrap();
    writer.write_all(b"MZdummy").unwrap();
    writer.finish().unwrap();
    ora_utils::hash::sha256_file(path).expect("hash the Hook artifact")
}

/// Builds one Hook listing with a universal release, so the package installs on any host.
fn hook_listing(identifier: &str, sha256: &str) -> String {
    format!(
        "resolver = 1\nidentifier = \"{identifier}\"\ntitle = \"RTK\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"Hook command rewrite\"\nurl = \"https://example.com/{identifier}.orax\"\nsha256 = \"{sha256}\"\n"
    )
}

/// Marketplace README reads resolve from the source checkout beside the listing's manifest.
#[test]
fn read_plugin_readme_resolves_from_the_marketplace_checkout() {
    with_trace_logging(|| {
        let data_dir = TempDir::new().expect("data dir");
        let pool = test_pool(data_dir.path());
        let api = test_plugin_api(data_dir.path(), &pool);
        let checkout = data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace");
        let digest = "ab".repeat(32);

        let listing_dir = checkout
            .join("registry")
            .join("o")
            .join("ora-space.weather");
        std::fs::create_dir_all(&listing_dir).expect("create listing dir");
        std::fs::write(
            listing_dir.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"ora-space.weather\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.2.0\"\ndescription = \"Weather plugin\"\nurl = \"https://example.com/weather.orax\"\nsha256 = \"{digest}\"\n"
            ),
        )
        .expect("write listing manifest");
        std::fs::write(
            listing_dir.join("README.md"),
            "# Weather\n\nLive forecasts.",
        )
        .expect("write listing README");

        let response = api
            .read_readme(ora_contracts::ReadPluginReadmeRequest {
                plugin_id: "official/ora-space.weather".to_string(),
            })
            .expect("read readme");
        assert_eq!(
            response.readme.as_deref(),
            Some("# Weather\n\nLive forecasts.")
        );

        // A listing without a README reports no documentation; an unknown id reports NotFound.
        let silent_dir = checkout.join("registry").join("s").join("ora-space.silent");
        std::fs::create_dir_all(&silent_dir).expect("create silent listing dir");
        std::fs::write(
            silent_dir.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"ora-space.silent\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Silent plugin\"\nurl = \"https://example.com/silent.orax\"\nsha256 = \"{digest}\"\n"
            ),
        )
        .expect("write silent manifest");

        let silent = api
            .read_readme(ora_contracts::ReadPluginReadmeRequest {
                plugin_id: "official/ora-space.silent".to_string(),
            })
            .expect("read silent readme");
        assert_eq!(silent.readme, None);

        let unknown = api
            .read_readme(ora_contracts::ReadPluginReadmeRequest {
                plugin_id: "official/absent".to_string(),
            })
            .expect_err("unknown id");
        assert_eq!(
            unknown.to_string(),
            "marketplace plugin was not found in the registry"
        );
    });
}

/// Opens the plugin host directly so pack orchestration can inject a local-file installer.
fn pack_test_host(root: &Path, pool: &RepositoryPool) -> Arc<PluginApi> {
    Arc::new(
        PluginApi::open(
            pool.clone(),
            root.to_path_buf(),
            std::path::PathBuf::from("deno"),
            SystemClock,
            AppEventHub::new().publisher(),
            Arc::new(Settings::new(pool.clone())),
        )
        .expect("open plugin host"),
    )
}

/// Installs a tracing dispatcher for the duration of one async pack test.
fn trace_guard() -> tracing::dispatcher::DefaultGuard {
    use tracing_subscriber::layer::SubscriberExt;
    let subscriber =
        tracing_subscriber::registry().with(tracing_subscriber::filter::LevelFilter::TRACE);
    tracing::dispatcher::set_default(&tracing::Dispatch::new(subscriber))
}

/// Writes one skill-member release archive and returns its lowercase hex SHA-256.
fn write_skill_orax(path: &Path, identifier: &str) -> String {
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Pack member skill\"\n"
    );
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default();
    writer.start_file("orax.toml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    writer.start_file("assets/demo/SKILL.md", options).unwrap();
    writer
        .write_all(b"---\nname: demo\ndescription: Pack member skill\n---\n\nBody.\n")
        .unwrap();
    writer.finish().unwrap();
    ora_utils::hash::sha256_file(path).expect("hash member artifact")
}

/// Writes one marketplace listing directory into the staged checkout.
fn stage_listing(root: &Path, identifier: &str, listing: &str) {
    let listing_dir = root
        .join("registry")
        .join(&identifier[0..1])
        .join(identifier);
    std::fs::create_dir_all(&listing_dir).expect("create listing dir");
    std::fs::write(listing_dir.join("orax.toml"), listing).expect("write listing manifest");
}

/// Builds one skill-member listing whose release digest matches the staged artifact.
fn skill_listing(identifier: &str, sha256: &str, marketplace_visible: bool) -> String {
    let visibility = if marketplace_visible {
        String::new()
    } else {
        "marketplace_visible = false\n".to_string()
    };
    format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Pack member skill\"\n{visibility}url = \"https://example.com/{identifier}.orax\"\nsha256 = \"{sha256}\"\n"
    )
}

/// Builds one pack listing with the supplied `[[pack.members]]` tables.
fn pack_listing(identifier: &str, members: &str) -> String {
    format!(
        "resolver = 1\nidentifier = \"{identifier}\"\ntitle = \"Python Extension Pack\"\nkind = \"pack\"\nversion = \"0.1.0\"\ndescription = \"Python development pack\"\n{members}"
    )
}

fn member_table(identifier: &str) -> String {
    format!("[[pack.members]]\nidentifier = \"{identifier}\"\n\n")
}

fn member_table_with_agents(identifier: &str, agents: &str) -> String {
    format!("[[pack.members]]\nidentifier = \"{identifier}\"\nagents = [{agents}]\n\n")
}

/// Substitutes only the transfer leg: each member's HTTPS locator is replaced by its locally
/// built artifact (the same substitution the RTK E2E uses), keeping digest verification on the
/// production path. Members without a staged artifact keep their marketplace release, which
/// fails at download under the local downloader — exactly the failure shape some tests need.
fn with_local_releases(
    fixture: &PackFixture,
    mut preflight: crate::plugin::pack::PackPreflight,
) -> crate::plugin::pack::PackPreflight {
    for member in preflight.applicable_mut() {
        let Some(artifact) = fixture.member_artifacts.get(member.plugin_id.name()) else {
            continue;
        };
        let digest = *ora_plugin_manifest::Sha256Digest::parse(
            &ora_utils::hash::sha256_file(artifact).expect("hash artifact"),
        )
        .expect("digest")
        .as_bytes();
        member.release = ResolvedReleaseSource::universal(
            ora_utils::http::DownloadSource::Local(artifact.clone()),
            digest,
        );
    }
    preflight
}
const HIDDEN_MEMBER: &str = "ora-space.python-core";
const VISIBLE_MEMBER: &str = "ora-space.claude-python-tools";

/// Builds one member release archive at an explicit version and returns its path.
fn build_member_artifact(data_dir: &Path, identifier: &str, version: &str) -> PathBuf {
    let artifacts = data_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts).expect("create artifacts dir");
    let artifact = artifacts.join(format!("{identifier}-v{version}.orax"));
    let mut writer = ZipWriter::new(File::create(&artifact).unwrap());
    let options = SimpleFileOptions::default();
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"skill\"\nversion = \"{version}\"\ndescription = \"Pack member skill\"\n"
    );
    writer.start_file("orax.toml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    writer.start_file("assets/demo/SKILL.md", options).unwrap();
    writer
        .write_all(b"---\nname: demo\ndescription: Pack member skill\n---\n\nBody.\n")
        .unwrap();
    writer.finish().unwrap();
    artifact
}
/// Installs one member package at an explicit version, the way an independent marketplace
/// upgrade would, so reconciliation observes a version that the pack never recorded.
async fn install_member_version(data_dir: &Path, identifier: &str, version: &str) {
    let artifact = build_member_artifact(data_dir, identifier, version);
    let listing = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"skill\"\nversion = \"{version}\"\ndescription = \"Pack member skill\"\nurl = \"https://example.com/{identifier}.orax\"\nsha256 = \"{}\"\n",
        ora_utils::hash::sha256_file(&artifact).expect("hash artifact")
    );
    let release_manifest = PluginManifest::parse(&listing).expect("parse upgrade listing");
    let digest = *ora_plugin_manifest::Sha256Digest::parse(
        &ora_utils::hash::sha256_file(&artifact).expect("hash artifact"),
    )
    .expect("digest")
    .as_bytes();
    Installer::new(ora_utils::http::LocalFileDownloader)
        .install(
            &release_manifest,
            &ora_domain::PluginNamespace::official(),
            ResolvedReleaseSource::universal(
                ora_utils::http::DownloadSource::Local(artifact),
                digest,
            ),
            data_dir,
        )
        .await
        .expect("install the member version");
}

/// Opens the Plugins wrapper together with its host so pack tests can drive both layers.
fn pack_test_plugins(root: &Path, pool: &RepositoryPool) -> (Plugins, Arc<PluginApi>) {
    let events = AppEventHub::new();
    let host = Arc::new(
        PluginApi::open(
            pool.clone(),
            root.to_path_buf(),
            std::path::PathBuf::from("deno"),
            SystemClock,
            events.publisher(),
            Arc::new(Settings::new(pool.clone())),
        )
        .expect("open plugin host"),
    );
    let runtime = Arc::new(
        open_agent_runtime(AgentRuntimeSetup {
            plugin_host: host.clone(),
            pool: pool.clone(),
            home_directory: root.to_path_buf(),
            relative_path_base: root.to_path_buf(),
            sessions_root: root.join("sessions"),
            scheduler: Scheduler::new(chrono_tz::Asia::Shanghai),
            app_events: events.publisher(),
        })
        .expect("agent runtime"),
    );
    (
        Plugins::new(
            host.clone(),
            runtime,
            Arc::new(crate::workflow::workflow_import(pool.clone(), SystemClock)),
            Arc::new(crate::workflow::marketplace_workflow_import(
                pool.clone(),
                SystemClock,
            )),
        ),
        host,
    )
}

/// Asserts the installed state of one member package directory.
fn member_installed(root: &Path, member: &str, version: &str) -> bool {
    root.join("plugins")
        .join("installed")
        .join("official")
        .join(member)
        .join(version)
        .is_dir()
}

/// Runs the pack orchestration over the standard fixture and records the ownership journal.
async fn install_and_record_pack(
    data_dir: &Path,
    api: &Arc<PluginApi>,
    pack_members: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = stage_pack_fixture(data_dir, pack_members);
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let preflight = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (_outcome, ledger) = api
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("pack members install");
    api.record_pack_run(
        &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
        "https://github.com/ora-space/marketplace",
        &ledger,
    )
    .expect("record pack ownership");
    Ok(())
}

/// Stages the pack fixture: the pack listing, one hidden member, and one visible member, each
/// backed by a real release artifact, and returns the pack manifest plus the member artifacts.
struct PackFixture {
    pack_manifest: ora_plugin_manifest::PluginManifest,
    member_artifacts: std::collections::BTreeMap<&'static str, std::path::PathBuf>,
}

fn stage_pack_fixture(root: &Path, pack_members: &str) -> PackFixture {
    let checkout = root
        .join("plugins")
        .join("sources")
        .join("github.com")
        .join("ora-space")
        .join("marketplace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&artifacts).expect("create artifacts dir");

    let hidden_sha = write_skill_orax(&artifacts.join("python-core.orax"), HIDDEN_MEMBER);
    let visible_sha = write_skill_orax(&artifacts.join("claude-python-tools.orax"), VISIBLE_MEMBER);
    stage_listing(
        &checkout,
        HIDDEN_MEMBER,
        &skill_listing(
            HIDDEN_MEMBER,
            &hidden_sha,
            /*marketplace_visible*/ false,
        ),
    );
    stage_listing(
        &checkout,
        VISIBLE_MEMBER,
        &skill_listing(VISIBLE_MEMBER, &visible_sha, true),
    );
    stage_listing(
        &checkout,
        PACK_IDENTIFIER,
        &pack_listing(PACK_IDENTIFIER, pack_members),
    );

    let pack_listing_path = checkout
        .join("registry")
        .join("o")
        .join("ora-space.python-extension-pack")
        .join("orax.toml");
    let pack_manifest = ora_plugin_manifest::PluginManifest::parse(
        &std::fs::read_to_string(pack_listing_path).expect("read pack listing"),
    )
    .expect("parse pack listing");
    PackFixture {
        pack_manifest,
        member_artifacts: std::collections::BTreeMap::from([
            (HIDDEN_MEMBER, artifacts.join("python-core.orax")),
            (VISIBLE_MEMBER, artifacts.join("claude-python-tools.orax")),
        ]),
    }
}

/// Two fresh members install in declaration order and report a clean pack outcome.
#[tokio::test]
async fn pack_install_installs_every_applicable_member_in_declaration_order() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    );
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        ora_domain::PluginNamespace::official(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );

    let preflight = api
        .preflight_pack(
            &fixture.pack_manifest,
            &ora_domain::PluginNamespace::official(),
            &source,
        )
        .expect("pack preflight succeeds");
    assert_eq!(preflight.already_installed(), Vec::<String>::new());
    assert_eq!(
        preflight
            .applicable()
            .iter()
            .map(|member| member.plugin_id.canonical())
            .collect::<Vec<_>>(),
        vec![
            format!("official/{HIDDEN_MEMBER}"),
            format!("official/{VISIBLE_MEMBER}"),
        ]
    );

    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, ledger) = api
        .install_members(
            &ora_domain::PluginNamespace::official(),
            preflight,
            &installer,
            /*progress*/ None,
        )
        .await
        .expect("pack members install");
    assert_eq!(
        outcome,
        InstallOutcome::PackInstalled {
            members: vec![
                format!("official/{HIDDEN_MEMBER}"),
                format!("official/{VISIBLE_MEMBER}"),
            ],
            skipped: Vec::new(),
            failed: None,
        }
    );
    // The hidden member installs even though discovery never listed it: visibility only affects
    // discovery, never addressability.
    assert!(
        data_dir
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join(HIDDEN_MEMBER)
            .join("1.0.0")
            .is_dir(),
        "the hidden member is installed"
    );

    // The ownership journal records what the run did: both members were created by this run, so
    // both are pack-managed at the version that landed.
    api.record_pack_run(
        &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
        "https://github.com/ora-space/marketplace",
        &ledger,
    )
    .expect("record pack ownership");
    let record = api
        .pack_installation(PACK_ID)
        .expect("load pack installation")
        .expect("the pack installation is recorded");
    assert_eq!(record.pack_id, PACK_ID);
    assert_eq!(
        record.source_url,
        "https://github.com/ora-space/marketplace"
    );
    assert_eq!(
        record.members,
        vec![
            ora_db::PackInstallationMemberRecord {
                member_id: format!("official/{VISIBLE_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::ManagedByPack,
            },
            ora_db::PackInstallationMemberRecord {
                member_id: format!("official/{HIDDEN_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::ManagedByPack,
            },
        ]
    );
}

/// A member that was already installed when the pack named it is recorded as pre-existing, while
/// members the run created are recorded as pack-managed.
#[tokio::test]
async fn pack_install_records_created_members_as_managed_and_skips_as_pre_existing() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    );
    // Pre-install the visible member so the pack run skips it instead of creating it.
    let visible_manifest = PluginManifest::parse(
        &std::fs::read_to_string(
            data_dir
                .path()
                .join("plugins")
                .join("sources")
                .join("github.com")
                .join("ora-space")
                .join("marketplace")
                .join("registry")
                .join("o")
                .join(VISIBLE_MEMBER)
                .join("orax.toml"),
        )
        .expect("read visible listing"),
    )
    .expect("parse visible listing");
    let digest = *ora_plugin_manifest::Sha256Digest::parse(
        &ora_utils::hash::sha256_file(fixture.member_artifacts[VISIBLE_MEMBER].as_path())
            .expect("hash artifact"),
    )
    .expect("digest")
    .as_bytes();
    Installer::new(ora_utils::http::LocalFileDownloader)
        .install(
            &visible_manifest,
            &ora_domain::PluginNamespace::official(),
            ResolvedReleaseSource::universal(
                ora_utils::http::DownloadSource::Local(
                    fixture.member_artifacts[VISIBLE_MEMBER].clone(),
                ),
                digest,
            ),
            data_dir.path(),
        )
        .await
        .expect("pre-install the visible member");

    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let preflight = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (_outcome, ledger) = api
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("pack members install");
    api.record_pack_run(
        &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
        "https://github.com/ora-space/marketplace",
        &ledger,
    )
    .expect("record pack ownership");
    let record = api
        .pack_installation(PACK_ID)
        .expect("load pack installation")
        .expect("the pack installation is recorded");
    assert_eq!(
        record.members,
        vec![
            ora_db::PackInstallationMemberRecord {
                member_id: format!("official/{VISIBLE_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::PreExisting,
            },
            ora_db::PackInstallationMemberRecord {
                member_id: format!("official/{HIDDEN_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::ManagedByPack,
            },
        ]
    );
}

/// The D4 presentation queries project the journal and reconciliation for the frontend: the
/// installed-packs list carries reconciled member states, and the uninstall plan separates
/// removable members from preserved ones with structured reasons.
#[tokio::test]
async fn pack_presentation_queries_project_journal_and_reconciliation() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    install_and_record_pack(
        data_dir.path(),
        &host,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("install and record the pack");

    let installations = plugins
        .list_pack_installations(ListPackInstallationsRequest {})
        .expect("list pack installations")
        .packs;
    assert_eq!(installations.len(), 1);
    let installation = &installations[0];
    assert_eq!(installation.pack_id, PACK_ID);
    assert_eq!(
        installation
            .members
            .iter()
            .map(|member| (
                member.member_id.as_str(),
                member.version_at_install.as_str(),
                member.ownership,
                member.state.clone(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                format!("official/{VISIBLE_MEMBER}").as_str(),
                "1.0.0",
                ora_contracts::PackMemberOwnership::ManagedByPack,
                ora_contracts::PackMemberReconciliationState::ExpectedAndPresent,
            ),
            (
                format!("official/{HIDDEN_MEMBER}").as_str(),
                "1.0.0",
                ora_contracts::PackMemberOwnership::ManagedByPack,
                ora_contracts::PackMemberReconciliationState::ExpectedAndPresent,
            ),
        ]
    );

    let plan = plugins
        .pack_uninstall_plan(PackUninstallPlanRequest {
            plugin_id: PACK_ID.to_string(),
        })
        .expect("compute the uninstall plan")
        .plan
        .expect("the recorded pack plans");
    assert_eq!(
        plan.remove,
        vec![
            format!("official/{VISIBLE_MEMBER}"),
            format!("official/{HIDDEN_MEMBER}"),
        ]
    );
    assert!(plan.preserve.is_empty());
    assert!(plan.already_missing.is_empty());

    // An unrecorded pack id projects neither an installation nor a plan.
    assert!(
        plugins
            .list_pack_installations(ListPackInstallationsRequest {})
            .expect("list pack installations")
            .packs
            .iter()
            .all(|pack| pack.pack_id != "official/ora-space.absent")
    );
    assert!(
        plugins
            .pack_uninstall_plan(PackUninstallPlanRequest {
                plugin_id: "official/ora-space.absent".to_string(),
            })
            .expect("plan for an unrecorded pack")
            .plan
            .is_none(),
        "an unrecorded pack has no uninstall plan"
    );
}

/// The ownership journal is durable: a fresh plugin host on the same database reads the same
/// relationships without any re-install.
#[tokio::test]
async fn pack_ownership_survives_a_restart() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    );
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let preflight = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (_outcome, ledger) = api
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("pack members install");
    api.record_pack_run(
        &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
        "https://github.com/ora-space/marketplace",
        &ledger,
    )
    .expect("record pack ownership");
    let before = api
        .pack_installation(PACK_ID)
        .expect("load pack installation")
        .expect("the pack installation is recorded");

    // Reopen the plugin host over the same database: the relationships persist unchanged.
    drop(api);
    drop(pool);
    let restarted_pool = test_pool(data_dir.path());
    let restarted = pack_test_host(data_dir.path(), &restarted_pool);
    let after = restarted
        .pack_installation(PACK_ID)
        .expect("load pack installation after restart")
        .expect("the pack installation survives the restart");
    assert_eq!(after, before);
}

/// A member that is already installed is skipped without touching its version, while the rest
/// of the pack still installs.
#[tokio::test]
async fn pack_install_skips_an_already_installed_member() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    );
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    // Install the visible member first through the same chain, so the pack sees it installed.
    let visible_manifest = PluginManifest::parse(
        &std::fs::read_to_string(
            data_dir
                .path()
                .join("plugins")
                .join("sources")
                .join("github.com")
                .join("ora-space")
                .join("marketplace")
                .join("registry")
                .join("o")
                .join(VISIBLE_MEMBER)
                .join("orax.toml"),
        )
        .expect("read visible listing"),
    )
    .expect("parse visible listing");
    let digest = *ora_plugin_manifest::Sha256Digest::parse(
        &ora_utils::hash::sha256_file(fixture.member_artifacts[VISIBLE_MEMBER].as_path())
            .expect("hash artifact"),
    )
    .expect("digest")
    .as_bytes();
    Installer::new(ora_utils::http::LocalFileDownloader)
        .install(
            &visible_manifest,
            &namespace,
            ResolvedReleaseSource::universal(
                ora_utils::http::DownloadSource::Local(
                    fixture.member_artifacts[VISIBLE_MEMBER].clone(),
                ),
                digest,
            ),
            data_dir.path(),
        )
        .await
        .expect("pre-install the visible member");

    let preflight = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    assert_eq!(
        preflight.already_installed(),
        vec![format!("official/{VISIBLE_MEMBER}")]
    );
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, _ledger) = api
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("pack members install");
    match outcome {
        InstallOutcome::PackInstalled {
            members,
            skipped,
            failed,
        } => {
            assert_eq!(members, vec![format!("official/{HIDDEN_MEMBER}")]);
            assert_eq!(
                skipped,
                vec![format!("official/{VISIBLE_MEMBER}")],
                "the installed member is skipped, not reinstalled or reported as failed"
            );
            assert_eq!(failed, None);
        }
        other => panic!("expected a pack outcome, got {other:?}"),
    }
}

/// Every static pack problem fails preflight before any member downloads or lands on disk.
#[tokio::test]
async fn pack_preflight_failures_leave_the_installed_tree_untouched() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );

    // Duplicate member: the same identifier declared twice.
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(HIDDEN_MEMBER)
        ),
    );
    let error = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect_err("duplicate member fails preflight");
    assert!(matches!(
        error.public_error(),
        PublicError::PackMemberDuplicate(_)
    ));

    // Self-reference: a member that names the pack itself.
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &member_table("ora-space.python-extension-pack"),
    );
    let error = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect_err("self-reference fails preflight");
    assert!(matches!(
        error.public_error(),
        PublicError::PackSelfReference(_)
    ));

    // Unresolved member: the identifier exists in no listing of the pack's source.
    let fixture = stage_pack_fixture(data_dir.path(), &member_table("ora-space.missing"));
    let error = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect_err("missing member fails preflight");
    assert!(matches!(
        error.public_error(),
        PublicError::PackMemberNotFound(_)
    ));

    // Nested pack: a member that is itself a pack listing.
    stage_listing(
        &data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
        "ora-space.nested-pack",
        &pack_listing("ora-space.nested-pack", &member_table(HIDDEN_MEMBER)),
    );
    let fixture = stage_pack_fixture(data_dir.path(), &member_table("ora-space.nested-pack"));
    let error = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect_err("nested pack fails preflight");
    assert!(matches!(
        error.public_error(),
        PublicError::PackMemberNested(_)
    ));

    // No applicable member: the only member is agent-gated and no agent is installed.
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &member_table_with_agents(VISIBLE_MEMBER, "\"ora-space.codex\""),
    );
    let error = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect_err("an agent-gated member without a match fails preflight");
    assert!(matches!(
        error.public_error(),
        PublicError::PackNoApplicableMembers(_)
    ));

    assert!(
        !data_dir.path().join("plugins").join("installed").exists(),
        "a failed preflight never touches the installed tree"
    );
}

/// A member whose agent reference matches an installed agent plugin applies and installs.
#[tokio::test]
async fn pack_install_applies_an_agent_gated_member_whose_agent_is_installed() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    // The referenced agent is installed ahead of the pack: the matching rule reads the
    // installed tree, not the marketplace.
    let agent_root = data_dir
        .path()
        .join("plugins")
        .join("installed")
        .join("official")
        .join("ora-space.codex")
        .join("1.0.0");
    std::fs::create_dir_all(&agent_root).expect("create agent package dir");
    std::fs::write(
        agent_root.join("orax.toml"),
        "resolver = 1\nidentifier = \"ora-space.codex\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Codex agent\"\n",
    )
    .expect("write agent manifest");
    std::fs::write(agent_root.join("main.js"), "export {};\n").expect("write entrypoint");
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &member_table_with_agents(VISIBLE_MEMBER, "\"ora-space.codex\""),
    );
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );

    let preflight = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("the agent match admits the member");
    assert_eq!(preflight.already_installed(), Vec::<String>::new());
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, _ledger) = api
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("pack members install");
    match outcome {
        InstallOutcome::PackInstalled {
            members,
            skipped,
            failed,
        } => {
            assert_eq!(members.len(), 1);
            assert_eq!(members[0], format!("official/{VISIBLE_MEMBER}"));
            assert!(skipped.is_empty());
            assert_eq!(failed, None);
        }
        other => panic!("expected a pack outcome, got {other:?}"),
    }
}

/// Holds one data directory open from a background process so directory replacement fails on
/// Windows, and terminates the whole process tree on drop or explicit release.
///
/// This fault injection depends on Windows directory locking and is intentionally Windows-only:
/// a child process whose working directory sits inside the directory makes the directory's
/// rename/replacement fail deterministically. POSIX has no equivalent lock — another process can
/// still unlink a directory that is somebody's cwd — so gating this on `cfg(windows)` is the only
/// honest way to keep the qualification deterministic; Linux CI cannot produce the same fault.
#[cfg(windows)]
struct DirectoryHolder {
    /// The holder process, owned until it is terminated.
    ///
    /// `release` takes it so `Drop` cannot issue a second `taskkill` for a PID the OS may already
    /// have reused for an unrelated process — including the holder of the test running beside this
    /// one, whose directory has to stay pinned for its own rollback assertion.
    child: Option<std::process::Child>,
}

#[cfg(windows)]
impl DirectoryHolder {
    /// Spawns a background process whose working directory pins `directory`.
    fn spawn_holding(directory: &Path) -> Self {
        let child = Command::new("cmd")
            .args(["/c", "ping", "-n", "30", "127.0.0.1"])
            .current_dir(directory)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the directory holder");
        Self { child: Some(child) }
    }

    /// Terminates the holding process tree so the pinned directory becomes replaceable.
    fn release(&mut self) {
        if let Some(mut child) = self.child.take() {
            terminate_holder(&mut child);
        }
    }
}

/// Terminates one holder process tree and reaps it before its PID can be reused.
#[cfg(windows)]
fn terminate_holder(child: &mut std::process::Child) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &child.id().to_string()])
        .output();
    let _ = child.wait();
}

#[cfg(windows)]
impl Drop for DirectoryHolder {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            terminate_holder(&mut child);
        }
    }
}

/// A member that fails mid-run stops the pack and triggers the transactional rollback (D3-D):
/// the members created before the failure are removed in reverse order, the failed member never
/// lands, and the journal restores to its pre-run facts. This test pins the two-member shape of
/// that behavior; the deep rollback coverage lives in the D3-D tests below.
#[tokio::test]
async fn pack_install_stops_at_a_failing_member_and_rolls_back_created_members() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    );
    // Corrupt the second member's artifact after staging so its digest no longer matches the
    // listing: the download fails during the run, after the first member has landed.
    std::fs::write(
        fixture.member_artifacts[VISIBLE_MEMBER].as_path(),
        b"corrupted",
    )
    .expect("corrupt the second member artifact");
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );

    let preflight = api
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, ledger) = api
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("a member failure is reported inside a successful pack outcome");
    match outcome {
        InstallOutcome::PackInstalled {
            members,
            skipped,
            failed,
        } => {
            assert!(
                members.is_empty(),
                "the member that landed before the failure is rolled back"
            );
            assert!(skipped.is_empty());
            let failed = failed.expect("the failed member is identified");
            assert_eq!(failed.plugin_id, format!("official/{VISIBLE_MEMBER}"));
            assert!(!failed.error_code.is_empty());
            assert!(
                failed.rollback_failures.is_empty(),
                "the rollback of a single created member succeeds"
            );
        }
        other => panic!("expected a pack outcome, got {other:?}"),
    }
    assert!(
        !data_dir
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join(HIDDEN_MEMBER)
            .join("1.0.0")
            .is_dir(),
        "the completed member is rolled back (D3-D transactional semantics)"
    );
    assert_eq!(ledger.rollback_failed(), Vec::<String>::new());
    assert_eq!(ledger.installed(), &[]);
}

/// A locally imported Hook package installs as an ordinary plugin, and the installed entry reports
/// the executable the host would run together with the target it was built for.
#[test]
fn importing_a_hook_package_reports_its_executable_and_target() {
    with_trace_logging(|| {
        let Some(host) = ora_plugin_registry::current_host_target() else {
            eprintln!("skipping Hook import: compiled host is not a plugin target");
            return;
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async move {
                let data_dir = TempDir::new().expect("data dir");
                let pool = test_pool(data_dir.path());
                let api = test_plugin_api(data_dir.path(), &pool);
                let package = data_dir.path().join("rtk.orax");
                write_hook_orax(&package, "rtk-ai.rtk", Some(host.as_str()));

                let response = api
                    .import(ImportPluginRequest {
                        hook_execution_acknowledged: false,
                        path: package.to_string_lossy().into_owned(),
                    })
                    .await
                    .expect("import Hook package");
                assert_eq!(
                    response,
                    ora_contracts::ImportPluginResponse {
                        workflows: Vec::new(),
                        plugin_id: "local/rtk-ai.rtk".to_string(),
                        outcome: InstallOutcome::Installed,
                    },
                    "a Hook import is an ordinary install with no conflict outcome"
                );

                let listed = api
                    .list_installed(ListInstalledPluginsRequest {})
                    .expect("installed snapshot");
                let installed = listed
                    .plugins
                    .iter()
                    .find(|plugin| plugin.id == "local/rtk-ai.rtk")
                    .expect("the imported Hook is listed");
                assert_eq!(
                    installed.contribution,
                    ora_contracts::InstalledPluginContribution::Hook {
                        executable: "assets/rtk.exe".to_string(),
                        supported_agents: Vec::new(),
                        target: Some(host.as_str().to_string()),
                    }
                );
            });
    });
}

/// One Start-only workflow document the run engine accepts, carrying an explicit version.
const WORKFLOW_DOCUMENT: &str = r#"{"name":"导入流程","version":"1.0.0","viewport":{"x":0,"y":0,"zoom":1},"nodes":[{"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","title":"开始"}}],"edges":[]}"#;

/// Writes a Workflow `.orax` carrying the given `assets/workflows/<name>` documents.
fn write_workflow_orax(path: &Path, identifier: &str, documents: &[(&str, &str)]) {
    write_workflow_orax_version(path, identifier, "0.1.0", documents);
}

/// Writes a Workflow `.orax` whose manifest declares `version`.
///
/// The installer refuses a package whose own version disagrees with the listing that delivered it,
/// so an upgrade fixture has to state the new version in both places.
fn write_workflow_orax_version(
    path: &Path,
    identifier: &str,
    version: &str,
    documents: &[(&str, &str)],
) {
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"workflow\"\nversion = \"{version}\"\ndescription = \"Workflow package\"\n"
    );
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default();
    writer.start_file("orax.toml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    for (name, contents) in documents {
        writer
            .start_file(format!("assets/workflows/{name}"), options)
            .unwrap();
        writer.write_all(contents.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
}

/// A Workflow package installs and turns each document into a workflow with a published
/// snapshot, reporting one malformed document on its own without costing the user the working
/// workflow beside it.
#[test]
fn imports_workflow_package_documents_alongside_the_plugin() {
    with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async move {
                let data_dir = TempDir::new().expect("data dir");
                let pool = test_pool(data_dir.path());
                let api = test_plugin_api(data_dir.path(), &pool);
                let archive = data_dir.path().join("workflows.orax");
                write_workflow_orax(
                    &archive,
                    "ora.workflows",
                    &[
                        ("1.0.0.json", WORKFLOW_DOCUMENT),
                        ("2.0.0.json", "{ not json"),
                    ],
                );

                let response = api
                    .import(ImportPluginRequest {
                        path: archive.to_string_lossy().into_owned(),
                        hook_execution_acknowledged: false,
                    })
                    .await
                    .expect("import workflow package");

                // The package itself installs under the reserved local namespace.
                assert_eq!(response.plugin_id, "local/ora.workflows");
                assert_eq!(response.outcome, InstallOutcome::Installed);

                let [imported, failed] = response.workflows.as_slice() else {
                    panic!(
                        "expected two document outcomes, got {:?}",
                        response.workflows
                    );
                };
                let ImportedWorkflowOutcome::Imported {
                    source_file,
                    workflow_id,
                    name,
                    version,
                } = imported
                else {
                    panic!("expected the valid document to import, got {imported:?}");
                };
                assert_eq!(source_file, "assets/workflows/1.0.0.json");
                assert_eq!(name, "导入流程");
                assert_eq!(version, "1.0.0");
                assert!(
                    !workflow_id.is_empty(),
                    "import must report the created workflow"
                );

                let ImportedWorkflowOutcome::Failed {
                    source_file,
                    reason,
                } = failed
                else {
                    panic!("expected the malformed document to fail, got {failed:?}");
                };
                assert_eq!(source_file, "assets/workflows/2.0.0.json");
                assert!(reason.contains("not valid JSON"), "{reason}");
            });
    });
}

/// A fresh pack install reconciles every member as present at the recorded version.
#[tokio::test]
async fn reconcile_reports_expected_present_after_a_fresh_pack_install() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    install_and_record_pack(
        data_dir.path(),
        &api,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("install and record the pack");
    let reconciliation = api
        .reconcile_pack_installation(PACK_ID)
        .expect("reconcile the recorded pack")
        .expect("the recorded pack reconciles");
    assert_eq!(reconciliation.pack_id(), PACK_ID);
    assert_eq!(
        reconciliation.members(),
        vec![
            PackMemberReconciliation::ExpectedAndPresent {
                member_id: format!("official/{VISIBLE_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::ManagedByPack,
            },
            PackMemberReconciliation::ExpectedAndPresent {
                member_id: format!("official/{HIDDEN_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::ManagedByPack,
            },
        ]
    );
}

/// An externally deleted member reconciles as missing, with the journal untouched.
#[tokio::test]
async fn reconcile_identifies_a_missing_member_without_touching_the_ledger() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    install_and_record_pack(
        data_dir.path(),
        &api,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("install and record the pack");
    std::fs::remove_dir_all(
        data_dir
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join(HIDDEN_MEMBER),
    )
    .expect("delete the member externally");
    let reconciliation = api
        .reconcile_pack_installation(PACK_ID)
        .expect("reconcile the recorded pack")
        .expect("the recorded pack reconciles");
    assert_eq!(
        reconciliation.members(),
        vec![
            PackMemberReconciliation::ExpectedAndPresent {
                member_id: format!("official/{VISIBLE_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::ManagedByPack,
            },
            PackMemberReconciliation::Missing {
                member_id: format!("official/{HIDDEN_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::ManagedByPack,
            },
        ]
    );
    let journal = api
        .pack_installation(PACK_ID)
        .expect("load the journal")
        .expect("the journal survives reconciliation");
    assert_eq!(journal.members.len(), 2);
}

/// An independently upgraded member reconciles as version-changed: the classification changes
/// but the journal keeps the historical version and the original ownership for both ownership
/// kinds.
#[tokio::test]
async fn reconcile_identifies_an_independently_upgraded_member_and_keeps_the_journal_facts() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    // The visible member is pre-installed, so the journal records it as pre-existing.
    install_member_version(data_dir.path(), VISIBLE_MEMBER, "1.0.0").await;
    install_and_record_pack(
        data_dir.path(),
        &api,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("install and record the pack");
    // Both members are upgraded independently after the pack install.
    install_member_version(data_dir.path(), HIDDEN_MEMBER, "2.0.0").await;
    install_member_version(data_dir.path(), VISIBLE_MEMBER, "2.0.0").await;
    let reconciliation = api
        .reconcile_pack_installation(PACK_ID)
        .expect("reconcile the recorded pack")
        .expect("the recorded pack reconciles");
    assert_eq!(
        reconciliation.members(),
        vec![
            PackMemberReconciliation::VersionChanged {
                member_id: format!("official/{VISIBLE_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                current_version: "2.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::PreExisting,
            },
            PackMemberReconciliation::VersionChanged {
                member_id: format!("official/{HIDDEN_MEMBER}"),
                version_at_install: "1.0.0".to_string(),
                current_version: "2.0.0".to_string(),
                ownership: ora_db::PackMemberOwnership::ManagedByPack,
            },
        ]
    );
    // Reconciliation is read-only: the journal still records the historical facts.
    let journal = api
        .pack_installation(PACK_ID)
        .expect("load the journal")
        .expect("the journal survives reconciliation");
    assert!(
        journal
            .members
            .iter()
            .all(|member| { member.version_at_install == "1.0.0" })
    );
}

/// Reconciliation never invents a relationship for a pack that no install ever recorded.
#[tokio::test]
async fn reconcile_returns_none_for_an_unrecorded_pack() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = pack_test_host(data_dir.path(), &pool);
    assert_eq!(
        api.reconcile_pack_installation(PACK_ID).expect("reconcile"),
        None
    );
}
/// A fresh pack uninstall removes every member the pack created and clears the journal.
#[tokio::test]
async fn pack_uninstall_removes_every_managed_member_and_clears_the_journal() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    install_and_record_pack(
        data_dir.path(),
        &host,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("install and record the pack");

    let response = plugins
        .uninstall(UninstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_string(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .expect("uninstall the pack");
    assert_eq!(response.plugin_id, PACK_ID);
    assert!(
        !member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the managed member is removed"
    );
    assert!(
        !member_installed(data_dir.path(), VISIBLE_MEMBER, "1.0.0"),
        "the other managed member is removed"
    );
    assert!(
        host.pack_installation(PACK_ID)
            .expect("load the journal")
            .is_none(),
        "the journal is cleared once every relationship is released"
    );
}

/// A pre-existing member survives the pack uninstall: the pack never created it, so it never
/// touches it, and the relationship is simply released.
#[tokio::test]
async fn pack_uninstall_preserves_a_pre_existing_member() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    install_member_version(data_dir.path(), VISIBLE_MEMBER, "1.0.0").await;
    install_and_record_pack(
        data_dir.path(),
        &host,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("install and record the pack");

    plugins
        .uninstall(UninstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_string(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .expect("uninstall the pack");
    assert!(
        member_installed(data_dir.path(), VISIBLE_MEMBER, "1.0.0"),
        "the pre-existing member survives the pack uninstall"
    );
    assert!(
        !member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the managed member is removed"
    );
    assert!(
        host.pack_installation(PACK_ID)
            .expect("load the journal")
            .is_none(),
        "the released relationship leaves no journal"
    );
}

/// A member the pack created but the user independently upgraded survives the pack uninstall.
#[tokio::test]
async fn pack_uninstall_preserves_an_independently_upgraded_member() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    install_and_record_pack(
        data_dir.path(),
        &host,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("install and record the pack");
    install_member_version(data_dir.path(), HIDDEN_MEMBER, "2.0.0").await;

    plugins
        .uninstall(UninstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_string(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .expect("uninstall the pack");
    assert!(
        member_installed(data_dir.path(), HIDDEN_MEMBER, "2.0.0"),
        "the independently upgraded member is preserved"
    );
    assert!(
        !member_installed(data_dir.path(), VISIBLE_MEMBER, "1.0.0"),
        "the member that stayed at its recorded version is removed"
    );
    assert!(
        host.pack_installation(PACK_ID)
            .expect("load the journal")
            .is_none(),
        "the journal is cleared after the run"
    );
}

/// A managed member that is already missing releases its relationship without filesystem work,
/// and the pack uninstall still completes.
#[tokio::test]
async fn pack_uninstall_completes_when_a_managed_member_is_already_missing() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    install_and_record_pack(
        data_dir.path(),
        &host,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("install and record the pack");
    std::fs::remove_dir_all(
        data_dir
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join(HIDDEN_MEMBER),
    )
    .expect("delete the member externally");

    plugins
        .uninstall(UninstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_string(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .expect("the missing member does not fail the uninstall");
    assert!(
        !member_installed(data_dir.path(), VISIBLE_MEMBER, "1.0.0"),
        "the remaining member is still removed"
    );
    assert!(
        host.pack_installation(PACK_ID)
            .expect("load the journal")
            .is_none(),
        "the journal is cleared"
    );
}

/// A mixed pack removes only the eligible member: pre-existing and independently upgraded
/// members are preserved, and the journal is cleared.
#[tokio::test]
async fn pack_uninstall_removes_only_the_eligible_member_in_a_mixed_pack() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const THIRD_MEMBER: &str = "ora-space.third-tools";
    // The visible member is pre-existing; the third member starts at 1.0.0 and is upgraded
    // after the pack install, so every preserve reason is exercised at once.
    install_member_version(data_dir.path(), VISIBLE_MEMBER, "1.0.0").await;
    let third_artifact = build_member_artifact(data_dir.path(), THIRD_MEMBER, "1.0.0");
    stage_listing(
        &data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
        THIRD_MEMBER,
        &skill_listing(
            THIRD_MEMBER,
            &ora_utils::hash::sha256_file(&third_artifact).expect("hash the third artifact"),
            true,
        ),
    );
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER),
            member_table(THIRD_MEMBER)
        ),
    );
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let mut preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    for member in preflight.applicable_mut() {
        let artifact = match member.plugin_id.name() {
            HIDDEN_MEMBER => fixture.member_artifacts[HIDDEN_MEMBER].clone(),
            VISIBLE_MEMBER => fixture.member_artifacts[VISIBLE_MEMBER].clone(),
            THIRD_MEMBER => third_artifact.clone(),
            other => panic!("unexpected member {other}"),
        };
        let digest = *ora_plugin_manifest::Sha256Digest::parse(
            &ora_utils::hash::sha256_file(&artifact).expect("hash artifact"),
        )
        .expect("digest")
        .as_bytes();
        member.release = ResolvedReleaseSource::universal(
            ora_utils::http::DownloadSource::Local(artifact),
            digest,
        );
    }
    let (_outcome, ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("pack members install");
    host.record_pack_run(
        &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
        "https://github.com/ora-space/marketplace",
        &ledger,
    )
    .expect("record pack ownership");
    install_member_version(data_dir.path(), THIRD_MEMBER, "2.0.0").await;

    plugins
        .uninstall(UninstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_string(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .expect("uninstall the pack");
    assert!(
        !member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the managed member at its recorded version is removed"
    );
    assert!(
        member_installed(data_dir.path(), VISIBLE_MEMBER, "1.0.0"),
        "the pre-existing member is preserved"
    );
    assert!(
        member_installed(data_dir.path(), THIRD_MEMBER, "2.0.0"),
        "the independently upgraded member is preserved"
    );
    assert!(
        host.pack_installation(PACK_ID)
            .expect("load the journal")
            .is_none(),
        "the journal is cleared"
    );
}

/// The ownership journal is durable: a fresh plugin host on the same database reads the same
/// relationships without any re-install. This test also verifies the full rollback of created
/// members when a later member fails during the pack install.
#[tokio::test]
async fn rollback_removes_created_members_when_a_later_member_fails() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let host = pack_test_host(data_dir.path(), &pool);
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    );
    // Corrupt the second member's archive so its extraction fails after the first member has
    // already landed.
    std::fs::write(
        fixture.member_artifacts[VISIBLE_MEMBER].as_path(),
        b"corrupted bytes",
    )
    .expect("corrupt the visible artifact");
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the failure is reported inside the pack outcome");

    assert_eq!(
        outcome,
        InstallOutcome::PackInstalled {
            members: Vec::new(),
            skipped: Vec::new(),
            failed: Some(PackInstallFailure {
                plugin_id: format!("official/{VISIBLE_MEMBER}"),
                error_code: "internal_error".to_string(),
                rollback_failures: Vec::new(),
            }),
        }
    );
    assert!(
        !member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the created member is rolled back"
    );
    assert!(
        !member_installed(data_dir.path(), VISIBLE_MEMBER, "1.0.0"),
        "the failed member never landed"
    );
    // A fully rolled-back run restores the journal to its pre-run facts: nothing is recorded.
    assert!(
        host.pack_installation(PACK_ID)
            .expect("load the journal")
            .is_none(),
        "the journal stays empty after a complete rollback"
    );
    assert_eq!(ledger.rollback_failed(), Vec::<String>::new());
}

/// The rollback never touches pre-existing members: only members created by the failed run are
/// removed, and the journal gains no phantom pre-existing relationships.
#[tokio::test]
async fn rollback_never_touches_pre_existing_members() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let host = pack_test_host(data_dir.path(), &pool);
    const THIRD_MEMBER: &str = "ora-space.third-tools";
    // The visible member is pre-existing; the third member's archive is corrupted so its
    // install fails after the hidden member has landed.
    install_member_version(data_dir.path(), VISIBLE_MEMBER, "1.0.0").await;
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER),
            member_table(THIRD_MEMBER)
        ),
    );
    stage_listing(
        &data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
        THIRD_MEMBER,
        &skill_listing(THIRD_MEMBER, "cd".repeat(32).as_str(), true),
    );
    std::fs::write(
        data_dir
            .path()
            .join("artifacts")
            .join(format!("{THIRD_MEMBER}-v1.0.0.orax")),
        b"corrupted bytes",
    )
    .expect("corrupt the third artifact");
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, _ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the failure is reported inside the pack outcome");

    assert!(
        !member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the created member is rolled back"
    );
    assert!(
        member_installed(data_dir.path(), VISIBLE_MEMBER, "1.0.0"),
        "the pre-existing member is never touched by the rollback"
    );
    assert!(
        !member_installed(data_dir.path(), THIRD_MEMBER, "1.0.0"),
        "the failed member never landed"
    );
    // The journal stays at its pre-run state: no phantom pre-existing relationship is minted
    // for the skipped member by a run that did not complete.
    assert!(
        host.pack_installation(PACK_ID)
            .expect("load the journal")
            .is_none(),
        "a failed and fully rolled-back run leaves the journal untouched"
    );
    let _ = outcome;
}

/// Three members install in declaration order and the third fails: the rollback runs in
/// reverse creation order (B then A) and clears every created member.
#[tokio::test]
async fn rollback_runs_in_reverse_order_and_clears_every_created_member() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let host = pack_test_host(data_dir.path(), &pool);
    const THIRD_MEMBER: &str = "ora-space.third-tools";
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER),
            member_table(THIRD_MEMBER)
        ),
    );
    stage_listing(
        &data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
        THIRD_MEMBER,
        &skill_listing(THIRD_MEMBER, "cd".repeat(32).as_str(), true),
    );
    std::fs::write(
        data_dir
            .path()
            .join("artifacts")
            .join(format!("{THIRD_MEMBER}-v1.0.0.orax")),
        b"corrupted bytes",
    )
    .expect("corrupt the third artifact");
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, _ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the failure is reported inside the pack outcome");

    match outcome {
        InstallOutcome::PackInstalled {
            members, failed, ..
        } => {
            assert!(members.is_empty(), "both created members are rolled back");
            let failed = failed.expect("the third member failed");
            assert_eq!(failed.plugin_id, format!("official/{THIRD_MEMBER}"));
            assert!(
                failed.rollback_failures.is_empty(),
                "both rollbacks succeeded"
            );
        }
        other => panic!("expected a pack outcome, got {other:?}"),
    }
    assert!(
        !member_installed(data_dir.path(), VISIBLE_MEMBER, "1.0.0"),
        "the second-created member is rolled back first"
    );
    assert!(
        !member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the first-created member is rolled back last"
    );
}

/// A rollback that fails leaves the residual member installed, journals it as pack-managed,
/// and surfaces the rollback failure next to the original install failure.
///
/// The filesystem fault is produced by holding the member's data directory open from a child
/// process (`DirectoryHolder`), which only Windows directory locking turns into a deterministic
/// rollback failure; POSIX cannot reproduce it, so the test is intentionally Windows-only and the
/// cross-platform rollback, journal, and reconciliation behavior stays covered by the D3-D and
/// `reconcile_*` tests above.
#[cfg(windows)]
#[tokio::test]
async fn a_failed_rollback_keeps_residual_members_and_journals_them() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const THIRD_MEMBER: &str = "ora-space.third-tools";
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER),
            member_table(THIRD_MEMBER)
        ),
    );
    stage_listing(
        &data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
        THIRD_MEMBER,
        &skill_listing(THIRD_MEMBER, "cd".repeat(32).as_str(), true),
    );
    std::fs::write(
        data_dir
            .path()
            .join("artifacts")
            .join(format!("{THIRD_MEMBER}-v1.0.0.orax")),
        b"corrupted bytes",
    )
    .expect("corrupt the third artifact");
    // The hidden member's data directory is held open by a child process whose working
    // directory is that directory: on Windows a directory in use cannot be renamed, which
    // deterministically fails the hidden member's rollback while the earlier member's
    // rollback succeeds.
    let hidden_data_dir = data_dir
        .path()
        .join("plugins")
        .join("data")
        .join("official")
        .join(HIDDEN_MEMBER);
    std::fs::create_dir_all(&hidden_data_dir).expect("create held data dir");
    let mut holder = DirectoryHolder::spawn_holding(&hidden_data_dir);
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the failure is reported inside the pack outcome");

    // The visible member rolled back; the hidden member's rollback failed, so it remains
    // installed as the residual evidence.
    assert!(
        !member_installed(data_dir.path(), VISIBLE_MEMBER, "1.0.0"),
        "the earlier rollback succeeded"
    );
    assert!(
        member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the rollback failure leaves the residual member installed"
    );
    match outcome {
        InstallOutcome::PackInstalled {
            members,
            failed,
            skipped: _,
        } => {
            assert_eq!(
                members,
                vec![format!("official/{HIDDEN_MEMBER}")],
                "the residual member is reported as installed"
            );
            let failed = failed.expect("the third member failed");
            assert_eq!(failed.plugin_id, format!("official/{THIRD_MEMBER}"));
            assert_eq!(
                failed
                    .rollback_failures
                    .iter()
                    .map(|failure| failure.plugin_id.as_str())
                    .collect::<Vec<_>>(),
                vec![format!("official/{HIDDEN_MEMBER}")],
                "the rollback failure is diagnosed without replacing the primary failure"
            );
        }
        other => panic!("expected a pack outcome, got {other:?}"),
    }
    // The residual evidence is durable: the journal records the residual member as
    // pack-managed while the rolled-back member and the failed member are absent.
    host.record_pack_run(
        &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
        "https://github.com/ora-space/marketplace",
        &ledger,
    )
    .expect("record the residual evidence");
    let journal = host
        .pack_installation(PACK_ID)
        .expect("load the journal")
        .expect("the residual run is journaled");
    assert_eq!(
        journal
            .members
            .iter()
            .map(|member| (member.member_id.as_str(), member.ownership,))
            .collect::<Vec<_>>(),
        vec![(
            format!("official/{HIDDEN_MEMBER}").as_str(),
            ora_db::PackMemberOwnership::ManagedByPack,
        )],
    );
    let _ = ledger;
    let _ = plugins;
    holder.release();
}

/// After a complete rollback a retry installs the pack normally, with no phantom ownership.
#[tokio::test]
async fn retry_after_a_complete_rollback_installs_normally() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let host = pack_test_host(data_dir.path(), &pool);
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    );
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    // First attempt: the visible artifact is corrupted, so the run fails and rolls back.
    std::fs::write(
        fixture.member_artifacts[VISIBLE_MEMBER].as_path(),
        b"corrupted bytes",
    )
    .expect("corrupt the visible artifact");
    let preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (failed_outcome, _ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the failure is reported inside the pack outcome");
    assert!(
        matches!(
            failed_outcome,
            InstallOutcome::PackInstalled {
                failed: Some(_),
                ..
            }
        ),
        "the first attempt fails and rolls back"
    );

    // Retry: the artifact is rebuilt at the exact path the fixture registered, so the transfer
    // substitution picks up the valid archive, and both members install.
    write_skill_orax(
        fixture.member_artifacts[VISIBLE_MEMBER].as_path(),
        VISIBLE_MEMBER,
    );
    let preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds on retry");
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the retry installs the pack");
    assert!(matches!(
        outcome,
        InstallOutcome::PackInstalled { failed: None, .. }
    ));
    host.record_pack_run(
        &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
        "https://github.com/ora-space/marketplace",
        &ledger,
    )
    .expect("record pack ownership");
    let journal = host
        .pack_installation(PACK_ID)
        .expect("load the journal")
        .expect("the retry records ownership");
    assert_eq!(journal.members.len(), 2);
    assert!(
        journal
            .members
            .iter()
            .all(|member| member.ownership == ora_db::PackMemberOwnership::ManagedByPack),
        "the retry records plain pack-managed ownership with no phantom facts"
    );
}

/// A partially rolled-back failure is honestly reconciled after a restart, and the residual
/// member can still be removed by a pack uninstall.
///
/// Like `a_failed_rollback_keeps_residual_members_and_journals_them`, the filesystem fault comes
/// from Windows directory locking (`DirectoryHolder`) and cannot be reproduced on POSIX, so the
/// test is intentionally Windows-only; generic restart reconciliation is covered cross-platform
/// by the `reconcile_*` and `pack_ownership_survives_a_restart` tests.
#[cfg(windows)]
#[tokio::test]
async fn partial_rollback_residual_is_reconciled_after_a_restart() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const THIRD_MEMBER: &str = "ora-space.third-tools";
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER),
            member_table(THIRD_MEMBER)
        ),
    );
    stage_listing(
        &data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
        THIRD_MEMBER,
        &skill_listing(THIRD_MEMBER, "cd".repeat(32).as_str(), true),
    );
    std::fs::write(
        data_dir
            .path()
            .join("artifacts")
            .join(format!("{THIRD_MEMBER}-v1.0.0.orax")),
        b"corrupted bytes",
    )
    .expect("corrupt the third artifact");
    let hidden_data_dir = data_dir
        .path()
        .join("plugins")
        .join("data")
        .join("official")
        .join(HIDDEN_MEMBER);
    std::fs::create_dir_all(&hidden_data_dir).expect("create held data dir");
    let mut holder = DirectoryHolder::spawn_holding(&hidden_data_dir);
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (_outcome, ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the failure is reported inside the pack outcome");
    // The rollback failure journaled the residual member as the durable evidence.
    host.record_pack_run(
        &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
        "https://github.com/ora-space/marketplace",
        &ledger,
    )
    .expect("record the residual evidence");

    // Restart: a fresh host reconciles the residual member honestly.
    drop(plugins);
    drop(host);
    drop(pool);
    let restarted_pool = test_pool(data_dir.path());
    let (restarted_plugins, restarted_host) = pack_test_plugins(data_dir.path(), &restarted_pool);
    let reconciliation = restarted_host
        .reconcile_pack_installation(PACK_ID)
        .expect("reconcile after restart")
        .expect("the residual journal reconciles");
    assert_eq!(
        reconciliation.members(),
        &[PackMemberReconciliation::ExpectedAndPresent {
            member_id: format!("official/{HIDDEN_MEMBER}"),
            version_at_install: "1.0.0".to_string(),
            ownership: ora_db::PackMemberOwnership::ManagedByPack,
        }],
        "the residual member is honestly visible after the restart"
    );

    // The residual member is still pack-owned and at its recorded version, so a pack
    // uninstall removes it and clears the journal. The directory holder from the failed
    // rollback is terminated first — on Windows its working-directory handle would block the
    // member's package removal — and the uninstall is retried briefly, exercising the exact
    // resume-after-transient-blockage behavior the journal was designed for.
    holder.release();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let mut uninstalled = None;
    for attempt in 0..5 {
        match restarted_plugins
            .uninstall(UninstallPluginRequest {
                hook_execution_acknowledged: false,
                plugin_id: PACK_ID.to_string(),
                data_disposition: PluginDataDisposition::Delete,
            })
            .await
        {
            Ok(response) => {
                uninstalled = Some(response);
                break;
            }
            Err(error) if attempt < 4 => {
                eprintln!("   uninstall retry ({attempt}): {error}");
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
            Err(error) => panic!("uninstall the residual pack: {error}"),
        }
    }
    let response = uninstalled.expect("uninstall eventually succeeds");
    assert_eq!(response.plugin_id, PACK_ID);
    assert!(
        !member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the residual member is removed by the pack uninstall"
    );
    assert!(
        restarted_host
            .pack_installation(PACK_ID)
            .expect("load the journal after uninstall")
            .is_none(),
        "the journal is cleared"
    );
}

/// A failed rollback never deletes ownership relationships that existed before the run.
#[tokio::test]
async fn a_failed_rollback_never_deletes_prior_journal_relations() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let host = pack_test_host(data_dir.path(), &pool);
    const THIRD_MEMBER: &str = "ora-space.third-tools";
    // Run 1 installs both members and records their relationships.
    install_and_record_pack(
        data_dir.path(),
        &host,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("run 1 records the pack");
    let before = host
        .pack_installation(PACK_ID)
        .expect("load the journal before run 2")
        .expect("the journal holds run 1 relationships");

    // The hidden member's package is deleted externally; its journal row remains.
    std::fs::remove_dir_all(
        data_dir
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join(HIDDEN_MEMBER),
    )
    .expect("delete the hidden member externally");

    // Run 2 re-creates the hidden member and fails on the corrupted third member; the
    // rollback removes the re-created hidden member. The journal must keep both run 1
    // relationships untouched.
    let fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER),
            member_table(THIRD_MEMBER)
        ),
    );
    stage_listing(
        &data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
        THIRD_MEMBER,
        &skill_listing(THIRD_MEMBER, "cd".repeat(32).as_str(), true),
    );
    std::fs::write(
        data_dir
            .path()
            .join("artifacts")
            .join(format!("{THIRD_MEMBER}-v1.0.0.orax")),
        b"corrupted bytes",
    )
    .expect("corrupt the third artifact");
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );
    let preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (_outcome, _ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the failure is reported inside the pack outcome");

    assert!(
        !member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the re-created member is rolled back"
    );
    let after = host
        .pack_installation(PACK_ID)
        .expect("load the journal after run 2")
        .expect("the prior journal survives the failed run 2");
    assert_eq!(after, before, "the prior journal relations are untouched");
}
/// Reads the member id off any reconciliation classification.
fn member_member_id(member: &PackMemberReconciliation) -> &str {
    match member {
        PackMemberReconciliation::ExpectedAndPresent { member_id, .. }
        | PackMemberReconciliation::VersionChanged { member_id, .. }
        | PackMemberReconciliation::Missing { member_id, .. } => member_id,
    }
}

/// Lifecycle Path A: sync → install pack → ownership recorded → restart reconcile
/// (ExpectedAndPresent) → independently upgrade a managed member → reconcile
/// (VersionChanged) → uninstall pack → unchanged member removed, upgraded member preserved,
/// journal cleared.
#[tokio::test]
async fn lifecycle_path_a_install_reconcile_upgrade_uninstall() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    install_and_record_pack(
        data_dir.path(),
        &host,
        &format!(
            "{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER)
        ),
    )
    .await
    .expect("install and record the pack");

    // Restart: ownership survives and reconciles as expected-and-present.
    drop(plugins);
    drop(host);
    drop(pool);
    let restarted_pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &restarted_pool);
    let reconciliation = host
        .reconcile_pack_installation(PACK_ID)
        .expect("reconcile after restart")
        .expect("the pack reconciles after restart");
    assert!(
        reconciliation
            .members()
            .iter()
            .all(|member| matches!(member, PackMemberReconciliation::ExpectedAndPresent { .. }))
    );

    // Independently upgrade one managed member: reconcile flips to version-changed while the
    // other member stays expected.
    install_member_version(data_dir.path(), VISIBLE_MEMBER, "2.0.0").await;
    let after_upgrade = host
        .reconcile_pack_installation(PACK_ID)
        .expect("reconcile after upgrade")
        .expect("the pack reconciles after upgrade");
    assert_eq!(
        after_upgrade
            .members()
            .iter()
            .map(|member| (
                member_member_id(member).to_owned(),
                std::mem::discriminant(member)
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                format!("official/{VISIBLE_MEMBER}"),
                std::mem::discriminant(&PackMemberReconciliation::VersionChanged {
                    member_id: String::new(),
                    version_at_install: String::new(),
                    current_version: String::new(),
                    ownership: ora_db::PackMemberOwnership::ManagedByPack,
                }),
            ),
            (
                format!("official/{HIDDEN_MEMBER}"),
                std::mem::discriminant(&PackMemberReconciliation::ExpectedAndPresent {
                    member_id: String::new(),
                    version_at_install: String::new(),
                    ownership: ora_db::PackMemberOwnership::ManagedByPack,
                }),
            ),
        ],
        "the upgraded member is version-changed; the untouched member stays expected"
    );

    // Uninstall: the version-changed member is preserved, the unchanged managed member is
    // removed, and the journal is cleared.
    plugins
        .uninstall(UninstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_string(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .expect("uninstall the pack");
    assert!(
        member_installed(data_dir.path(), VISIBLE_MEMBER, "2.0.0"),
        "the independently upgraded member is preserved"
    );
    assert!(
        !member_installed(data_dir.path(), HIDDEN_MEMBER, "1.0.0"),
        "the unchanged managed member is removed"
    );
    assert!(
        host.pack_installation(PACK_ID)
            .expect("load the journal")
            .is_none(),
        "the journal is cleared"
    );
}

/// Lifecycle Path B: install pack where the third member fails → reverse rollback → restart
/// with no phantom ownership → retry → final install succeeds and ownership is recorded.
#[tokio::test]
async fn lifecycle_path_b_failed_install_rolls_back_and_retry_succeeds() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const THIRD_MEMBER: &str = "ora-space.third-tools";
    let mut fixture = stage_pack_fixture(
        data_dir.path(),
        &format!(
            "{}{}{}",
            member_table(HIDDEN_MEMBER),
            member_table(VISIBLE_MEMBER),
            member_table(THIRD_MEMBER)
        ),
    );
    stage_listing(
        &data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
        THIRD_MEMBER,
        &skill_listing(THIRD_MEMBER, "cd".repeat(32).as_str(), true),
    );
    std::fs::write(
        data_dir
            .path()
            .join("artifacts")
            .join(format!("{THIRD_MEMBER}-v1.0.0.orax")),
        b"corrupted bytes",
    )
    .expect("corrupt the third artifact");
    let namespace = ora_domain::PluginNamespace::official();
    let source = ora_plugin_registry::RegistrySource::new(
        "https://github.com/ora-space/marketplace",
        namespace.clone(),
        gitlancer::BranchName::new("main"),
        data_dir
            .path()
            .join("plugins")
            .join("sources")
            .join("github.com")
            .join("ora-space")
            .join("marketplace"),
    );

    // First attempt: hidden and visible land, third fails → reverse rollback clears both.
    let preflight = host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds");
    let installer = Installer::new(ora_utils::http::LocalFileDownloader);
    let preflight = with_local_releases(&fixture, preflight);
    let (failed_outcome, _ledger) = host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the failure is reported inside the pack outcome");
    match failed_outcome {
        InstallOutcome::PackInstalled {
            members,
            failed,
            skipped: _,
        } => {
            assert!(members.is_empty(), "created members are rolled back");
            let failed = failed.expect("the third member failed");
            assert_eq!(failed.plugin_id, format!("official/{THIRD_MEMBER}"));
            assert!(failed.rollback_failures.is_empty());
        }
        other => panic!("expected a pack outcome, got {other:?}"),
    }
    assert!(
        host.pack_installation(PACK_ID)
            .expect("load the journal")
            .is_none(),
        "no phantom ownership survives the rolled-back attempt"
    );

    // Restart: the rolled-back state persists and the retry installs everything. The third
    // member's artifact is rebuilt and registered with the fixture so the transfer-substitution
    // map covers it on the retry.
    drop(plugins);
    drop(host);
    drop(pool);
    let restarted_pool = test_pool(data_dir.path());
    let (_restarted_plugins, restarted_host) = pack_test_plugins(data_dir.path(), &restarted_pool);
    let third_artifact = build_member_artifact(data_dir.path(), THIRD_MEMBER, "1.0.0");
    fixture
        .member_artifacts
        .insert(THIRD_MEMBER, third_artifact);
    let preflight = restarted_host
        .preflight_pack(&fixture.pack_manifest, &namespace, &source)
        .expect("pack preflight succeeds on retry");
    let preflight = with_local_releases(&fixture, preflight);
    let (outcome, ledger) = restarted_host
        .install_members(&namespace, preflight, &installer, /*progress*/ None)
        .await
        .expect("the retry installs the pack");
    match outcome {
        InstallOutcome::PackInstalled {
            members, failed, ..
        } => {
            assert_eq!(members.len(), 3, "all three members install on retry");
            assert_eq!(failed, None);
        }
        other => panic!("expected a pack outcome, got {other:?}"),
    }
    restarted_host
        .record_pack_run(
            &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
            "https://github.com/ora-space/marketplace",
            &ledger,
        )
        .expect("record ownership");
    let journal = restarted_host
        .pack_installation(PACK_ID)
        .expect("load the journal")
        .expect("the retry records ownership");
    assert_eq!(journal.members.len(), 3);
    assert!(
        journal
            .members
            .iter()
            .all(|member| member.ownership == ora_db::PackMemberOwnership::ManagedByPack),
        "every member is pack-managed after the successful retry"
    );
}

// ---- Release qualification: marketplace update, content update, download failures ----

/// Runs one git command against `directory` with an isolated author identity.
fn run_git(directory: &Path, git_config: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-c")
        .arg("user.name=Release Qualification")
        .arg("-c")
        .arg("user.email=qualification@ora.local")
        .args(args)
        .env("GIT_CONFIG_GLOBAL", git_config)
        .env("GIT_CONFIG_SYSTEM", git_config)
        .current_dir(directory)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Writes an orax-shaped zip package with the given files.
fn write_orax_zip(path: &Path, files: &[(&str, &[u8])]) {
    let file = File::create(path).expect("create orax archive");
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    for (name, data) in files {
        writer.start_file(*name, options).expect("start zip entry");
        writer.write_all(data).expect("write zip entry");
    }
    writer.finish().expect("finish zip");
}

/// Builds one skill artifact at an explicit version and returns `(path, sha256_hex)`.
fn build_skill_artifact_v(data_dir: &Path, identifier: &str, version: &str) -> (PathBuf, String) {
    let artifact = data_dir
        .join("artifacts")
        .join(format!("{identifier}-v{version}.orax"));
    fs::create_dir_all(artifact.parent().unwrap()).expect("create artifacts dir");
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"skill\"\nversion = \"{version}\"\ndescription = \"Python skill\"\n"
    );
    write_orax_zip(
        &artifact,
        &[
            ("orax.toml", manifest.as_bytes()),
            (
                "assets/python-core/SKILL.md",
                format!("---\nname: python-core\ndescription: Python skill\n---\n\nBody.\n")
                    .as_bytes(),
            ),
        ],
    );
    let sha = ora_utils::hash::sha256_file(&artifact).expect("hash artifact");
    (artifact, sha)
}

/// Builds one MCP member artifact and returns `(path, sha256_hex)`.
///
/// The transport is HTTP so the fixture installs identically on every host: production validation
/// requires a stdio command inside the package to carry the Unix executable bit, a property this
/// zip fixture cannot express portably from Windows. Pack qualification only proves member
/// installation, marketplace resolution, and journal invariants — not the stdio runtime — so the
/// transport shape is free to be the platform-independent one.
fn build_mcp_artifact_v(data_dir: &Path, identifier: &str, version: &str) -> (PathBuf, String) {
    let artifact = data_dir
        .join("artifacts")
        .join(format!("{identifier}-v{version}.orax"));
    fs::create_dir_all(artifact.parent().unwrap()).expect("create artifacts dir");
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"mcp\"\nversion = \"{version}\"\ndescription = \"Python MCP server\"\n"
    );
    let config =
        br#"{"schemaVersion":1,"transport":{"type":"http","url":"https://example.com/mcp"}}"#;
    write_orax_zip(
        &artifact,
        &[
            ("orax.toml", manifest.as_bytes()),
            ("assets/config.json", config),
        ],
    );
    let sha = ora_utils::hash::sha256_file(&artifact).expect("hash artifact");
    (artifact, sha)
}

/// Stages the supplied marketplace listings into a git origin and clones it into the checkout the
/// seeded official source reads, so production installs resolve without touching the network.
///
/// The listings are `(identifier, orax.toml content)` pairs. `PluginApi::install` resolves release
/// manifests from the checkout directly, so tests that assert install behavior rather than
/// discovery need no `sync_available` round trip.
fn stage_marketplace_checkout(data_dir: &Path, listings: &[(&str, String)]) {
    let origin = data_dir.join("marketplace-origin");
    fs::create_dir_all(origin.join("registry")).expect("create origin registry");
    let git_config = data_dir.join("gitconfig");
    fs::write(&git_config, "").expect("write git config");
    run_git(&origin, &git_config, &["init", "--initial-branch=main"]);
    for (identifier, listing) in listings {
        stage_listing(&origin, identifier, listing);
    }
    run_git(&origin, &git_config, &["add", "."]);
    run_git(
        &origin,
        &git_config,
        &["commit", "-m", "stage marketplace listings"],
    );

    let sources_root = data_dir.join("plugins").join("sources");
    let source = RegistrySource::try_from_git(
        SOURCE_URL,
        ora_domain::PluginNamespace::official(),
        "main",
        &sources_root,
    )
    .expect("derive checkout");
    let checkout = source.checkout_dir().to_path_buf();
    fs::create_dir_all(checkout.parent().unwrap()).expect("create checkout parent");
    run_git(
        &origin,
        &git_config,
        &[
            "clone",
            "--branch",
            "main",
            ".",
            &checkout.to_string_lossy(),
        ],
    );
}

/// Release qualification B: a marketplace member version bump flows through sync → update →
/// old version retirement → new version installation, with no CWD lock residue.
#[tokio::test]
async fn marketplace_member_version_bump_updates_through_production_flow() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const MEMBER_A: &str = "ora-space.python-core";
    const MEMBER_A_ID: &str = "official/ora-space.python-core";

    // Commit A: member at 1.0.0, cloned into the checkout the seeded official source reads.
    let (artifact_v1, sha_v1) = build_skill_artifact_v(data_dir.path(), MEMBER_A, "1.0.0");
    stage_marketplace_checkout(
        data_dir.path(),
        &[(
            MEMBER_A,
            format!(
                "resolver = 1\nidentifier = \"{MEMBER_A}\"\ntitle = \"Python Core\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Python core skill\"\nurl = \"https://example.com/{MEMBER_A}-v1.orax\"\nsha256 = \"{sha_v1}\"\n"
            ),
        )],
    );

    // Install at 1.0.0 through Plugins → PluginApi. The local mapping substitutes only transport;
    // release resolution, verification, finalization, and runtime reconciliation stay on the
    // production entry path.
    host.use_local_marketplace_release(MEMBER_A_ID, artifact_v1);
    plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: MEMBER_A_ID.to_owned(),
        })
        .await
        .expect("install member at 1.0.0");
    assert!(
        member_installed(data_dir.path(), MEMBER_A, "1.0.0"),
        "member installed at 1.0.0"
    );

    // Marketplace publishes 1.1.0.
    let origin = data_dir.path().join("marketplace-origin");
    let git_config = data_dir.path().join("gitconfig");
    let (artifact_v2, sha_v2) = build_skill_artifact_v(data_dir.path(), MEMBER_A, "1.1.0");
    stage_listing(
        &origin,
        MEMBER_A,
        &format!(
            "resolver = 1\nidentifier = \"{MEMBER_A}\"\ntitle = \"Python Core\"\nkind = \"skill\"\nversion = \"1.1.0\"\ndescription = \"Python core skill\"\nurl = \"https://example.com/{MEMBER_A}-v1.1.0.orax\"\nsha256 = \"{sha_v2}\"\n"
        ),
    );
    run_git(&origin, &git_config, &["add", "."]);
    run_git(
        &origin,
        &git_config,
        &["commit", "-m", "publish member-a 1.1.0"],
    );

    // Sync picks up the new version.
    let synced = plugins
        .sync_available(ora_contracts::SyncAvailablePluginsRequest {})
        .expect("sync marketplace");
    let entry = synced
        .plugins
        .iter()
        .find(|p| p.id.contains(MEMBER_A))
        .expect("member still indexed");
    assert_eq!(entry.version, "1.1.0", "sync picks up the new version");

    // Update through Plugins → PluginApi so supervisor suspension/resume and runtime reconcile
    // wrap the same resolved Installer::update path exercised by Desktop.
    host.use_local_marketplace_release(MEMBER_A_ID, artifact_v2);
    plugins
        .update(UpdatePluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: MEMBER_A_ID.to_owned(),
        })
        .await
        .expect("update member to 1.1.0");

    // Old version retired, new version installed.
    assert!(
        !member_installed(data_dir.path(), MEMBER_A, "1.0.0"),
        "old version directory is retired"
    );
    assert!(
        member_installed(data_dir.path(), MEMBER_A, "1.1.0"),
        "new version directory is installed"
    );
    let listed = plugins
        .list_installed(ListInstalledPluginsRequest {})
        .expect("list installed plugins");
    assert_eq!(
        listed
            .plugins
            .iter()
            .find(|plugin| plugin.id == MEMBER_A_ID)
            .expect("member listed after update")
            .version,
        "1.1.0",
        "the reconciled surface reports the new version"
    );
    // A standalone member update must not touch the pack ownership journal.
    let ownership_repository = SqlitePackInstallationRepository::new(pool.clone());
    assert_eq!(
        ownership_repository
            .load(PACK_ID)
            .expect("load the pack ledger"),
        None,
        "a standalone update creates no pack ownership"
    );
}

/// Release qualification C: adding a new member to the pack manifest is picked up on sync,
/// the new member is resolvable, and the ownership journal is not rewritten.
#[tokio::test]
async fn pack_manifest_content_update_adds_new_resolvable_member() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    let namespace = ora_domain::PluginNamespace::official();

    let origin = data_dir.path().join("marketplace-origin");
    fs::create_dir_all(origin.join("registry")).expect("create origin registry");
    let git_config = data_dir.path().join("gitconfig");
    fs::write(&git_config, "").expect("write git config");
    run_git(&origin, &git_config, &["init", "--initial-branch=main"]);

    // Commit A: pack with two members backed by real packages.
    let (core_artifact, core_sha) =
        build_skill_artifact_v(data_dir.path(), "ora-space.python-core", "1.0.0");
    let (mcp_artifact, mcp_sha) =
        build_mcp_artifact_v(data_dir.path(), "ora-space.python-mcp", "1.0.0");
    stage_listing(
        &origin,
        "ora-space.python-core",
        &skill_listing("ora-space.python-core", &core_sha, false),
    );
    stage_listing(
        &origin,
        "ora-space.python-mcp",
        &format!(
            "resolver = 1\nidentifier = \"ora-space.python-mcp\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"Python MCP\"\nurl = \"https://example.com/python-mcp.orax\"\nsha256 = \"{}\"\n",
            mcp_sha
        ),
    );
    stage_listing(
        &origin,
        PACK_IDENTIFIER,
        &pack_listing(
            PACK_IDENTIFIER,
            &format!(
                "{}{}",
                member_table("ora-space.python-core"),
                member_table("ora-space.python-mcp")
            ),
        ),
    );
    run_git(&origin, &git_config, &["add", "."]);
    run_git(
        &origin,
        &git_config,
        &["commit", "-m", "pack with two members"],
    );

    // Stage the checkout so sync does a local git fetch/pull instead of a network clone.
    let sources_root = data_dir.path().join("plugins").join("sources");
    let marketplace_source =
        RegistrySource::try_from_git(SOURCE_URL, namespace.clone(), "main", &sources_root)
            .expect("derive checkout");
    let checkout = marketplace_source.checkout_dir().to_path_buf();
    fs::create_dir_all(checkout.parent().unwrap()).expect("create checkout parent");
    run_git(
        &origin,
        &git_config,
        &[
            "clone",
            "--branch",
            "main",
            ".",
            &checkout.to_string_lossy(),
        ],
    );

    // Sync A: indexes the initial two members.
    let synced_a = plugins
        .sync_available(ora_contracts::SyncAvailablePluginsRequest {})
        .expect("sync A");
    assert_eq!(synced_a.plugins.len(), 2, "two members indexed");

    host.use_local_marketplace_release("official/ora-space.python-core", core_artifact);
    host.use_local_marketplace_release("official/ora-space.python-mcp", mcp_artifact);
    let installed = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_owned(),
        })
        .await
        .expect("install pack V1 through production entry");
    assert!(
        matches!(
            installed.outcome,
            InstallOutcome::PackInstalled { failed: None, .. }
        ),
        "pack V1 installs before the marketplace content changes: {installed:?}"
    );
    let ownership_repository = SqlitePackInstallationRepository::new(pool.clone());
    let ownership_before = ownership_repository
        .load(PACK_ID)
        .expect("load V1 ownership")
        .expect("V1 ownership exists");

    // Marketplace adds a third member to the pack manifest.
    let (_lint_artifact, lint_sha) =
        build_skill_artifact_v(data_dir.path(), "ora-space.python-lint", "1.0.0");
    stage_listing(
        &origin,
        "ora-space.python-lint",
        &format!(
            "resolver = 1\nidentifier = \"ora-space.python-lint\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Python lint skill\"\nurl = \"https://example.com/python-lint.orax\"\nsha256 = \"{}\"\n",
            lint_sha
        ),
    );
    stage_listing(
        &origin,
        PACK_IDENTIFIER,
        &pack_listing(
            PACK_IDENTIFIER,
            &format!(
                "{}{}{}",
                member_table("ora-space.python-core"),
                member_table("ora-space.python-mcp"),
                member_table("ora-space.python-lint")
            ),
        ),
    );
    run_git(&origin, &git_config, &["add", "."]);
    run_git(
        &origin,
        &git_config,
        &["commit", "-m", "add lint member to pack"],
    );

    // Sync B picks up the new member.
    let synced_b = plugins
        .sync_available(ora_contracts::SyncAvailablePluginsRequest {})
        .expect("sync B");
    assert_eq!(
        synced_b.plugins.len(),
        3,
        "three members indexed after update"
    );

    // The current source resolves the V2 pack, its new visible member, and the hidden member.
    let pack_v2 = ora_plugin_registry::RegistryIndex::resolve_manifest(
        &marketplace_source,
        &ora_domain::PluginId::parse(PACK_ID).expect("pack id"),
    )
    .expect("resolve Pack V2")
    .expect("Pack V2 exists");
    assert_eq!(pack_v2.kind(), PluginKind::Pack);
    assert_eq!(
        pack_v2
            .pack()
            .expect("Pack V2 membership")
            .members()
            .iter()
            .map(|member| member.identifier().as_str())
            .collect::<Vec<_>>(),
        vec![
            "ora-space.python-core",
            "ora-space.python-mcp",
            "ora-space.python-lint",
        ]
    );
    for member_id in [
        "official/ora-space.python-core",
        "official/ora-space.python-lint",
    ] {
        assert!(
            ora_plugin_registry::RegistryIndex::resolve_manifest(
                &marketplace_source,
                &ora_domain::PluginId::parse(member_id).expect("member id"),
            )
            .expect("resolve member")
            .is_some(),
            "{member_id} remains resolvable from the source checkout"
        );
    }
    assert!(
        synced_b
            .plugins
            .iter()
            .all(|plugin| plugin.id != "official/ora-space.python-core"),
        "the hidden member stays absent from marketplace discovery"
    );
    assert!(
        synced_b
            .plugins
            .iter()
            .any(|plugin| plugin.id == "official/ora-space.python-lint"),
        "the new visible member is discoverable"
    );

    // Sync may change current declarations, but it must not rewrite historical ownership.
    let ownership_after = ownership_repository
        .load(PACK_ID)
        .expect("reload ownership after sync")
        .expect("ownership survives sync");
    assert_eq!(ownership_after, ownership_before);
}

/// Release qualification D: a member artifact that does not match its declared SHA-256 fails
/// inside the production pack install entry, the atomic install never commits, and the ownership
/// journal gains no phantom relation.
#[tokio::test]
async fn sha256_mismatch_aborts_install_without_phantom_ownership() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const MEMBER: &str = "ora-space.bad-sha";
    const MEMBER_ID: &str = "official/ora-space.bad-sha";

    // The artifact is real, but the listing declares a WRONG sha256: the transfer succeeds and
    // the digest verification must abort the commit.
    let (artifact, _real_sha) = build_skill_artifact_v(data_dir.path(), MEMBER, "1.0.0");
    let wrong_sha = "ff".repeat(32);
    stage_marketplace_checkout(
        data_dir.path(),
        &[
            (
                MEMBER,
                skill_listing(MEMBER, &wrong_sha, /*marketplace_visible*/ false),
            ),
            (
                PACK_IDENTIFIER,
                pack_listing(PACK_IDENTIFIER, &member_table(MEMBER)),
            ),
        ],
    );

    host.use_local_marketplace_release(MEMBER_ID, artifact);
    let installed = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_owned(),
        })
        .await
        .expect("the digest failure is reported inside the pack outcome");
    assert_eq!(
        installed.outcome,
        InstallOutcome::PackInstalled {
            members: Vec::new(),
            skipped: Vec::new(),
            failed: Some(PackInstallFailure {
                plugin_id: MEMBER_ID.to_owned(),
                error_code: "plugin_package_invalid".to_owned(),
                rollback_failures: Vec::new(),
            }),
        }
    );
    assert!(
        !member_installed(data_dir.path(), MEMBER, "1.0.0"),
        "the atomic install is not committed on a digest mismatch"
    );

    // Ownership is read from the repository, not inferred from the filesystem: a pack that never
    // completed records neither a root nor a member relation.
    let ownership_repository = SqlitePackInstallationRepository::new(pool.clone());
    assert_eq!(
        ownership_repository
            .load(PACK_ID)
            .expect("load the pack ledger"),
        None,
        "a failed pack install records no pack_installation root"
    );
    assert_eq!(
        ownership_repository
            .load_member(PACK_ID, MEMBER_ID)
            .expect("load the member relation"),
        None,
        "a failed pack install records no pack_installation_member relation"
    );
}

/// Release qualification D (cont.): a member whose artifact cannot be transferred fails inside
/// the production pack install entry after an earlier member already landed; the rollback removes
/// what the run created, and the ownership ledger ends where it started.
#[tokio::test]
async fn missing_artifact_aborts_install_without_creating_directory() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const PRESENT_MEMBER: &str = "ora-space.present-member";
    const PRESENT_MEMBER_ID: &str = "official/ora-space.present-member";
    const MISSING_MEMBER: &str = "ora-space.missing-artifact";
    const MISSING_MEMBER_ID: &str = "official/ora-space.missing-artifact";

    let (present_artifact, present_sha) =
        build_skill_artifact_v(data_dir.path(), PRESENT_MEMBER, "1.0.0");
    let missing_artifact = data_dir.path().join("nonexistent.orax");
    stage_marketplace_checkout(
        data_dir.path(),
        &[
            (
                PRESENT_MEMBER,
                skill_listing(
                    PRESENT_MEMBER,
                    &present_sha,
                    /*marketplace_visible*/ false,
                ),
            ),
            (
                MISSING_MEMBER,
                skill_listing(MISSING_MEMBER, "ab".repeat(32).as_str(), false),
            ),
            (
                PACK_IDENTIFIER,
                pack_listing(
                    PACK_IDENTIFIER,
                    &format!(
                        "{}{}",
                        member_table(PRESENT_MEMBER),
                        member_table(MISSING_MEMBER)
                    ),
                ),
            ),
        ],
    );

    // The first member transfers; the second maps to a path that does not exist, so its transfer
    // fails inside the production install loop.
    host.use_local_marketplace_release(PRESENT_MEMBER_ID, present_artifact);
    host.use_local_marketplace_release(MISSING_MEMBER_ID, missing_artifact);
    let installed = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_owned(),
        })
        .await
        .expect("the transfer failure is reported inside the pack outcome");
    assert_eq!(
        installed.outcome,
        InstallOutcome::PackInstalled {
            members: Vec::new(),
            skipped: Vec::new(),
            failed: Some(PackInstallFailure {
                plugin_id: MISSING_MEMBER_ID.to_owned(),
                error_code: "internal_error".to_owned(),
                rollback_failures: Vec::new(),
            }),
        }
    );
    assert!(
        !member_installed(data_dir.path(), PRESENT_MEMBER, "1.0.0"),
        "the member created before the failure is rolled back"
    );
    assert!(
        !member_installed(data_dir.path(), MISSING_MEMBER, "1.0.0"),
        "the failed member never lands"
    );

    // Complete rollback: the ledger after the operation equals the ledger before it (empty), read
    // directly from the repository rather than inferred from missing directories.
    let ownership_repository = SqlitePackInstallationRepository::new(pool.clone());
    assert_eq!(
        ownership_repository
            .load(PACK_ID)
            .expect("load the pack ledger"),
        None,
        "a completely rolled-back pack install leaves no pack_installation root"
    );
    assert_eq!(
        ownership_repository
            .load_member(PACK_ID, PRESENT_MEMBER_ID)
            .expect("load the rolled-back member relation"),
        None,
        "the rolled-back member keeps no pack_installation_member relation"
    );
    assert_eq!(
        ownership_repository
            .load_member(PACK_ID, MISSING_MEMBER_ID)
            .expect("load the failed member relation"),
        None,
        "the failed member gains no phantom pack_installation_member relation"
    );
}

// ---- Release qualification: package content validation ----

/// A marketplace package whose in-package manifest omits a required field is refused with the
/// public `plugin_package_invalid` error naming that field, and nothing is committed.
///
/// Before the installer validated the package's own manifest, this archive installed
/// "successfully", failed later during discovery, and left the bad package on disk.
#[tokio::test]
async fn incomplete_package_manifest_is_reported_as_a_package_error() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const MEMBER: &str = "ora-space.incomplete";
    const MEMBER_ID: &str = "official/ora-space.incomplete";

    // The archive is a real, well-formed package whose manifest omits `description`.
    let artifact = data_dir.path().join("incomplete.orax");
    write_orax_zip(
        &artifact,
        &[
            (
                "orax.toml",
                b"resolver = 1\nidentifier = \"ora-space.incomplete\"\nkind = \"skill\"\nversion = \"1.0.0\"\n"
                    .as_slice(),
            ),
            (
                "assets/demo/SKILL.md",
                b"---\nname: demo\ndescription: Demo\n---\n\nBody.\n".as_slice(),
            ),
        ],
    );
    let sha = ora_utils::hash::sha256_file(&artifact).expect("hash artifact");
    stage_marketplace_checkout(
        data_dir.path(),
        &[(
            MEMBER,
            skill_listing(MEMBER, &sha, /*marketplace_visible*/ true),
        )],
    );
    host.use_local_marketplace_release(MEMBER_ID, artifact);

    let error = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: MEMBER_ID.to_owned(),
        })
        .await
        .expect_err("a package missing a required manifest field is refused");

    assert_eq!(error.classification(), ErrorClassification::Unprocessable);
    assert_eq!(
        error.public_error(),
        &PublicError::PluginPackageInvalid(PluginPackageInvalidParams {
            field: "description".to_owned(),
            message: "missing field `description`".to_owned(),
        })
    );
    assert!(
        !member_installed(data_dir.path(), MEMBER, "1.0.0"),
        "nothing is committed for a rejected package"
    );
}

/// A marketplace package whose in-package manifest is not valid TOML is refused with
/// `plugin_package_invalid` naming the manifest file itself, because a syntax error belongs to no
/// single field, and nothing is committed.
#[tokio::test]
async fn malformed_package_manifest_is_reported_against_the_manifest_file() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const MEMBER: &str = "ora-space.malformed";
    const MEMBER_ID: &str = "official/ora-space.malformed";

    // The `version` value is missing, so the document does not parse as TOML at all.
    let artifact = data_dir.path().join("malformed.orax");
    write_orax_zip(
        &artifact,
        &[
            (
                "orax.toml",
                b"resolver = 1\nidentifier = \"ora-space.malformed\"\nkind = \"skill\"\nversion = \ndescription = \"Demo\"\n"
                    .as_slice(),
            ),
            (
                "assets/demo/SKILL.md",
                b"---\nname: demo\ndescription: Demo\n---\n\nBody.\n".as_slice(),
            ),
        ],
    );
    let sha = ora_utils::hash::sha256_file(&artifact).expect("hash artifact");
    stage_marketplace_checkout(
        data_dir.path(),
        &[(
            MEMBER,
            skill_listing(MEMBER, &sha, /*marketplace_visible*/ true),
        )],
    );
    host.use_local_marketplace_release(MEMBER_ID, artifact);

    let error = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: MEMBER_ID.to_owned(),
        })
        .await
        .expect_err("a package whose manifest is not valid TOML is refused");

    assert_eq!(error.classification(), ErrorClassification::Unprocessable);
    assert_eq!(
        error.public_error(),
        &PublicError::PluginPackageInvalid(PluginPackageInvalidParams {
            field: "orax.toml".to_owned(),
            message: "string values must be quoted, expected literal string".to_owned(),
        })
    );
    assert!(
        !member_installed(data_dir.path(), MEMBER, "1.0.0"),
        "nothing is committed for a rejected package"
    );
}

/// A package whose own manifest contradicts the listing's identity is refused, so the installed
/// directory can never disagree with the manifest discovery reads back from it.
#[tokio::test]
async fn package_identity_mismatch_is_reported_as_a_package_error() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const MEMBER: &str = "ora-space.version-drift";
    const MEMBER_ID: &str = "official/ora-space.version-drift";

    // The listing publishes 1.0.0, while the archive's own manifest says 1.1.0.
    let (artifact, sha) = build_skill_artifact_v(data_dir.path(), MEMBER, "1.1.0");
    stage_marketplace_checkout(
        data_dir.path(),
        &[(
            MEMBER,
            skill_listing(MEMBER, &sha, /*marketplace_visible*/ true),
        )],
    );
    host.use_local_marketplace_release(MEMBER_ID, artifact);

    let error = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: MEMBER_ID.to_owned(),
        })
        .await
        .expect_err("a package contradicting the listing identity is refused");

    assert_eq!(error.classification(), ErrorClassification::Unprocessable);
    assert_eq!(
        error.public_error(),
        &PublicError::PluginPackageInvalid(PluginPackageInvalidParams {
            field: "version".to_owned(),
            message:
                "package version `1.1.0` does not match the marketplace listing version `1.0.0`"
                    .to_owned(),
        })
    );
    assert!(
        !member_installed(data_dir.path(), MEMBER, "1.0.0")
            && !member_installed(data_dir.path(), MEMBER, "1.1.0"),
        "neither the listing's version nor the package's own version is committed"
    );
}

/// A local import of a package whose manifest omits a required field is refused with the same
/// public error a marketplace install reports, so both entry points agree on what a user can fix
/// instead of the import path quoting a request ID for the same defect.
#[tokio::test]
async fn importing_an_invalid_package_is_reported_as_a_package_error() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, _host) = pack_test_plugins(data_dir.path(), &pool);
    const IDENTIFIER: &str = "ora-space.incomplete-local";

    // A real skill package whose in-archive manifest omits `description`.
    let artifact = data_dir.path().join("incomplete-local.orax");
    write_orax_zip(
        &artifact,
        &[
            (
                "orax.toml",
                format!(
                    "resolver = 1\nidentifier = \"{IDENTIFIER}\"\nkind = \"skill\"\nversion = \"1.0.0\"\n"
                )
                .as_bytes(),
            ),
            (
                "assets/demo/SKILL.md",
                b"---\nname: demo\ndescription: Demo\n---\n\nBody.\n",
            ),
        ],
    );

    let error = plugins
        .import(ImportPluginRequest {
            hook_execution_acknowledged: false,
            path: artifact.to_string_lossy().into_owned(),
        })
        .await
        .expect_err("an imported package missing a required manifest field is refused");

    assert_eq!(error.classification(), ErrorClassification::Unprocessable);
    assert_eq!(
        error.public_error(),
        &PublicError::PluginPackageInvalid(PluginPackageInvalidParams {
            field: "description".to_owned(),
            message: "missing field `description`".to_owned(),
        })
    );
    assert!(
        !data_dir
            .path()
            .join("plugins")
            .join("installed")
            .join("local")
            .join(IDENTIFIER)
            .exists(),
        "nothing is committed under the local namespace for a rejected import"
    );
}

/// An update whose newly published package is invalid is refused with the field it is missing
/// while the installed version stays untouched, and the failed transfer leaves no cached
/// archive: the update path shares the install-time validation and cleanup.
#[tokio::test]
async fn updating_to_an_invalid_package_keeps_the_installed_version() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const MEMBER: &str = "ora-space.broken-update";
    const MEMBER_ID: &str = "official/ora-space.broken-update";

    // Install a working 1.0.0 through the production entry.
    let (artifact_v1, sha_v1) = build_skill_artifact_v(data_dir.path(), MEMBER, "1.0.0");
    stage_marketplace_checkout(
        data_dir.path(),
        &[(
            MEMBER,
            format!(
                "resolver = 1\nidentifier = \"{MEMBER}\"\ntitle = \"Broken Update\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Skill\"\nurl = \"https://example.com/{MEMBER}-v1.0.0.orax\"\nsha256 = \"{sha_v1}\"\n"
            ),
        )],
    );
    host.use_local_marketplace_release(MEMBER_ID, artifact_v1);
    plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: MEMBER_ID.to_owned(),
        })
        .await
        .expect("install member at 1.0.0");

    // The marketplace publishes 1.1.0 whose archive omits `description` in its own manifest.
    let artifact_v2 = data_dir
        .path()
        .join("artifacts")
        .join(format!("{MEMBER}-v1.1.0.orax"));
    write_orax_zip(
        &artifact_v2,
        &[
            (
                "orax.toml",
                format!(
                    "resolver = 1\nidentifier = \"{MEMBER}\"\nkind = \"skill\"\nversion = \"1.1.0\"\n"
                )
                .as_bytes(),
            ),
            (
                "assets/python-core/SKILL.md",
                b"---\nname: python-core\ndescription: Skill\n---\n\nBody.\n",
            ),
        ],
    );
    let sha_v2 = ora_utils::hash::sha256_file(&artifact_v2).expect("hash artifact");
    let origin = data_dir.path().join("marketplace-origin");
    let git_config = data_dir.path().join("gitconfig");
    stage_listing(
        &origin,
        MEMBER,
        &format!(
            "resolver = 1\nidentifier = \"{MEMBER}\"\ntitle = \"Broken Update\"\nkind = \"skill\"\nversion = \"1.1.0\"\ndescription = \"Skill\"\nurl = \"https://example.com/{MEMBER}-v1.1.0.orax\"\nsha256 = \"{sha_v2}\"\n"
        ),
    );
    run_git(&origin, &git_config, &["add", "."]);
    run_git(
        &origin,
        &git_config,
        &["commit", "-m", "publish a broken 1.1.0"],
    );
    plugins
        .sync_available(ora_contracts::SyncAvailablePluginsRequest {})
        .expect("sync marketplace");
    host.use_local_marketplace_release(MEMBER_ID, artifact_v2);

    let error = plugins
        .update(UpdatePluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: MEMBER_ID.to_owned(),
        })
        .await
        .expect_err("an update to an invalid package is refused");

    assert_eq!(error.classification(), ErrorClassification::Unprocessable);
    assert_eq!(
        error.public_error(),
        &PublicError::PluginPackageInvalid(PluginPackageInvalidParams {
            field: "description".to_owned(),
            message: "missing field `description`".to_owned(),
        })
    );
    assert!(
        member_installed(data_dir.path(), MEMBER, "1.0.0"),
        "the working installation survives a refused update"
    );
    assert!(
        !member_installed(data_dir.path(), MEMBER, "1.1.0"),
        "the broken release is never committed"
    );
    assert!(
        !data_dir
            .path()
            .join("plugins")
            .join("cache")
            .join("official")
            .join(format!("{MEMBER}-1.1.0.orax"))
            .exists(),
        "the failed update leaves no cached archive"
    );
}

// ---- Hook lifecycle execution: authorization, initialization, and removal ----

const HOOK_MEMBER: &str = "rtk-ai.rtk";
const HOOK_ID: &str = "official/rtk-ai.rtk";

/// One lifecycle command the host resolved, with what it would have run it against.
#[derive(Debug, PartialEq, Eq)]
struct HookCall {
    program: PathBuf,
    args: Vec<String>,
    working_directory: PathBuf,
    /// Whether the package's executable still existed when the command was asked to run: removing
    /// a package must execute its `deinit` before the file the command needs is gone.
    executable_present: bool,
}

/// Answers each Hook lifecycle command with the next scripted exit code and records the calls.
///
/// A lifecycle command is the package's own program, so a test that drives the real install,
/// initialization, and uninstall operations substitutes only the process boundary: what is under
/// test is which command the host decided to run, when, and on whose authorization.
struct ScriptedHookRunner {
    exit_codes: Mutex<VecDeque<i32>>,
    calls: Mutex<Vec<HookCall>>,
}

impl ScriptedHookRunner {
    /// Builds a runner that answers the commands with `exit_codes`, in order.
    fn new(exit_codes: impl IntoIterator<Item = i32>) -> Self {
        Self {
            exit_codes: Mutex::new(exit_codes.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Returns the calls recorded so far, in order.
    fn calls(&self) -> Vec<HookCall> {
        std::mem::take(&mut *self.calls.lock().expect("calls lock"))
    }
}

impl HookCommandRunner for Arc<ScriptedHookRunner> {
    /// Records the resolved command and answers with the next scripted exit code.
    async fn run(&self, spec: &HookCommandSpec) -> Result<HookCommandExecution, HookCommandError> {
        let exit_code = self
            .exit_codes
            .lock()
            .expect("exit codes lock")
            .pop_front()
            .expect("a scripted exit code for every lifecycle command");
        self.calls.lock().expect("calls lock").push(HookCall {
            executable_present: spec.program.is_file(),
            program: spec.program.clone(),
            args: spec.args.clone(),
            working_directory: spec.working_directory.clone(),
        });
        Ok(HookCommandExecution {
            finished: HookCommandFinished::Exited {
                code: Some(exit_code),
            },
            stderr: HookCommandOutput::default(),
        })
    }
}

/// Returns the host triple a Hook package has to declare, or `None` on an unsupported host.
///
/// A Hook package carries a native executable and self-declares the host triple it was built for,
/// so a case that installs one needs a host the plugin registry supports; nothing else in these
/// cases depends on the host.
fn hook_host_target() -> Option<ora_plugin_manifest::HookTarget> {
    let target = ora_plugin_registry::current_host_target();
    if target.is_none() {
        eprintln!("skipping Hook install case: compiled host is not a plugin target");
    }
    target
}

/// Writes one Hook archive for `target` under the data directory and returns it with its digest.
fn write_hook_artifact(data_dir: &Path, identifier: &str, target: &str) -> (PathBuf, String) {
    let artifact = data_dir
        .join("artifacts")
        .join(format!("{identifier}.orax"));
    fs::create_dir_all(artifact.parent().expect("artifacts directory")).expect("create artifacts");
    let sha256 = write_hook_orax(&artifact, identifier, Some(target));
    (artifact, sha256)
}

/// Stages one Hook release in the marketplace checkout and returns its artifact path.
///
/// The listing declares a universal release — a URL and a digest, no `[[targets]]` — so resolution
/// never enters the target-selecting path; the archive itself still self-declares the host triple
/// an installed Hook must carry.
fn stage_hook_release(data_dir: &Path, identifier: &str, target: &str) -> PathBuf {
    let (artifact, sha256) = write_hook_artifact(data_dir, identifier, target);
    stage_marketplace_checkout(data_dir, &[(identifier, hook_listing(identifier, &sha256))]);
    artifact
}

/// Installs one staged Hook release by its canonical id.
async fn install_hook(
    plugins: &Plugins,
    host: &PluginApi,
    artifact: PathBuf,
    acknowledged: bool,
) -> ora_contracts::InstallPluginResponse {
    host.use_local_marketplace_release(HOOK_ID, artifact);
    plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: acknowledged,
            plugin_id: HOOK_ID.to_owned(),
        })
        .await
        .expect("install the Hook")
}

/// Returns the canonical path the host resolves for one installed member's declared executable.
fn installed_executable(root: &Path, member: &str, version: &str) -> PathBuf {
    root.join("plugins")
        .join("installed")
        .join("official")
        .join(member)
        .join(version)
        .join("assets")
        .join("rtk.exe")
        .canonicalize()
        .expect("canonical installed executable")
}

/// Returns this session's Hook lifecycle reports, in identifier order.
fn hook_reports(plugins: &Plugins) -> Vec<ora_contracts::HookLifecycleReport> {
    plugins
        .list_hook_lifecycle_reports(ListHookLifecycleReportsRequest {})
        .expect("read this session's Hook lifecycle reports")
        .reports
}

/// An install that declares no authorization lands the package and runs nothing: laying a package
/// down and running a program it ships are separate authorizations.
#[tokio::test]
async fn installing_a_hook_without_authorization_runs_nothing() {
    let _trace = trace_guard();
    let Some(target) = hook_host_target() else {
        return;
    };
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    let artifact = stage_hook_release(data_dir.path(), HOOK_MEMBER, target.as_str());
    let runner = Arc::new(ScriptedHookRunner::new([0]));
    host.install_hook_command_runner(Arc::clone(&runner));

    let installed = install_hook(&plugins, &host, artifact, /*acknowledged*/ false).await;

    assert_eq!(
        installed,
        ora_contracts::InstallPluginResponse {
            plugin_id: HOOK_ID.to_owned(),
            outcome: InstallOutcome::Installed,
            workflows: Vec::new(),
        }
    );
    assert!(
        member_installed(data_dir.path(), HOOK_MEMBER, "0.1.0"),
        "the package lands as an ordinary install"
    );
    assert_eq!(
        runner.calls(),
        Vec::new(),
        "the package's own program never runs without the authorization that names it"
    );
    assert_eq!(
        hook_reports(&plugins),
        Vec::new(),
        "nothing ran, so there is no lifecycle result to report"
    );
}

/// An authorized install runs the declared `init` exactly once and reports the result, which is
/// what the settings surface shows next to the Hook.
#[tokio::test]
async fn an_authorized_install_runs_init_once() {
    let _trace = trace_guard();
    let Some(target) = hook_host_target() else {
        return;
    };
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    let artifact = stage_hook_release(data_dir.path(), HOOK_MEMBER, target.as_str());
    let runner = Arc::new(ScriptedHookRunner::new([0]));
    host.install_hook_command_runner(Arc::clone(&runner));

    install_hook(&plugins, &host, artifact, /*acknowledged*/ true).await;

    assert_eq!(
        runner.calls(),
        vec![HookCall {
            program: installed_executable(data_dir.path(), HOOK_MEMBER, "0.1.0"),
            args: vec!["--init".to_owned()],
            working_directory: data_dir.path().to_path_buf(),
            executable_present: true,
        }],
        "the install runs `init` once from the installed package"
    );
}

/// A failed `init` does not undo the install: the package stays installed and visible, and the
/// explicit initialization the user asks for afterwards replaces the failed result.
#[tokio::test]
async fn a_failed_init_keeps_the_hook_installed_and_retryable() {
    let _trace = trace_guard();
    let Some(target) = hook_host_target() else {
        return;
    };
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    let artifact = stage_hook_release(data_dir.path(), HOOK_MEMBER, target.as_str());
    let runner = Arc::new(ScriptedHookRunner::new([3, 0]));
    host.install_hook_command_runner(Arc::clone(&runner));

    install_hook(&plugins, &host, artifact, /*acknowledged*/ true).await;

    let listed = plugins
        .list_installed(ListInstalledPluginsRequest {})
        .expect("installed snapshot");
    assert!(
        listed.plugins.iter().any(|plugin| plugin.id == HOOK_ID),
        "a failed lifecycle command keeps the package installed"
    );
    let failed = hook_reports(&plugins);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].plugin_id, HOOK_ID);
    assert_eq!(failed[0].phase, HookLifecyclePhase::Init);
    assert!(
        matches!(
            failed[0].outcome,
            HookLifecycleOutcome::Failed {
                exit_code: Some(3),
                ..
            }
        ),
        "the failure names the exit code the tool reported, got {:?}",
        failed[0].outcome
    );

    let retried = plugins
        .initialize_hook(InitializeHookRequest {
            plugin_id: HOOK_ID.to_owned(),
            hook_execution_acknowledged: true,
        })
        .await
        .expect("retry the initialization");

    assert!(
        matches!(
            retried.report.outcome,
            HookLifecycleOutcome::Succeeded { .. }
        ),
        "the retry runs the same command again, got {:?}",
        retried.report.outcome
    );
    assert_eq!(
        hook_reports(&plugins),
        vec![retried.report],
        "the retry replaces the failed result instead of accumulating history"
    );
}

/// The explicit initialization is gated by the same authorization as the automatic one, so a
/// request that declares none is refused rather than treated as consent.
#[tokio::test]
async fn initializing_a_hook_without_authorization_is_refused() {
    let _trace = trace_guard();
    let Some(target) = hook_host_target() else {
        return;
    };
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    let artifact = stage_hook_release(data_dir.path(), HOOK_MEMBER, target.as_str());
    let runner = Arc::new(ScriptedHookRunner::new([0]));
    host.install_hook_command_runner(Arc::clone(&runner));
    install_hook(&plugins, &host, artifact, /*acknowledged*/ true).await;

    let error = plugins
        .initialize_hook(InitializeHookRequest {
            plugin_id: HOOK_ID.to_owned(),
            hook_execution_acknowledged: false,
        })
        .await
        .expect_err("an unacknowledged initialization is refused");

    assert!(matches!(
        error.public_error(),
        PublicError::InvalidRequest(_)
    ));
    assert_eq!(
        runner.calls().len(),
        1,
        "only the authorized install ran a command"
    );
}

/// An authorized uninstall runs the declared `deinit` while the package is still on disk, because
/// a lifecycle command is the package's own program and cannot run once it has been removed.
#[tokio::test]
async fn uninstall_runs_deinit_before_the_package_is_removed() {
    let _trace = trace_guard();
    let Some(target) = hook_host_target() else {
        return;
    };
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    let artifact = stage_hook_release(data_dir.path(), HOOK_MEMBER, target.as_str());
    let runner = Arc::new(ScriptedHookRunner::new([0, 0]));
    host.install_hook_command_runner(Arc::clone(&runner));
    install_hook(&plugins, &host, artifact, /*acknowledged*/ true).await;

    plugins
        .uninstall(UninstallPluginRequest {
            hook_execution_acknowledged: true,
            plugin_id: HOOK_ID.to_owned(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .expect("uninstall the Hook");

    let calls = runner.calls();
    assert_eq!(
        calls.len(),
        2,
        "the install ran `init` and the uninstall ran `deinit`"
    );
    assert_eq!(calls[1].args, vec!["--deinit".to_owned()]);
    assert!(
        calls[1].executable_present,
        "`deinit` runs while the package's executable is still in place"
    );
    assert!(
        !member_installed(data_dir.path(), HOOK_MEMBER, "0.1.0"),
        "the package is removed once its removal command has run"
    );
}

/// An uninstall that declares no authorization removes the package without running its program.
#[tokio::test]
async fn uninstall_without_authorization_runs_nothing() {
    let _trace = trace_guard();
    let Some(target) = hook_host_target() else {
        return;
    };
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    let artifact = stage_hook_release(data_dir.path(), HOOK_MEMBER, target.as_str());
    let runner = Arc::new(ScriptedHookRunner::new([0]));
    host.install_hook_command_runner(Arc::clone(&runner));
    install_hook(&plugins, &host, artifact, /*acknowledged*/ true).await;

    plugins
        .uninstall(UninstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: HOOK_ID.to_owned(),
            data_disposition: PluginDataDisposition::Delete,
        })
        .await
        .expect("uninstall the Hook");

    assert_eq!(
        runner.calls().len(),
        1,
        "nothing runs on removal beyond the install's own `init`"
    );
    assert!(
        !member_installed(data_dir.path(), HOOK_MEMBER, "0.1.0"),
        "the package is removed either way"
    );
}

/// A pack install lands its Hook member without executing it, and that member is initialized only
/// once the user authorizes the member itself: the pack's authorization covers the pack, not the
/// programs its members ship.
#[tokio::test]
async fn a_pack_install_does_not_run_a_hook_member() {
    let _trace = trace_guard();
    let Some(target) = hook_host_target() else {
        return;
    };
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    let member_id = format!("official/{HOOK_MEMBER}");
    let (member_artifact, sha256) =
        write_hook_artifact(data_dir.path(), HOOK_MEMBER, target.as_str());
    stage_marketplace_checkout(
        data_dir.path(),
        &[
            (HOOK_MEMBER, hook_listing(HOOK_MEMBER, &sha256)),
            (
                PACK_IDENTIFIER,
                pack_listing(PACK_IDENTIFIER, &member_table(HOOK_MEMBER)),
            ),
        ],
    );
    let runner = Arc::new(ScriptedHookRunner::new([0]));
    host.install_hook_command_runner(Arc::clone(&runner));
    host.use_local_marketplace_release(&member_id, member_artifact);

    let installed = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: true,
            plugin_id: PACK_ID.to_owned(),
        })
        .await
        .expect("install the pack");

    assert_eq!(
        installed.outcome,
        InstallOutcome::PackInstalled {
            members: vec![member_id.clone()],
            skipped: Vec::new(),
            failed: None,
        },
        "the pack lands its Hook member like any other member"
    );
    assert!(member_installed(data_dir.path(), HOOK_MEMBER, "0.1.0"));
    assert_eq!(
        runner.calls(),
        Vec::new(),
        "an authorized pack install still runs nothing its members ship"
    );
    assert_eq!(
        hook_reports(&plugins),
        Vec::new(),
        "an unexecuted member reports no lifecycle result, which is how the surface shows it as not initialized"
    );

    let initialized = plugins
        .initialize_hook(InitializeHookRequest {
            plugin_id: member_id,
            hook_execution_acknowledged: true,
        })
        .await
        .expect("initialize the member the user authorized");

    assert_eq!(initialized.report.phase, HookLifecyclePhase::Init);
    assert_eq!(
        runner.calls().len(),
        1,
        "the member's own authorization is what runs its program"
    );
}

/// A second Start-only workflow document, distinct from [`WORKFLOW_DOCUMENT`] by name and version.
const SECOND_WORKFLOW_DOCUMENT: &str = r#"{"name":"发布流程","version":"2.0.0","viewport":{"x":0,"y":0,"zoom":1},"nodes":[{"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","title":"开始"}}],"edges":[]}"#;

/// Builds one marketplace listing for a Workflow package at `version` with `sha`.
fn workflow_listing(identifier: &str, version: &str, sha: &str) -> String {
    format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"workflow\"\nversion = \"{version}\"\ndescription = \"Workflow package\"\nurl = \"https://example.com/{identifier}-{version}.orax\"\nsha256 = \"{sha}\"\n"
    )
}

/// Lists the local workflow library's names, sorted so the assertion ignores creation order.
fn listed_workflow_names(pool: &RepositoryPool) -> Vec<String> {
    let listed = crate::workflow::WorkflowApi::new(pool.clone(), SystemClock)
        .list(ora_contracts::ListWorkflowsRequest {})
        .expect("list workflows");
    let mut names = listed
        .workflows
        .iter()
        .map(|workflow| workflow.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Returns one installed package's document path for a document a test rewrites in place.
fn installed_workflow_document(
    root: &Path,
    plugin: &str,
    version: &str,
    document: &str,
) -> PathBuf {
    root.join("plugins")
        .join("installed")
        .join("official")
        .join(plugin)
        .join(version)
        .join("assets")
        .join("workflows")
        .join(document)
}

/// Writes a Workflow `.orax` under the data directory's artifacts and returns its digest.
fn build_workflow_artifact(
    root: &Path,
    identifier: &str,
    version: &str,
    documents: &[(&str, &str)],
) -> (PathBuf, String) {
    let artifact = root
        .join("artifacts")
        .join(format!("{identifier}-{version}.orax"));
    fs::create_dir_all(artifact.parent().unwrap()).expect("create artifacts dir");
    write_workflow_orax_version(&artifact, identifier, version, documents);
    let sha = ora_utils::hash::sha256_file(&artifact).expect("hash artifact");
    (artifact, sha)
}

/// Commits one already-staged marketplace listing as a new origin commit.
fn publish_listing(root: &Path, identifier: &str, listing: &str, message: &str) {
    let origin = root.join("marketplace-origin");
    let git_config = root.join("gitconfig");
    stage_listing(&origin, identifier, listing);
    run_git(&origin, &git_config, &["add", "."]);
    run_git(&origin, &git_config, &["commit", "-m", message]);
}

/// Returns the member plugin a marketplace document outcome is attributed to.
fn attributed_plugin_id(outcome: &MarketplaceWorkflowOutcome) -> &str {
    match outcome {
        MarketplaceWorkflowOutcome::Imported { plugin_id, .. }
        | MarketplaceWorkflowOutcome::SkippedProcessed { plugin_id, .. }
        | MarketplaceWorkflowOutcome::SkippedConflict { plugin_id, .. }
        | MarketplaceWorkflowOutcome::Failed { plugin_id, .. } => plugin_id,
    }
}

/// Returns the package-relative document path one outcome is about.
fn outcome_source_file(outcome: &MarketplaceWorkflowOutcome) -> &str {
    match outcome {
        MarketplaceWorkflowOutcome::Imported { source_file, .. }
        | MarketplaceWorkflowOutcome::SkippedProcessed { source_file, .. }
        | MarketplaceWorkflowOutcome::SkippedConflict { source_file, .. }
        | MarketplaceWorkflowOutcome::Failed { source_file, .. } => source_file,
    }
}

/// circus#1010 repro promoted to a regression: a marketplace install of a Workflow package imports
/// its documents exactly like the local `.orax` path does.
///
/// The package lands on both paths with the same on-disk layout, so the only observable difference
/// is the workflow library and the per-document report the install returns.
#[tokio::test]
async fn marketplace_install_imports_workflow_documents() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const IDENTIFIER: &str = "ora-space.cdase-workflow";
    const PLUGIN_ID: &str = "official/ora-space.cdase-workflow";

    let (artifact, sha) = build_workflow_artifact(
        data_dir.path(),
        IDENTIFIER,
        "0.1.0",
        &[("1.0.0.json", WORKFLOW_DOCUMENT)],
    );
    stage_marketplace_checkout(
        data_dir.path(),
        &[(IDENTIFIER, workflow_listing(IDENTIFIER, "0.1.0", &sha))],
    );
    host.use_local_marketplace_release(PLUGIN_ID, artifact);

    let installed = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("install the workflow package from the marketplace");

    assert_eq!(installed.outcome, InstallOutcome::Installed);
    assert_eq!(listed_workflow_names(&pool), vec!["导入流程".to_string()]);
    assert!(
        matches!(
            installed.workflows.as_slice(),
            [MarketplaceWorkflowOutcome::Imported {
                plugin_id,
                source_file,
                name,
                version,
                ..
            }] if plugin_id == PLUGIN_ID
                && source_file == "assets/workflows/1.0.0.json"
                && name == "导入流程"
                && version == "1.0.0"
        ),
        "the install reports the document it imported: {:?}",
        installed.workflows
    );
}

/// circus#1010 repro promoted to a regression: the local `.orax` import keeps refusing a second
/// workflow of the same name, which is what makes a repeated import observable.
///
/// A fix that re-imported documents on `update` or on a repeated install would report a failed
/// document on every attempt after the first; the marketplace path answers with a skip instead.
#[tokio::test]
async fn importing_the_same_document_name_twice_refuses_the_second_import() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let api = test_plugin_api(data_dir.path(), &pool);

    let mut outcomes = Vec::new();
    for identifier in ["ora.wf-a", "ora.wf-b"] {
        let archive = data_dir.path().join(format!("{identifier}.orax"));
        write_workflow_orax(&archive, identifier, &[("1.0.0.json", WORKFLOW_DOCUMENT)]);
        let response = api
            .import(ImportPluginRequest {
                path: archive.to_string_lossy().into_owned(),
                hook_execution_acknowledged: false,
            })
            .await
            .expect("import the workflow package");
        outcomes.push(response.workflows);
    }

    assert!(
        matches!(
            outcomes[0].as_slice(),
            [ImportedWorkflowOutcome::Imported { .. }]
        ),
        "the first import creates the workflow: {:?}",
        outcomes[0]
    );
    assert!(
        matches!(
            outcomes[1].as_slice(),
            [ImportedWorkflowOutcome::Failed { .. }]
        ),
        "the second import of the same workflow name must be refused: {:?}",
        outcomes[1]
    );
}

/// circus#1010 repro promoted to a regression: a pack install imports the documents its installed
/// member contributed, attributed to that member rather than to the pack.
#[tokio::test]
async fn pack_install_imports_workflow_documents_of_its_members() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const MEMBER: &str = "ora-space.cdase-workflow";
    const MEMBER_ID: &str = "official/ora-space.cdase-workflow";
    const PACK: &str = "ora-space.cdase-pack";
    const PACK_ID: &str = "official/ora-space.cdase-pack";

    let (artifact, sha) = build_workflow_artifact(
        data_dir.path(),
        MEMBER,
        "0.1.0",
        &[("1.0.0.json", WORKFLOW_DOCUMENT)],
    );
    stage_marketplace_checkout(
        data_dir.path(),
        &[
            (MEMBER, workflow_listing(MEMBER, "0.1.0", &sha)),
            (PACK, pack_listing(PACK, &member_table(MEMBER))),
        ],
    );
    host.use_local_marketplace_release(MEMBER_ID, artifact);

    let installed = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PACK_ID.to_owned(),
        })
        .await
        .expect("install the pack");

    assert_eq!(
        installed.outcome,
        InstallOutcome::PackInstalled {
            members: vec![MEMBER_ID.to_owned()],
            skipped: Vec::new(),
            failed: None,
        }
    );
    assert!(
        member_installed(data_dir.path(), MEMBER, "0.1.0"),
        "the member package lands"
    );
    assert_eq!(listed_workflow_names(&pool), vec!["导入流程".to_string()]);
    assert_eq!(
        installed
            .workflows
            .iter()
            .map(attributed_plugin_id)
            .collect::<Vec<_>>(),
        vec![MEMBER_ID],
        "the report attributes the document to the member, not to the pack"
    );
}

/// An upgrade imports only the documents the new version added, and never rewrites the copy an
/// earlier version delivered.
#[tokio::test]
async fn marketplace_upgrade_imports_only_new_documents() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const IDENTIFIER: &str = "ora-space.cdase-workflow";
    const PLUGIN_ID: &str = "official/ora-space.cdase-workflow";

    let (v1, sha_v1) = build_workflow_artifact(
        data_dir.path(),
        IDENTIFIER,
        "0.1.0",
        &[("1.0.0.json", WORKFLOW_DOCUMENT)],
    );
    stage_marketplace_checkout(
        data_dir.path(),
        &[(IDENTIFIER, workflow_listing(IDENTIFIER, "0.1.0", &sha_v1))],
    );
    host.use_local_marketplace_release(PLUGIN_ID, v1);
    plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("install the first version");

    // The upgrade carries the same document under a changed graph plus one genuinely new document.
    let changed_first_document = WORKFLOW_DOCUMENT.replace("\"zoom\":1", "\"zoom\":2");
    let (v2, sha_v2) = build_workflow_artifact(
        data_dir.path(),
        IDENTIFIER,
        "0.2.0",
        &[
            ("1.0.0.json", changed_first_document.as_str()),
            ("2.0.0.json", SECOND_WORKFLOW_DOCUMENT),
        ],
    );
    publish_listing(
        data_dir.path(),
        IDENTIFIER,
        &workflow_listing(IDENTIFIER, "0.2.0", &sha_v2),
        "publish workflow package 0.2.0",
    );
    plugins
        .sync_available(ora_contracts::SyncAvailablePluginsRequest {})
        .expect("sync marketplace");
    host.use_local_marketplace_release(PLUGIN_ID, v2);
    let updated = plugins
        .update(UpdatePluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("update the workflow package");

    assert_eq!(
        listed_workflow_names(&pool),
        vec!["发布流程".to_string(), "导入流程".to_string()]
    );
    assert_eq!(
        updated
            .workflows
            .iter()
            .map(outcome_source_file)
            .collect::<Vec<_>>(),
        vec!["assets/workflows/1.0.0.json", "assets/workflows/2.0.0.json"],
        "the upgrade reports every document it considered, in package order"
    );
    assert!(
        matches!(
            &updated.workflows[0],
            MarketplaceWorkflowOutcome::SkippedProcessed { .. }
        ),
        "a path the first version delivered stays untouched: {:?}",
        updated.workflows[0]
    );
    assert!(
        matches!(
            &updated.workflows[1],
            MarketplaceWorkflowOutcome::Imported { name, .. } if name == "发布流程"
        ),
        "the new document is imported: {:?}",
        updated.workflows[1]
    );

    // The earlier delivery must still hold the graph the first version shipped.
    let workflows = crate::workflow::WorkflowApi::new(pool.clone(), SystemClock);
    let listed = workflows
        .list(ora_contracts::ListWorkflowsRequest {})
        .expect("list workflows");
    let imported = listed
        .workflows
        .iter()
        .find(|workflow| workflow.name == "导入流程")
        .expect("the first workflow survives the upgrade");
    let detail = workflows
        .get(ora_contracts::GetWorkflowRequest {
            workflow_id: imported.id.clone(),
        })
        .expect("load the imported workflow");
    assert_eq!(
        detail.draft.graph, WORKFLOW_DOCUMENT,
        "an upgrade never rewrites a copy an earlier version delivered"
    );
}

/// A name conflict with the user's own workflow is reported once and then remembered as terminal.
#[tokio::test]
async fn marketplace_install_reports_a_name_conflict_once() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const IDENTIFIER: &str = "ora-space.cdase-workflow";
    const PLUGIN_ID: &str = "official/ora-space.cdase-workflow";

    // The user already has a workflow of that name; the package must keep it, not replace it.
    let mine = crate::workflow::WorkflowApi::new(pool.clone(), SystemClock)
        .create(ora_contracts::CreateWorkflowRequest {
            name: "导入流程".to_owned(),
            graph: None,
        })
        .expect("create the user's workflow");

    let (v1, sha_v1) = build_workflow_artifact(
        data_dir.path(),
        IDENTIFIER,
        "0.1.0",
        &[("1.0.0.json", WORKFLOW_DOCUMENT)],
    );
    stage_marketplace_checkout(
        data_dir.path(),
        &[(IDENTIFIER, workflow_listing(IDENTIFIER, "0.1.0", &sha_v1))],
    );
    host.use_local_marketplace_release(PLUGIN_ID, v1);
    let installed = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("install the workflow package");
    assert_eq!(
        installed.workflows,
        vec![MarketplaceWorkflowOutcome::SkippedConflict {
            plugin_id: PLUGIN_ID.to_owned(),
            source_file: "assets/workflows/1.0.0.json".to_owned(),
        }]
    );
    assert_eq!(
        listed_workflow_names(&pool),
        vec!["导入流程".to_string()],
        "the user's workflow keeps its name and the package adds no second copy"
    );
    let workflows = crate::workflow::WorkflowApi::new(pool.clone(), SystemClock);
    let listed = workflows
        .list(ora_contracts::ListWorkflowsRequest {})
        .expect("list workflows");
    assert_eq!(
        listed.workflows[0].id, mine.workflow.id,
        "the copy that keeps the name is the user's own"
    );

    let (v2, sha_v2) = build_workflow_artifact(
        data_dir.path(),
        IDENTIFIER,
        "0.2.0",
        &[("1.0.0.json", WORKFLOW_DOCUMENT)],
    );
    publish_listing(
        data_dir.path(),
        IDENTIFIER,
        &workflow_listing(IDENTIFIER, "0.2.0", &sha_v2),
        "publish workflow package 0.2.0",
    );
    plugins
        .sync_available(ora_contracts::SyncAvailablePluginsRequest {})
        .expect("sync marketplace");
    host.use_local_marketplace_release(PLUGIN_ID, v2);
    let updated = plugins
        .update(UpdatePluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("update the workflow package");
    assert_eq!(
        updated.workflows,
        vec![MarketplaceWorkflowOutcome::SkippedProcessed {
            plugin_id: PLUGIN_ID.to_owned(),
            source_file: "assets/workflows/1.0.0.json".to_owned(),
        }],
        "the conflict is reported once, not on every upgrade"
    );
}

/// Only a document without a terminal record is retried; a successfully imported path stays done.
#[tokio::test]
async fn retry_workflow_import_reconsiders_only_unrecorded_paths() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const IDENTIFIER: &str = "ora-space.cdase-workflow";
    const PLUGIN_ID: &str = "official/ora-space.cdase-workflow";
    const MEMBER: &str = "ora-space.cdase-workflow";

    let (artifact, sha) = build_workflow_artifact(
        data_dir.path(),
        IDENTIFIER,
        "0.1.0",
        &[
            ("1.0.0.json", WORKFLOW_DOCUMENT),
            ("2.0.0.json", "{ not json"),
        ],
    );
    stage_marketplace_checkout(
        data_dir.path(),
        &[(IDENTIFIER, workflow_listing(IDENTIFIER, "0.1.0", &sha))],
    );
    host.use_local_marketplace_release(PLUGIN_ID, artifact);
    let installed = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("install the workflow package");
    assert!(
        matches!(
            installed.workflows.as_slice(),
            [
                MarketplaceWorkflowOutcome::Imported { .. },
                MarketplaceWorkflowOutcome::Failed { .. }
            ]
        ),
        "one broken document must not cost the working one: {:?}",
        installed.workflows
    );

    // The user fixes the document in place (a repaired package would ship the same bytes), then
    // asks for a retry without reinstalling anything.
    let broken = installed_workflow_document(data_dir.path(), MEMBER, "0.1.0", "2.0.0.json");
    fs::write(&broken, SECOND_WORKFLOW_DOCUMENT).expect("repair the document");

    let retried = plugins
        .retry_workflow_import(RetryWorkflowImportRequest {
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .expect("retry the document import");
    assert!(
        matches!(
            retried.workflows.as_slice(),
            [
                MarketplaceWorkflowOutcome::SkippedProcessed {
                    source_file: first,
                    ..
                },
                MarketplaceWorkflowOutcome::Imported {
                    source_file: second,
                    name,
                    version,
                    ..
                }
            ] if first == "assets/workflows/1.0.0.json"
                && second == "assets/workflows/2.0.0.json"
                && name == "发布流程"
                && version == "2.0.0"
        ),
        "the retry skips the imported path and imports the repaired one: {:?}",
        retried.workflows
    );
    assert_eq!(
        listed_workflow_names(&pool),
        vec!["发布流程".to_string(), "导入流程".to_string()]
    );
}

/// A workflow the user deleted stays deleted: its source path is terminal, so an upgrade and a
/// retry both refuse to bring it back.
#[tokio::test]
async fn a_deleted_workflow_is_not_resurrected_by_an_upgrade_or_retry() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const IDENTIFIER: &str = "ora-space.cdase-workflow";
    const PLUGIN_ID: &str = "official/ora-space.cdase-workflow";

    let (v1, sha_v1) = build_workflow_artifact(
        data_dir.path(),
        IDENTIFIER,
        "0.1.0",
        &[("1.0.0.json", WORKFLOW_DOCUMENT)],
    );
    stage_marketplace_checkout(
        data_dir.path(),
        &[(IDENTIFIER, workflow_listing(IDENTIFIER, "0.1.0", &sha_v1))],
    );
    host.use_local_marketplace_release(PLUGIN_ID, v1);
    plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("install the workflow package");

    let workflows = crate::workflow::WorkflowApi::new(pool.clone(), SystemClock);
    let listed = workflows
        .list(ora_contracts::ListWorkflowsRequest {})
        .expect("list workflows");
    workflows
        .delete(ora_contracts::DeleteWorkflowRequest {
            workflow_id: listed.workflows[0].id.clone(),
        })
        .expect("the user deletes the imported workflow");

    let (v2, sha_v2) = build_workflow_artifact(
        data_dir.path(),
        IDENTIFIER,
        "0.2.0",
        &[("1.0.0.json", WORKFLOW_DOCUMENT)],
    );
    publish_listing(
        data_dir.path(),
        IDENTIFIER,
        &workflow_listing(IDENTIFIER, "0.2.0", &sha_v2),
        "publish workflow package 0.2.0",
    );
    plugins
        .sync_available(ora_contracts::SyncAvailablePluginsRequest {})
        .expect("sync marketplace");
    host.use_local_marketplace_release(PLUGIN_ID, v2);
    let updated = plugins
        .update(UpdatePluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("update the workflow package");
    assert!(
        matches!(
            updated.workflows.as_slice(),
            [MarketplaceWorkflowOutcome::SkippedProcessed { .. }]
        ),
        "an upgrade leaves a deleted copy deleted: {:?}",
        updated.workflows
    );
    assert_eq!(listed_workflow_names(&pool), Vec::<String>::new());

    let retried = plugins
        .retry_workflow_import(RetryWorkflowImportRequest {
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .expect("retry the document import");
    assert!(
        matches!(
            retried.workflows.as_slice(),
            [MarketplaceWorkflowOutcome::SkippedProcessed { .. }]
        ),
        "a retry does not reset the user's choice either: {:?}",
        retried.workflows
    );
}

/// Uninstalling and reinstalling the package keeps the source record, so no copy comes back.
#[tokio::test]
async fn reinstalling_the_package_does_not_import_the_documents_again() {
    let _trace = trace_guard();
    let data_dir = TempDir::new().expect("data dir");
    let pool = test_pool(data_dir.path());
    let (plugins, host) = pack_test_plugins(data_dir.path(), &pool);
    const IDENTIFIER: &str = "ora-space.cdase-workflow";
    const PLUGIN_ID: &str = "official/ora-space.cdase-workflow";

    let (artifact, sha) = build_workflow_artifact(
        data_dir.path(),
        IDENTIFIER,
        "0.1.0",
        &[("1.0.0.json", WORKFLOW_DOCUMENT)],
    );
    stage_marketplace_checkout(
        data_dir.path(),
        &[(IDENTIFIER, workflow_listing(IDENTIFIER, "0.1.0", &sha))],
    );
    host.use_local_marketplace_release(PLUGIN_ID, artifact.clone());
    plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("install the workflow package");
    plugins
        .uninstall(UninstallPluginRequest {
            plugin_id: PLUGIN_ID.to_owned(),
            data_disposition: PluginDataDisposition::Retain,
            hook_execution_acknowledged: false,
        })
        .await
        .expect("uninstall the workflow package");

    let reinstalled = plugins
        .install(InstallPluginRequest {
            hook_execution_acknowledged: false,
            plugin_id: PLUGIN_ID.to_owned(),
        })
        .await
        .expect("reinstall the workflow package");
    assert_eq!(
        reinstalled.workflows,
        vec![MarketplaceWorkflowOutcome::SkippedProcessed {
            plugin_id: PLUGIN_ID.to_owned(),
            source_file: "assets/workflows/1.0.0.json".to_owned(),
        }],
        "a reinstall keeps every terminal record"
    );
    assert_eq!(
        listed_workflow_names(&pool),
        vec!["导入流程".to_string()],
        "the reinstall adds no second copy"
    );
}
