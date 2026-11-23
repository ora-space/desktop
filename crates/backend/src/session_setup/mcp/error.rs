//! Desktop error classification for the shared, secret-free resolver.
pub(crate) use ora_agent_runtime::SessionMcpError;

/// Converts a shared resolver diagnostic into Desktop's native error type.
pub(crate) fn into_backend(error: SessionMcpError) -> crate::BackendError {
    let classification = match error {
        SessionMcpError::LoadCapabilityMissing | SessionMcpError::HttpCapabilityMissing { .. } => {
            crate::ErrorClassification::Conflict
        }
        SessionMcpError::SettingMissing { .. }
        | SessionMcpError::IllegalRuntimeText { .. }
        | SessionMcpError::CommandNotInPackage { .. }
        | SessionMcpError::WorkspaceCwdUnresolved { .. }
        | SessionMcpError::RevisionChanged { .. }
        | SessionMcpError::ConfigurationUnavailable { .. }
        | SessionMcpError::CatalogUnavailable
        | SessionMcpError::SelectedPluginUnavailable { .. }
        | SessionMcpError::ConfigurationIncomplete { .. } => {
            crate::ErrorClassification::Unprocessable
        }
    };
    crate::BackendError::new(classification, error.public_error(), error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{SessionMcpError, into_backend};
    use crate::ErrorClassification;
    use ora_agent_runtime::SessionMcpTransportKind;
    use ora_domain::PluginId;
    use pretty_assertions::assert_eq;

    /// Preserves the Desktop HTTP classification across the shared-resolver extraction.
    #[test]
    fn capability_errors_are_conflicts_and_all_other_setup_errors_are_unprocessable() {
        let plugin_id = || PluginId::parse("official/acme.mcp").unwrap();
        let errors = vec![
            SessionMcpError::LoadCapabilityMissing,
            SessionMcpError::HttpCapabilityMissing {
                plugin_id: plugin_id(),
            },
            SessionMcpError::SettingMissing {
                plugin_id: plugin_id(),
                setting_id: "token".into(),
                transport: SessionMcpTransportKind::Stdio,
            },
            SessionMcpError::IllegalRuntimeText {
                plugin_id: plugin_id(),
                setting_id: "token".into(),
                transport: SessionMcpTransportKind::Stdio,
            },
            SessionMcpError::CommandNotInPackage {
                plugin_id: plugin_id(),
            },
            SessionMcpError::WorkspaceCwdUnresolved {
                plugin_id: plugin_id(),
            },
            SessionMcpError::RevisionChanged { plugin_id: None },
            SessionMcpError::ConfigurationUnavailable {
                plugin_id: plugin_id(),
            },
            SessionMcpError::CatalogUnavailable,
            SessionMcpError::SelectedPluginUnavailable {
                plugin_id: plugin_id(),
            },
            SessionMcpError::ConfigurationIncomplete {
                plugin_id: plugin_id(),
                transport: SessionMcpTransportKind::Http,
            },
        ];

        let classifications = errors
            .into_iter()
            .map(|error| into_backend(error).classification())
            .collect::<Vec<_>>();

        assert_eq!(
            classifications,
            vec![
                ErrorClassification::Conflict,
                ErrorClassification::Conflict,
                ErrorClassification::Unprocessable,
                ErrorClassification::Unprocessable,
                ErrorClassification::Unprocessable,
                ErrorClassification::Unprocessable,
                ErrorClassification::Unprocessable,
                ErrorClassification::Unprocessable,
                ErrorClassification::Unprocessable,
                ErrorClassification::Unprocessable,
                ErrorClassification::Unprocessable,
            ]
        );
    }
}
