//! Direct tests for the origin-presentation ADR completion conditions.
//!
//! Every origin-bearing response resolves against the live binding table: one binding set labels
//! the Skill, installed and marketplace lists alike, a new namesake relabels the next query
//! without moving identity, and a binding table that cannot be read degrades every derived
//! namespace with a warning instead of failing the lists.

use crate::{Backend, test_backend::backend_paths};
use gitlancer::BranchName;
use ora_contracts::{
    AddMarketplaceSourceRequest, DeleteMarketplaceSourceRequest, GetSkillRequest,
    ListAvailablePluginsRequest, ListInstalledPluginsRequest, ListSkillsRequest, PluginOrigin,
    SkillSource, SyncAvailablePluginsRequest,
};
use ora_domain::PluginNamespace;
use ora_plugin_registry::{RegistryIndex, RegistrySource};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const FORK_URL: &str = "https://github.com/obsismc/marketplace";
/// A namesake whose adjacent segment matches the fork, forcing the fork's label to skip to the host.
const NAMESAKE_URL: &str = "https://gitlab.com/obsismc/marketplace";
const PLUGIN_NAME: &str = "acme.review";

/// The namespace the fork URL derives, frozen into the installed package and registry entry.
fn fork_namespace() -> PluginNamespace {
    PluginNamespace::derive_from_canonical_url(FORK_URL)
}

/// Writes the marketplace manifest a registry entry directory publishes.
fn registry_manifest(identifier: &str) -> String {
    format!(
        "resolver = 1\n\
         identifier = \"{identifier}\"\n\
         kind = \"skill\"\n\
         version = \"1.0.0\"\n\
         description = \"Example plugin\"\n\
         url = \"https://example.com/{identifier}.orax\"\n\
         sha256 = \"{}\"\n",
        "ab".repeat(32)
    )
}

/// Writes one installed Skill package under `namespace`, as an install from the fork would.
fn write_installed_skill_package(home: &Path, namespace: &str) {
    let package_root = home
        .join("plugins")
        .join("installed")
        .join(namespace)
        .join(PLUGIN_NAME)
        .join("1.0.0");
    fs::create_dir_all(package_root.join("assets/review")).expect("create skill assets");
    fs::write(
        package_root.join("assets/review/SKILL.md"),
        "---\nname: review\ndescription: Reviews code\n---\n",
    )
    .expect("write bundled skill manifest");
    fs::write(
        package_root.join("orax.toml"),
        format!(
            "identifier = \"{PLUGIN_NAME}\"\nnamespace = \"{namespace}\"\nkind = \"skill\"\n\
             version = \"1.0.0\"\ndescription = \"Example Skill plugin\"\n"
        ),
    )
    .expect("write skill plugin manifest");
}

/// Writes a cached registry index carrying one listing published under `namespace`.
fn write_registry_cache(home: &Path, url: &str, namespace: PluginNamespace) {
    let checkout = home.join("source-checkout");
    let entry_dir = checkout.join("registry").join(PLUGIN_NAME);
    fs::create_dir_all(&entry_dir).expect("create registry entry directory");
    fs::write(entry_dir.join("orax.toml"), registry_manifest(PLUGIN_NAME))
        .expect("write registry manifest");
    let source = RegistrySource::new(url, namespace, BranchName::new("main"), &checkout);
    let build = RegistryIndex::build_all(&[&source], 1);
    let cache = home
        .join("plugins")
        .join("cache")
        .join("registry_index.json");
    fs::create_dir_all(cache.parent().expect("cache has parent")).expect("create cache directory");
    build.index().write(&cache).expect("write registry cache");
}

/// Opens a backend whose fork source is bound and whose plugin appears in all three lists.
///
/// The installed package and registry cache are written before open so discovery and the cached
/// index both carry the derived namespace; boot reconciliation registers the bundled plugin Skill.
fn fork_backend(temp: &TempDir) -> (Backend, PluginNamespace) {
    let namespace = fork_namespace();
    write_installed_skill_package(temp.path(), namespace.as_str());
    write_registry_cache(temp.path(), FORK_URL, namespace.clone());
    let backend = Backend::open(backend_paths(temp.path(), temp.path())).expect("open backend");
    backend
        .plugins()
        .add_source(AddMarketplaceSourceRequest {
            url: FORK_URL.to_string(),
            branch: "main".to_string(),
            use_proxy: false,
        })
        .expect("bind the fork source");
    (backend, namespace)
}

/// The marketplace listing's origin, read through the cached-index endpoint.
fn marketplace_origin(backend: &Backend) -> PluginOrigin {
    backend
        .plugins()
        .list_available(ListAvailablePluginsRequest {})
        .expect("list available")
        .plugins
        .into_iter()
        .find(|plugin| plugin.name == PLUGIN_NAME)
        .expect("fork listing")
        .origin
}

/// The installed plugin's origin, read through the installed-list endpoint.
fn installed_origin(backend: &Backend) -> PluginOrigin {
    backend
        .plugins()
        .list_installed(ListInstalledPluginsRequest {})
        .expect("list installed")
        .plugins
        .into_iter()
        .find(|plugin| plugin.name == PLUGIN_NAME)
        .expect("installed fork plugin")
        .origin
}

/// The plugin Skill's origin, read through the Skill-list endpoint.
fn skill_origin(backend: &Backend) -> PluginOrigin {
    let listed = backend
        .skills()
        .list(ListSkillsRequest {})
        .expect("list skills")
        .skills
        .into_iter()
        .find(|skill| skill.name == "review")
        .expect("plugin skill")
        .source;
    match listed {
        SkillSource::Plugin { origin, .. } => origin,
        SkillSource::Local => panic!("expected a plugin skill"),
    }
}

/// One binding set gives the Skill, installed and marketplace lists the same label.
#[test]
fn one_binding_set_labels_all_three_lists_alike() {
    ora_logging::with_trace_logging(|| {
        let temp = TempDir::new().expect("temp directory");
        let (backend, _namespace) = fork_backend(&temp);

        let expected = PluginOrigin::Marketplace {
            source_url: Some(FORK_URL.to_string()),
            label: "obsismc/marketplace".to_string(),
        };
        assert_eq!(
            (
                marketplace_origin(&backend),
                installed_origin(&backend),
                skill_origin(&backend),
            ),
            (expected.clone(), expected.clone(), expected),
        );
    });
}

/// A new namesake source relabels the next query while namespace and Canonical Plugin ID stay put.
#[test]
fn a_new_namesake_relabels_the_next_query_without_moving_identity() {
    ora_logging::with_trace_logging(|| {
        let temp = TempDir::new().expect("temp directory");
        let (backend, namespace) = fork_backend(&temp);
        let canonical_id = format!("{}/{PLUGIN_NAME}", namespace.as_str());

        let before = backend
            .plugins()
            .list_available(ListAvailablePluginsRequest {})
            .expect("list available")
            .plugins
            .into_iter()
            .find(|plugin| plugin.name == PLUGIN_NAME)
            .expect("fork listing");
        assert_eq!(
            before.origin,
            PluginOrigin::Marketplace {
                source_url: Some(FORK_URL.to_string()),
                label: "obsismc/marketplace".to_string(),
            },
        );

        backend
            .plugins()
            .add_source(AddMarketplaceSourceRequest {
                url: NAMESAKE_URL.to_string(),
                branch: "main".to_string(),
                use_proxy: false,
            })
            .expect("bind the namesake source");

        let after = backend
            .plugins()
            .list_available(ListAvailablePluginsRequest {})
            .expect("list available again")
            .plugins
            .into_iter()
            .find(|plugin| plugin.name == PLUGIN_NAME)
            .expect("fork listing still present");
        // The label is recomputed per query, so the namesake forces the host into it; identity is
        // frozen and must not move.
        assert_eq!(
            (after.id.clone(), after.namespace.clone(), after.origin),
            (
                canonical_id.clone(),
                namespace.as_str().to_string(),
                PluginOrigin::Marketplace {
                    source_url: Some(FORK_URL.to_string()),
                    label: "github.com/obsismc/marketplace".to_string(),
                },
            ),
        );
        // The installed list reads the same live bindings, so it relabels too.
        assert_eq!(
            installed_origin(&backend),
            PluginOrigin::Marketplace {
                source_url: Some(FORK_URL.to_string()),
                label: "github.com/obsismc/marketplace".to_string(),
            },
        );
        assert_eq!(
            backend
                .plugins()
                .list_installed(ListInstalledPluginsRequest {})
                .expect("list installed")
                .plugins
                .into_iter()
                .find(|plugin| plugin.name == PLUGIN_NAME)
                .expect("installed plugin")
                .id,
            canonical_id,
        );
    });
}

/// Collects the message of every recorded event so a test can assert what was warned.
#[derive(Clone, Default)]
struct Recorded(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Recorded {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _context: tracing_subscriber::layer::Context<'_, S>,
    ) {
        use tracing::field::{Field, Visit};
        struct Message(Option<String>);
        impl Visit for Message {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = Some(format!("{value:?}"));
                }
            }
        }
        let mut message = Message(None);
        event.record(&mut message);
        if let Some(message) = message.0 {
            self.0.lock().unwrap().push(message);
        }
    }
}

/// A binding table that cannot be read degrades every derived namespace and warns, but no list
/// fails: origins only label entries, so hiding the lists would hide what the user wants to see.
#[test]
fn a_binding_read_failure_degrades_every_list_and_warns() {
    let temp = TempDir::new().expect("temp directory");
    let (backend, _namespace) = fork_backend(&temp);
    rusqlite::Connection::open(temp.path().join("ora.sqlite3"))
        .expect("open raw connection")
        .execute_batch("DROP TABLE plugin_source_namespace")
        .expect("make the binding table unreadable");

    let recorded = Recorded::default();
    let degraded = PluginOrigin::Marketplace {
        source_url: None,
        label: "marketplace".to_string(),
    };
    let origins = ora_logging::with_recorded_trace_logging(recorded.clone(), || {
        (
            marketplace_origin(&backend),
            installed_origin(&backend),
            skill_origin(&backend),
        )
    });

    assert_eq!(origins, (degraded.clone(), degraded.clone(), degraded));
    assert!(
        recorded
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|message| message == "failed to read plugin source bindings for origin labels"),
        "the binding read failure must be warned, got {:?}",
        recorded.0.lock().unwrap(),
    );
}

/// The Skill detail response carries the same origin as the lists, resolved against the bindings.
#[test]
fn skill_details_carry_the_plugin_origin() {
    ora_logging::with_trace_logging(|| {
        let temp = TempDir::new().expect("temp directory");
        let (backend, namespace) = fork_backend(&temp);

        let details = backend
            .skills()
            .get(GetSkillRequest {
                skill_id: format!("plugin:{}/{PLUGIN_NAME}:review", namespace.as_str()),
            })
            .expect("get skill")
            .skill;
        assert_eq!(
            details.source,
            SkillSource::Plugin {
                plugin_id: format!("{}/{PLUGIN_NAME}", namespace.as_str()),
                origin: PluginOrigin::Marketplace {
                    source_url: Some(FORK_URL.to_string()),
                    label: "obsismc/marketplace".to_string(),
                },
            },
        );
    });
}

/// Runs one git command with an isolated empty global/system config.
fn run_git(directory: &Path, git_config: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-c")
        .arg("user.name=Origin Test")
        .arg("-c")
        .arg("user.email=origin@ora.local")
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

/// The sync response carries the derived-namespace origin, computed against the live bindings.
///
/// A local git origin stands in for the remote so the rebuild fetches offline; the seeded official
/// source is removed so the rebuild reaches only the fork checkout.
#[test]
fn sync_available_projects_the_derived_namespace_origin() {
    ora_logging::with_trace_logging(|| {
        let temp = TempDir::new().expect("temp directory");
        let backend = Backend::open(backend_paths(temp.path(), temp.path())).expect("open backend");
        backend
            .plugins()
            .delete_source(DeleteMarketplaceSourceRequest {
                url: PluginNamespace::DEFAULT_MARKETPLACE_URL.to_string(),
            })
            .expect("remove the seeded official source");
        backend
            .plugins()
            .add_source(AddMarketplaceSourceRequest {
                url: FORK_URL.to_string(),
                branch: "main".to_string(),
                use_proxy: false,
            })
            .expect("bind the fork source");
        let namespace = fork_namespace();

        let origin = temp.path().join("fork-origin");
        let entry_dir = origin.join("registry").join(PLUGIN_NAME);
        fs::create_dir_all(&entry_dir).expect("create origin registry");
        fs::write(entry_dir.join("orax.toml"), registry_manifest(PLUGIN_NAME))
            .expect("write the fork listing");
        let git_config = temp.path().join("gitconfig");
        fs::write(&git_config, "").expect("write git config");
        run_git(&origin, &git_config, &["init", "--initial-branch=main"]);
        run_git(&origin, &git_config, &["add", "."]);
        run_git(
            &origin,
            &git_config,
            &["commit", "-m", "stage fork listing"],
        );

        let sources_root = temp.path().join("plugins").join("sources");
        let source = RegistrySource::try_from_git(FORK_URL, namespace, "main", &sources_root)
            .expect("derive the fork checkout");
        let checkout = source.checkout_dir().to_path_buf();
        fs::create_dir_all(checkout.parent().expect("checkout has parent"))
            .expect("create checkout parent");
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

        let synced = backend
            .plugins()
            .sync_available(SyncAvailablePluginsRequest {})
            .expect("sync the fork source");
        let entry = synced
            .plugins
            .iter()
            .find(|plugin| plugin.name == PLUGIN_NAME)
            .expect("fork listing");
        assert_eq!(
            entry.origin,
            PluginOrigin::Marketplace {
                source_url: Some(FORK_URL.to_string()),
                label: "obsismc/marketplace".to_string(),
            },
        );
    });
}
