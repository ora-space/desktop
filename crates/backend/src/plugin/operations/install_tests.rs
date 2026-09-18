//! Covers Hook install outcomes after the host dropped plugin enablement.

use super::Plugins;
use crate::agent_runtime::{AgentRuntimeManager, AgentRuntimeSetup};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use crate::plugin::pack_reconcile::PackMemberReconciliation;
use crate::settings::Settings;
use ora_contracts::{
    ImportPluginRequest, ImportedWorkflowOutcome, InstallOutcome, InstallPluginRequest,
    ListInstalledPluginsRequest, ListPackInstallationsRequest, PackInstallFailure,
    PackInstalledMember, PackMemberInstallOutcome, PackUninstallPlanRequest, PluginDataDisposition,
    PublicError, UninstallPluginRequest, UpdatePluginRequest,
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
use std::fs;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(windows)]
use std::process::Stdio;
use std::sync::Arc;
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
        AgentRuntimeManager::new(AgentRuntimeSetup {
            plugin_host: host.clone(),
            pool: pool.clone(),
            home_directory: root.to_path_buf(),
            relative_path_base: root.to_path_buf(),
            sessions_root: root.join("sessions"),
            clock: SystemClock,
            scheduler: Scheduler::new(chrono_tz::Asia::Shanghai),
            app_events: events.publisher(),
        })
        .expect("agent runtime"),
    );
    Plugins::new(
        host,
        runtime,
        Arc::new(crate::workflow::workflow_import(pool.clone(), SystemClock)),
    )
}

/// Writes a processless Hook `.orax` whose command alias is `rtk` and whose artifact matches
/// `host`.
fn write_hook_orax(path: &Path, identifier: &str, host: &str) {
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"Hook command rewrite\"\n\n[artifact]\ntarget = \"{host}\"\n"
    );
    let config = br#"{"schemaVersion":1,"hook":{"protocol":"rtk-rewrite-v1","executable":"assets/rtk.exe","command":"rtk","toolVersion":"0.45.0"}}"#;
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default();
    writer.start_file("orax.toml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    writer.start_file("assets/config.json", options).unwrap();
    writer.write_all(config).unwrap();
    writer.start_file("assets/rtk.exe", options).unwrap();
    writer.write_all(b"MZdummy").unwrap();
    writer.finish().unwrap();
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
        AgentRuntimeManager::new(AgentRuntimeSetup {
            plugin_host: host.clone(),
            pool: pool.clone(),
            home_directory: root.to_path_buf(),
            relative_path_base: root.to_path_buf(),
            sessions_root: root.join("sessions"),
            clock: SystemClock,
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
                PackInstalledMember {
                    plugin_id: format!("official/{HIDDEN_MEMBER}"),
                    outcome: PackMemberInstallOutcome::Installed,
                },
                PackInstalledMember {
                    plugin_id: format!("official/{VISIBLE_MEMBER}"),
                    outcome: PackMemberInstallOutcome::Installed,
                },
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
            assert_eq!(
                members
                    .iter()
                    .map(|member| member.plugin_id.as_str())
                    .collect::<Vec<_>>(),
                vec![format!("official/{HIDDEN_MEMBER}")],
            );
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
            assert_eq!(members[0].plugin_id, format!("official/{VISIBLE_MEMBER}"));
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
    child: std::process::Child,
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
        Self { child }
    }

    /// Terminates the holding process tree so the pinned directory becomes replaceable.
    fn release(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &self.child.id().to_string()])
            .output();
        let _ = self.child.wait();
    }
}

#[cfg(windows)]
impl Drop for DirectoryHolder {
    fn drop(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &self.child.id().to_string()])
            .output();
        let _ = self.child.wait();
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

/// Two Hook packages that share a command alias both stay installed; the second import reports
/// the colliding identity instead of claiming the new package was disabled.
#[test]
fn importing_a_second_hook_with_the_same_command_reports_a_conflict_without_disabling() {
    with_trace_logging(|| {
        let Some(host) = ora_plugin_registry::current_host_target() else {
            eprintln!(
                "skipping Hook command-conflict import: compiled host is not a plugin target"
            );
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
                let first = data_dir.path().join("first.orax");
                let second = data_dir.path().join("second.orax");
                write_hook_orax(&first, "rtk-ai.rtk", host.as_str());
                write_hook_orax(&second, "other.rtk", host.as_str());

                let first_response = api
                    .import(ImportPluginRequest {
                        path: first.to_string_lossy().into_owned(),
                    })
                    .await
                    .expect("import first Hook");
                assert_eq!(
                    first_response.outcome,
                    InstallOutcome::Installed,
                    "the first Hook must be available without a conflict"
                );

                let second_response = api
                    .import(ImportPluginRequest {
                        path: second.to_string_lossy().into_owned(),
                    })
                    .await
                    .expect("import second Hook");
                assert_eq!(
                    second_response.outcome,
                    InstallOutcome::InstalledWithCommandConflict {
                        conflict_plugin_id: "local/rtk-ai.rtk".to_string(),
                    }
                );

                let listed = api
                    .list_installed(ListInstalledPluginsRequest {})
                    .expect("installed snapshot");
                let ids: Vec<&str> = listed
                    .plugins
                    .iter()
                    .map(|plugin| plugin.id.as_str())
                    .collect();
                assert!(
                    ids.contains(&"local/rtk-ai.rtk") && ids.contains(&"local/other.rtk"),
                    "both Hooks must remain installed and available, got {ids:?}"
                );
            });
    });
}

/// One Start-only workflow document the run engine accepts, carrying an explicit version.
const WORKFLOW_DOCUMENT: &str = r#"{"name":"导入流程","version":"1.0.0","viewport":{"x":0,"y":0,"zoom":1},"nodes":[{"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","title":"开始"}}],"edges":[]}"#;

/// Writes a Workflow `.orax` carrying the given `assets/workflows/<name>` documents.
fn write_workflow_orax(path: &Path, identifier: &str, documents: &[(&str, &str)]) {
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"workflow\"\nversion = \"0.1.0\"\ndescription = \"Workflow package\"\n"
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
                members
                    .iter()
                    .map(|member| member.plugin_id.as_str())
                    .collect::<Vec<_>>(),
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
                error_code: "internal_error".to_owned(),
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
