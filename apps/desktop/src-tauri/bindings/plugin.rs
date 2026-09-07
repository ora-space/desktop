//! Desktop bindings for plugin.

use super::{Binding, Permission};

pub(super) const BINDINGS: &[Binding] = &[
    Binding::Unary {
        operation: "listAvailablePlugins",
        handler: "commands::plugin::list_available_plugins",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "syncAvailablePlugins",
        handler: "commands::plugin::sync_available_plugins",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "readPluginReadme",
        handler: "commands::plugin::read_plugin_readme",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listMarketplaceSources",
        handler: "commands::plugin::list_marketplace_sources",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "addMarketplaceSource",
        handler: "commands::plugin::add_marketplace_source",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "deleteMarketplaceSource",
        handler: "commands::plugin::delete_marketplace_source",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "updateMarketplaceSource",
        handler: "commands::plugin::update_marketplace_source",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "listInstalledPlugins",
        handler: "commands::plugin::list_installed_plugins",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "getPluginConfiguration",
        handler: "commands::plugin::get_plugin_configuration",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "savePluginConfiguration",
        handler: "commands::plugin::save_plugin_configuration",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "resetPluginConfiguration",
        handler: "commands::plugin::reset_plugin_configuration",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "scanPlugins",
        handler: "commands::plugin::scan_plugins",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "activatePlugin",
        handler: "commands::plugin::activate_plugin",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "stopPlugin",
        handler: "commands::plugin::stop_plugin",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "uninstallPlugin",
        handler: "commands::plugin::uninstall_plugin",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "installPlugin",
        handler: "commands::plugin::install_plugin",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "updatePlugin",
        handler: "commands::plugin::update_plugin",
        permission: Permission::MainWebview,
    },
    Binding::Unary {
        operation: "importPlugin",
        handler: "commands::plugin::import_plugin",
        permission: Permission::MainWebview,
    },
];
