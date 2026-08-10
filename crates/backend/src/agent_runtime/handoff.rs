use super::RuntimeActor;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_history::{read_session_history, render_handoff};
use ora_logging::{ora_debug, ora_warn};

/// Builds the provider prompt, injecting the recorded transcript only after an agent switch.
pub(super) fn prompt_for_agent(
    actor: &mut RuntimeActor,
    prompt: &[ContentBlock],
) -> Vec<ContentBlock> {
    if !actor.handoff_pending {
        return prompt.to_vec();
    }
    let history = match read_session_history(&actor.sessions_root, actor.session.id.as_ref()) {
        Ok(history) => history,
        Err(error) => {
            // Keep the flag set because recording this prompt is what proves the handoff
            // has been delivered. A transient read failure must not silently lose it.
            ora_warn!(
                session_id = %actor.session.id,
                error = %error,
                "handoff transcript unreadable; retrying on the next prompt",
            );
            return prompt.to_vec();
        }
    };
    actor.handoff_pending = false;
    // Nothing recorded means the session was switched before it was ever prompted.
    let Some(transcript) = render_handoff(&history) else {
        return prompt.to_vec();
    };
    ora_debug!(
        session_id = %actor.session.id,
        transcript_bytes = transcript.len(),
        "prepending recorded transcript for a new agent binding",
    );
    let mut sent = Vec::with_capacity(prompt.len() + 1);
    sent.push(ContentBlock::Text(TextContent::new(transcript)));
    sent.extend_from_slice(prompt);
    sent
}
