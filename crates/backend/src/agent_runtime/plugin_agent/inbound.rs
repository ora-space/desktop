use ora_acp::AcpMessages;
use ora_logging::ora_warn;
use ora_plugin_runtime::PluginNotification;
use serde_json::Value;
use tokio::sync::mpsc;

use super::control::AGENT_ACP_METHOD;

/// Discards notifications that arrived before the agent was started.
///
/// Frames delivered before `agent/start` returned have no connection to be routed to. Draining
/// them here — rather than letting the channel buffer replay them into a connection that did not
/// exist when they were produced — keeps the stream that reaches the ACP peer aligned with one
/// live agent generation.
pub(super) fn discard_frames_before_start(
    notifications: &mut mpsc::UnboundedReceiver<PluginNotification>,
    plugin_id: &str,
) {
    let mut discarded = 0_usize;
    while notifications.try_recv().is_ok() {
        discarded += 1;
    }
    if discarded != 0 {
        ora_warn!(
            plugin_id = %plugin_id,
            discarded,
            "agent plugin sent frames before its agent was started"
        );
    }
}

/// Forwards `agent/acp` payloads into the message stream one ACP peer reads.
///
/// A single malformed or unrecognized notification is dropped with a warning instead of failing
/// the connection: the host is a pipe for these payloads, and letting one bad frame tear down
/// every session on the agent would trade a recoverable defect for an outage.
pub(super) fn spawn_frame_forwarding(
    mut notifications: mpsc::UnboundedReceiver<PluginNotification>,
    plugin_id: String,
) -> AcpMessages {
    let (sender, messages) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(notification) = notifications.recv().await {
            if notification.method != AGENT_ACP_METHOD {
                ora_warn!(
                    plugin_id = %plugin_id,
                    method = %notification.method,
                    "agent plugin sent a notification the agent runtime does not consume"
                );
                continue;
            }
            if !matches!(notification.params, Value::Object(_)) {
                ora_warn!(
                    plugin_id = %plugin_id,
                    "agent plugin sent an ACP frame that is not a JSON object"
                );
                continue;
            }
            if sender.send(Ok(notification.params)).is_err() {
                return;
            }
        }
    });
    messages
}
