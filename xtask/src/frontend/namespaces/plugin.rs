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
        operation_name: "readPluginReadme",
        namespace: NAMESPACE,
        member_name: "readReadme",
        request_type: "ReadPluginReadmeRequest",
        response_type: "ReadPluginReadmeResponse",
    },
    FrontendEndpoint {
        operation_name: "listMarketplaceSources",
        namespace: NAMESPACE,
        member_name: "listSources",
        request_type: "ListMarketplaceSourcesRequest",
        response_type: "ListMarketplaceSourcesResponse",
    },
    FrontendEndpoint {
        operation_name: "addMarketplaceSource",
        namespace: NAMESPACE,
        member_name: "addSource",
        request_type: "AddMarketplaceSourceRequest",
        response_type: "AddMarketplaceSourceResponse",
    },
    FrontendEndpoint {
        operation_name: "deleteMarketplaceSource",
        namespace: NAMESPACE,
        member_name: "deleteSource",
        request_type: "DeleteMarketplaceSourceRequest",
        response_type: "DeleteMarketplaceSourceResponse",
    },
    FrontendEndpoint {
        operation_name: "updateMarketplaceSource",
        namespace: NAMESPACE,
        member_name: "updateSource",
        request_type: "UpdateMarketplaceSourceRequest",
        response_type: "UpdateMarketplaceSourceResponse",
    },
    FrontendEndpoint {
        operation_name: "listInstalledPlugins",
        namespace: NAMESPACE,
        member_name: "listInstalled",
        request_type: "ListInstalledPluginsRequest",
        response_type: "ListInstalledPluginsResponse",
    },
    FrontendEndpoint {
        operation_name: "getPluginConfiguration",
        namespace: NAMESPACE,
        member_name: "getConfiguration",
        request_type: "GetPluginConfigurationRequest",
        response_type: "GetPluginConfigurationResponse",
    },
    FrontendEndpoint {
        operation_name: "savePluginConfiguration",
        namespace: NAMESPACE,
        member_name: "saveConfiguration",
        request_type: "SavePluginConfigurationRequest",
        response_type: "SavePluginConfigurationResponse",
    },
    FrontendEndpoint {
        operation_name: "resetPluginConfiguration",
        namespace: NAMESPACE,
        member_name: "resetConfiguration",
        request_type: "ResetPluginConfigurationRequest",
        response_type: "ResetPluginConfigurationResponse",
    },
    FrontendEndpoint {
        operation_name: "scanPlugins",
        namespace: NAMESPACE,
        member_name: "scan",
        request_type: "ScanPluginsRequest",
        response_type: "ScanPluginsResponse",
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
    FrontendEndpoint {
        operation_name: "updatePlugin",
        namespace: NAMESPACE,
        member_name: "update",
        request_type: "UpdatePluginRequest",
        response_type: "UpdatePluginResponse",
    },
    FrontendEndpoint {
        operation_name: "importPlugin",
        namespace: NAMESPACE,
        member_name: "import",
        request_type: "ImportPluginRequest",
        response_type: "ImportPluginResponse",
    },
];
