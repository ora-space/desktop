//! Projection of one cached registry entry into the marketplace summary the frontend renders.

use ora_application::plugin_origin_contract;
use ora_contracts::{AvailablePlugin, PluginHostCompatibility};
use ora_domain::PluginOriginCatalog;
use ora_plugin_asset::{AssetUrlForm, LogoAssetRoot, plugin_logo};
use ora_plugin_registry::RegistryEntry;

/// Converts one registry entry into the frontend-facing marketplace summary.
///
/// `origins` is the snapshot shared by every entry of one response, so namesake sources in the
/// same list are labelled against the same binding set.
pub(super) fn available_plugin(
    entry: &RegistryEntry,
    origins: &PluginOriginCatalog,
) -> AvailablePlugin {
    AvailablePlugin {
        id: entry.id().canonical(),
        name: entry.identifier().to_owned(),
        title: entry.title().to_owned(),
        kind: entry.kind().to_owned(),
        namespace: entry.namespace().to_owned(),
        source_url: entry.source_url().to_owned(),
        origin: plugin_origin_contract(origins, entry.namespace()),
        version: entry.version().to_string(),
        description: entry.description().to_owned(),
        logo: entry.logo().and_then(|variants| {
            // The card is about the listing, so it draws what the source publishes now,
            // which is also the only directory the index resolved these variants from.
            plugin_logo(
                AssetUrlForm::CURRENT,
                LogoAssetRoot::Registry,
                entry.id(),
                &variants,
            )
        }),
        compatibility: match entry.host_compatibility() {
            Ok(()) => PluginHostCompatibility::Compatible,
            Err(reason) => PluginHostCompatibility::Incompatible { reason },
        },
        pack_members: entry.pack_members().map(<[String]>::to_vec),
    }
}

#[cfg(test)]
mod tests {
    use super::available_plugin;
    use crate::{Backend, BackendPaths};
    use gitlancer::BranchName;
    use ora_contracts::{ListAvailablePluginsRequest, ListAvailablePluginsResponse, PluginLogo};
    use ora_domain::{PluginId, PluginNamespace};
    use ora_plugin_asset::{
        AssetUrlForm, LogoCandidate, LogoExtension, LogoRole, PluginLogoVariants,
    };
    use ora_plugin_manager::PluginManager;
    use ora_plugin_registry::{RegistryIndex, RegistrySource};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::{Path, PathBuf};

    const SAFE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="8"/></svg>"#;

    /// Builds the marketplace manifest a registry entry directory publishes.
    fn registry_manifest(identifier: &str) -> String {
        format!(
            "resolver = 1\n\
             identifier = \"{identifier}\"\n\
             kind = \"agent\"\n\
             version = \"1.0.0\"\n\
             description = \"Example plugin\"\n\
             url = \"https://example.com/{identifier}.orax\"\n\
             sha256 = \"{}\"\n",
            "ab".repeat(32)
        )
    }

    /// Writes one registry entry directory and returns it.
    fn write_registry_entry(checkout: &Path, identifier: &str) -> PathBuf {
        let entry_dir = checkout.join("registry").join(identifier);
        fs::create_dir_all(&entry_dir).expect("create the registry entry directory");
        fs::write(entry_dir.join("orax.toml"), registry_manifest(identifier))
            .expect("write the registry manifest");
        entry_dir
    }

    /// Writes one installed package directory and returns it.
    fn write_installed_package(data_dir: &Path, identifier: &str) -> PathBuf {
        let package_root = data_dir
            .join("plugins")
            .join("installed")
            .join("official")
            .join(identifier)
            .join("1.0.0");
        fs::create_dir_all(&package_root).expect("create the package directory");
        fs::write(
            package_root.join("orax.toml"),
            registry_manifest(identifier),
        )
        .expect("write the package manifest");
        fs::write(package_root.join("main.js"), "export default {};")
            .expect("write the package entrypoint");
        package_root
    }

    /// Writes the icon files a plugin shipping a mixed-format theme pair would publish.
    fn write_theme_pair(directory: &Path) {
        fs::write(directory.join("logo.light.svg"), SAFE_SVG).expect("write the light icon");
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        fs::write(directory.join("logo.dark.png"), png).expect("write the dark icon");
    }

    /// The contract carries one asset URL per theme, never the icon's bytes.
    #[test]
    fn projects_a_cached_entry_into_asset_urls() -> Result<(), Box<dyn std::error::Error>> {
        let checkout = tempfile::tempdir()?;
        let entry_dir = write_registry_entry(checkout.path(), "acme.hub");
        write_theme_pair(&entry_dir);
        let source = RegistrySource::new(
            "https://github.com/ora-space/marketplace",
            PluginNamespace::official(),
            BranchName::new("main"),
            checkout.path(),
        );

        let build = RegistryIndex::build_all(&[&source], 1);
        let listed = available_plugin(
            &build.index().plugins()[0],
            &ora_domain::PluginOriginCatalog::EMPTY,
        );

        let origin = AssetUrlForm::CURRENT.origin();
        assert_eq!(
            listed.logo,
            Some(PluginLogo::Themed {
                light: format!("{origin}logo/registry/official/acme.hub/light.svg"),
                dark: format!("{origin}logo/registry/official/acme.hub/dark.png"),
            })
        );
        Ok(())
    }

    /// A third-party listing carries the label computed against every recorded source binding.
    #[test]
    fn labels_a_third_party_listing_against_its_namesakes() -> Result<(), Box<dyn std::error::Error>>
    {
        let checkout = tempfile::tempdir()?;
        write_registry_entry(checkout.path(), "acme.hub");
        let url = "https://github.com/obsismc/marketplace";
        let namespace = PluginNamespace::derive_from_canonical_url(url);
        let source = RegistrySource::new(
            url,
            namespace.clone(),
            BranchName::new("main"),
            checkout.path(),
        );
        let origins = ora_domain::PluginOriginCatalog::from_bindings([
            (
                "marketplace.11111111".to_string(),
                "https://github.com/ora-space-fork/marketplace".to_string(),
            ),
            (namespace.to_string(), url.to_string()),
        ]);

        let build = RegistryIndex::build_all(&[&source], 1);
        let listed = available_plugin(&build.index().plugins()[0], &origins);

        assert_eq!(
            (listed.source_url, listed.origin),
            (
                url.to_string(),
                ora_contracts::PluginOrigin::Marketplace {
                    source_url: Some(url.to_string()),
                    label: "obsismc/marketplace".to_string(),
                },
            ),
        );
        Ok(())
    }

    /// A lone fork of the official marketplace is labelled against the default source's URL even
    /// though that URL is not a binding, so the card reads `obsismc/marketplace` beside "official".
    #[test]
    fn labels_a_lone_official_fork_against_the_default_marketplace()
    -> Result<(), Box<dyn std::error::Error>> {
        let checkout = tempfile::tempdir()?;
        write_registry_entry(checkout.path(), "acme.hub");
        let url = "https://github.com/obsismc/marketplace";
        let namespace = PluginNamespace::derive_from_canonical_url(url);
        let source = RegistrySource::new(
            url,
            namespace.clone(),
            BranchName::new("main"),
            checkout.path(),
        );
        let origins = ora_domain::PluginOriginCatalog::from_bindings([(
            namespace.to_string(),
            url.to_string(),
        )]);

        let build = RegistryIndex::build_all(&[&source], 1);
        let listed = available_plugin(&build.index().plugins()[0], &origins);

        assert_eq!(
            listed.origin,
            ora_contracts::PluginOrigin::Marketplace {
                source_url: Some(url.to_string()),
                label: "obsismc/marketplace".to_string(),
            },
        );
        Ok(())
    }

    /// The same files resolve to the same icon in the marketplace and after install.
    ///
    /// This is the drift the shared resolver exists to prevent: two implementations of the
    /// candidate table would show a plugin one icon on its marketplace card and a different one
    /// in the installed list, a difference a user can see but nobody would think to test.
    #[test]
    fn the_market_and_the_installed_package_agree_on_the_icon()
    -> Result<(), Box<dyn std::error::Error>> {
        let checkout = tempfile::tempdir()?;
        let data = tempfile::tempdir()?;
        let entry_dir = write_registry_entry(checkout.path(), "acme.hub");
        let package_root = write_installed_package(data.path(), "acme.hub");
        // Both directories carry an identical, deliberately awkward candidate set: a themed pair
        // in two different formats, plus a universal file the pair must win over.
        write_theme_pair(&entry_dir);
        write_theme_pair(&package_root);
        fs::write(entry_dir.join("logo.svg"), SAFE_SVG)?;
        fs::write(package_root.join("logo.svg"), SAFE_SVG)?;
        let source = RegistrySource::new(
            "https://github.com/ora-space/marketplace",
            PluginNamespace::official(),
            BranchName::new("main"),
            checkout.path(),
        );

        let build = RegistryIndex::build_all(&[&source], 1);
        let manager = PluginManager::discover(data.path());

        // The pair wins over the universal file on both sides, and each half keeps naming the
        // file it came from rather than the theme it is drawn under.
        let expected = Some(PluginLogoVariants::Themed {
            light: LogoCandidate {
                role: LogoRole::Light,
                extension: LogoExtension::Svg,
            },
            dark: LogoCandidate {
                role: LogoRole::Dark,
                extension: LogoExtension::Png,
            },
        });
        assert_eq!(
            (
                build.index().plugins()[0].logo(),
                manager.installed_plugins()[0].logo,
            ),
            (expected, expected)
        );
        Ok(())
    }

    /// A cache this host cannot read degrades to not-yet-synced instead of failing the endpoint.
    ///
    /// The pre-upgrade cache spells `logo` as SVG source text, which is a type mismatch rather
    /// than a missing field, so no serde default rescues it. Reporting it as an error would turn
    /// an index schema change into a marketplace that looks broken, and the endpoint never
    /// reaches the network, so the only remedy either way is the user syncing once.
    #[tokio::test]
    async fn a_stale_index_cache_degrades_to_not_yet_synced() {
        ora_logging::initialize_test_clock();
        let temporary = tempfile::tempdir().expect("temporary backend root");
        let home = temporary.path().join("home");
        let cache = home.join("plugins").join("cache");
        fs::create_dir_all(&cache).expect("create the cache directory");
        let stale = cache.join("registry_index.json");
        fs::write(
            &stale,
            r#"{"updated_at":1700000000,"version":"1.0","plugins":[{
                "id":"official/acme.hub","identifier":"acme.hub","title":"Hub","kind":"agent",
                "namespace":"official","source_url":"https://github.com/ora-space/marketplace",
                "version":"1.0.0","description":"Example plugin","logo":"<svg/>"
            }]}"#,
        )
        .expect("write a pre-upgrade index cache");

        let backend = Backend::open(BackendPaths {
            app_data_directory: temporary.path().join("data"),
            home_directory: home,
            deno_path: PathBuf::from("deno"),
            relative_path_base: temporary.path().to_path_buf(),
            timezone: "Asia/Shanghai".parse().expect("local timezone"),
        })
        .expect("open backend");

        let response = ora_logging::with_trace_logging(|| {
            backend
                .plugins()
                .list_available(ListAvailablePluginsRequest {})
                .expect("an unusable cache is not an endpoint failure")
        });

        assert_eq!(
            response,
            ListAvailablePluginsResponse {
                updated_at: 0,
                plugins: Vec::new(),
                failed_sources: Vec::new(),
            }
        );
        // Reading the marketplace never repairs the cache: that would mean touching the network
        // on a path the distribution decision keeps offline.
        assert!(stale.is_file());
        assert!(
            fs::read_to_string(&stale)
                .expect("read back")
                .contains("\"1.0\"")
        );
    }

    /// An entry that publishes no icon reaches the contract with none, not with a placeholder.
    #[test]
    fn projects_an_entry_without_an_icon_as_no_logo() -> Result<(), Box<dyn std::error::Error>> {
        let checkout = tempfile::tempdir()?;
        write_registry_entry(checkout.path(), "acme.hub");
        let source = RegistrySource::new(
            "https://github.com/ora-space/marketplace",
            PluginNamespace::official(),
            BranchName::new("main"),
            checkout.path(),
        );

        let build = RegistryIndex::build_all(&[&source], 1);

        assert_eq!(
            available_plugin(
                &build.index().plugins()[0],
                &ora_domain::PluginOriginCatalog::EMPTY
            )
            .logo,
            None
        );
        assert_eq!(
            build.index().plugins()[0].id(),
            &PluginId::new("official", "acme.hub").expect("plugin id")
        );
        Ok(())
    }
}
