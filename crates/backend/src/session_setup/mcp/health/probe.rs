//! Binds one eligible MCP member onto the shared MCP probe transport.
//!
//! The binding uses `resolve_mcp_transport` — the same product Session setup interprets for ACP —
//! so the object a probe speaks to is the object that would be delivered, not a second
//! configuration parse of its own.

use super::super::SessionMcpError;
use super::super::member::EligibleMcpMember;
use super::super::resolve::revalidate_stdio_command;
use ora_plugin_config::{
    ResolveMcpBindingError, ResolvedMcpArgument, ResolvedMcpTransport, resolve_mcp_transport,
};
use ora_utils::mcp::ProbeTransport;
use std::path::Path;

/// Converts the same binding product Session setup uses into a probe transport.
///
/// Failures here are resolution problems (a missing Setting, a command that left the package).
/// They are not product Unhealthy codes from a completed handshake, so they are returned as the
/// setup diagnostic instead of being flattened into the health vocabulary.
pub(crate) fn bind_probe_transport(
    member: &EligibleMcpMember,
    cwd: Option<&Path>,
) -> Result<ProbeTransport, SessionMcpError> {
    let resolved = resolve_mcp_transport(&member.candidate.configuration, &member.values).map_err(
        |error| match error {
            ResolveMcpBindingError::MissingSetting { setting_id } => {
                SessionMcpError::SettingMissing {
                    plugin_id: member.candidate.plugin_id.clone(),
                    setting_id,
                    transport: member.transport_kind(),
                }
            }
            ResolveMcpBindingError::IllegalRuntimeText { setting_id } => {
                SessionMcpError::IllegalRuntimeText {
                    plugin_id: member.candidate.plugin_id.clone(),
                    setting_id,
                    transport: member.transport_kind(),
                }
            }
        },
    )?;
    match resolved {
        ResolvedMcpTransport::Stdio { command, args, env } => {
            let command_path = revalidate_stdio_command(
                &member.candidate.package_root,
                &command,
                &member.candidate.plugin_id,
            )?;
            let mut mapped_args = Vec::with_capacity(args.len());
            for argument in args {
                mapped_args.push(match argument {
                    ResolvedMcpArgument::Literal(value) => value,
                    ResolvedMcpArgument::WorkspaceContext => {
                        let Some(cwd) = cwd.filter(|path| path.is_absolute()) else {
                            return Err(SessionMcpError::WorkspaceCwdUnresolved {
                                plugin_id: member.candidate.plugin_id.clone(),
                            });
                        };
                        cwd.to_string_lossy().into_owned()
                    }
                });
            }
            Ok(ProbeTransport::Stdio {
                command: command_path,
                args: mapped_args,
                env,
                // The process working directory is not part of ACP delivery: workspace context is
                // an argv token, so the probe must not invent a different cwd for the process.
                cwd: None,
            })
        }
        ResolvedMcpTransport::Http { url, headers } => Ok(ProbeTransport::Http { url, headers }),
    }
}
