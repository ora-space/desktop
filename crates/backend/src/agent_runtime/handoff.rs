use super::RuntimeActor;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_history::{read_session_history, render_handoff};
use ora_logging::{ora_debug, ora_warn};

/// The blocks to send a provider, paired with what their delivery would settle.
///
/// Building the prompt and settling the handoff are deliberately separate steps,
/// because the request between them can still fail. Nothing here acts on the
/// binding; this type only carries the decision as far as the send, where the
/// caller learns whether it may be applied.
pub(super) struct AgentPrompt {
    /// What goes on the wire, transcript included when one was injected.
    pub(super) blocks: Vec<ContentBlock>,
    /// Whether a provider accepting `blocks` brings the binding up to date.
    ///
    /// True even when the transcript rendered to nothing: a session switched
    /// before it was ever prompted has nothing to hand over and is current the
    /// moment its first prompt lands.
    pub(super) settles_handoff: bool,
}

/// Builds the provider prompt, injecting the recorded transcript only after an agent switch.
pub(super) fn prompt_for_agent(actor: &RuntimeActor, prompt: &[ContentBlock]) -> AgentPrompt {
    if !actor.handoff_pending {
        return AgentPrompt {
            blocks: prompt.to_vec(),
            settles_handoff: false,
        };
    }
    let history = match read_session_history(&actor.sessions_root, actor.session.id.as_ref()) {
        Ok(history) => history,
        Err(error) => {
            // Leaving the debt open is what makes a transient read failure cost a
            // retry rather than the transcript itself.
            ora_warn!(
                session_id = %actor.session.id,
                error = %error,
                "handoff transcript unreadable; retrying on the next prompt",
            );
            return AgentPrompt {
                blocks: prompt.to_vec(),
                settles_handoff: false,
            };
        }
    };
    // Nothing recorded means the session was switched before it was ever prompted.
    let Some(transcript) = render_handoff(&history) else {
        return AgentPrompt {
            blocks: prompt.to_vec(),
            settles_handoff: true,
        };
    };
    ora_debug!(
        session_id = %actor.session.id,
        transcript_bytes = transcript.len(),
        "prepending recorded transcript for a new agent binding",
    );
    let mut blocks = Vec::with_capacity(prompt.len() + 1);
    blocks.push(ContentBlock::Text(TextContent::new(transcript)));
    blocks.extend_from_slice(prompt);
    AgentPrompt {
        blocks,
        settles_handoff: true,
    }
}
