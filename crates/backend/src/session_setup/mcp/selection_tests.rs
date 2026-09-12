//! Explicit workflow policies must restrict delivery, validation, and revision observation alike.

use super::tests::{
    FakeCatalog, FakeConfigurations, Fixture, candidate, capabilities, plugin, stdio_config,
};
use super::{
    McpConfigurationEligibility, SessionMcpError, SessionMcpRevision, SessionMcpSelection,
    resolve_session_mcp, resolve_session_mcp_revision,
};
use pretty_assertions::assert_eq;
use semver::Version;
use std::collections::BTreeMap;

/// Builds canonical allowlists just as frozen workflow bindings do.
fn explicit(names: &[&str]) -> SessionMcpSelection {
    SessionMcpSelection::Explicit(names.iter().map(|name| plugin(name).canonical()).collect())
}

/// An unrelated broken plugin cannot block a workflow, and empty selections need no capabilities.
#[test]
fn explicit_selection_filters_before_configuration_and_capability_checks() {
    let fixture = Fixture::new();
    let first = candidate(
        "first",
        Version::new(1, 0, 0),
        &fixture.package_root,
        stdio_config(),
    );
    let second = candidate(
        "second",
        Version::new(1, 0, 0),
        &fixture.package_root,
        stdio_config(),
    );
    let catalog = FakeCatalog::new(vec![first.clone(), second]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([(
            first.plugin_id.canonical(),
            McpConfigurationEligibility::NoSettings,
        )]),
    };
    let empty = resolve_session_mcp(
        &catalog,
        &configurations,
        &fixture.package_root,
        capabilities(/*load_session*/ false, /*http*/ false),
        &explicit(&[]),
    )
    .unwrap();
    assert_eq!(
        (empty.servers(), empty.revision()),
        (&[][..], &SessionMcpRevision::default())
    );
    let selected = resolve_session_mcp(
        &catalog,
        &configurations,
        &fixture.package_root,
        capabilities(/*load_session*/ true, /*http*/ false),
        &explicit(&["first"]),
    )
    .unwrap();
    let isolated = resolve_session_mcp(
        &FakeCatalog::new(vec![first]),
        &configurations,
        &fixture.package_root,
        capabilities(/*load_session*/ true, /*http*/ false),
        &SessionMcpSelection::Automatic,
    )
    .unwrap();
    assert_eq!(
        (selected.servers(), selected.revision()),
        (isolated.servers(), isolated.revision())
    );
    assert_eq!(
        resolve_session_mcp_revision(&catalog, &configurations, &explicit(&["second"])),
        Err(SessionMcpError::ConfigurationUnavailable {
            plugin_id: plugin("second")
        })
    );
}

/// Explicit dependencies fail closed while automatic chat discovery still omits incomplete ones.
#[test]
fn selected_missing_or_incomplete_plugins_fail_without_a_partial_set() {
    let fixture = Fixture::new();
    let catalog = FakeCatalog::new(vec![candidate(
        "first",
        Version::new(1, 0, 0),
        &fixture.package_root,
        stdio_config(),
    )]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([(
            plugin("first").canonical(),
            McpConfigurationEligibility::Incomplete,
        )]),
    };
    for selection in [explicit(&["first"]), explicit(&["missing"])] {
        let expected = if selection == explicit(&["first"]) {
            SessionMcpError::ConfigurationIncomplete {
                plugin_id: plugin("first"),
            }
        } else {
            SessionMcpError::SelectedPluginUnavailable {
                plugin_id: plugin("missing").canonical(),
            }
        };
        assert_eq!(
            resolve_session_mcp_revision(&catalog, &configurations, &selection),
            Err(expected.clone())
        );
        assert_eq!(
            resolve_session_mcp(
                &catalog,
                &configurations,
                &fixture.package_root,
                capabilities(/*load_session*/ true, /*http*/ true),
                &selection
            )
            .err(),
            Some(expected)
        );
    }
    assert_eq!(
        resolve_session_mcp_revision(&catalog, &configurations, &SessionMcpSelection::Automatic),
        Ok(SessionMcpRevision::default())
    );
}

/// Changes outside the allowlist cannot schedule a refresh or prevent a prompt.
#[test]
fn unselected_package_changes_do_not_change_the_desired_revision() {
    let fixture = Fixture::new();
    let selected = candidate(
        "first",
        Version::new(1, 0, 0),
        &fixture.package_root,
        stdio_config(),
    );
    let catalog = FakeCatalog::new(vec![selected.clone()]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([(
            plugin("first").canonical(),
            McpConfigurationEligibility::NoSettings,
        )]),
    };
    let before =
        resolve_session_mcp_revision(&catalog, &configurations, &explicit(&["first"])).unwrap();
    catalog.then(vec![
        selected,
        candidate(
            "unrelated",
            Version::new(2, 0, 0),
            &fixture.package_root,
            stdio_config(),
        ),
    ]);
    // Consume the previous catalog page before inspecting the updated installation.
    let _ = resolve_session_mcp_revision(&catalog, &configurations, &explicit(&["first"])).unwrap();
    assert_eq!(
        resolve_session_mcp_revision(&catalog, &configurations, &explicit(&["first"])),
        Ok(before)
    );
}

/// Selecting a plugin never bypasses the provider's required session-load capability.
#[test]
fn selected_plugins_require_agent_capabilities() {
    let fixture = Fixture::new();
    let catalog = FakeCatalog::new(vec![candidate(
        "first",
        Version::new(1, 0, 0),
        &fixture.package_root,
        stdio_config(),
    )]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([(
            plugin("first").canonical(),
            McpConfigurationEligibility::NoSettings,
        )]),
    };
    assert_eq!(
        resolve_session_mcp(
            &catalog,
            &configurations,
            &fixture.package_root,
            capabilities(/*load_session*/ false, /*http*/ false),
            &explicit(&["first"]),
        )
        .err(),
        Some(SessionMcpError::LoadCapabilityMissing)
    );
}
