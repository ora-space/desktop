//! Endpoint declarations for the plugin generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "plugin";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "listAvailablePlugins",
        namespace: NAMESPACE,
        member_name: "listAvailable",
        request_type: "ListAvailablePluginsRequest",
        response_type: "ListAvailablePluginsResponse",
    },
    FrontendEndpoint {
        operation_name: "syncAvailablePlugins",
        namespace: NAMESPACE,
        member_name: "syncAvailable",
        request_type: "SyncAvailablePluginsRequest",
        response_type: "SyncAvailablePluginsResponse",
    },
    FrontendEndpoint {
        operation_name: "listInstalledPlugins",
        namespace: NAMESPACE,
        member_name: "listInstalled",
        request_type: "ListInstalledPluginsRequest",
        response_type: "ListInstalledPluginsResponse",
    },
    FrontendEndpoint {
        operation_name: "scanPlugins",
        namespace: NAMESPACE,
        member_name: "scan",
        request_type: "ScanPluginsRequest",
        response_type: "ScanPluginsResponse",
    },
    FrontendEndpoint {
        operation_name: "enablePlugin",
        namespace: NAMESPACE,
        member_name: "enable",
        request_type: "EnablePluginRequest",
        response_type: "EnablePluginResponse",
    },
    FrontendEndpoint {
        operation_name: "disablePlugin",
        namespace: NAMESPACE,
        member_name: "disable",
        request_type: "DisablePluginRequest",
        response_type: "DisablePluginResponse",
    },
    FrontendEndpoint {
        operation_name: "activatePlugin",
        namespace: NAMESPACE,
        member_name: "activate",
        request_type: "ActivatePluginRequest",
        response_type: "ActivatePluginResponse",
    },
    FrontendEndpoint {
        operation_name: "stopPlugin",
        namespace: NAMESPACE,
        member_name: "stop",
        request_type: "StopPluginRequest",
        response_type: "StopPluginResponse",
    },
    FrontendEndpoint {
        operation_name: "uninstallPlugin",
        namespace: NAMESPACE,
        member_name: "uninstall",
        request_type: "UninstallPluginRequest",
        response_type: "UninstallPluginResponse",
    },
    FrontendEndpoint {
        operation_name: "installPlugin",
        namespace: NAMESPACE,
        member_name: "install",
        request_type: "InstallPluginRequest",
        response_type: "InstallPluginResponse",
    },
];
