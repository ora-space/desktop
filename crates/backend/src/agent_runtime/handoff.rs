use super::RuntimeActor;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_history::{read_session_history, render_handoff};
use ora_logging::{ora_debug, ora_warn};

/// Whether the current provider binding still has to be told the conversation, and whether
/// settling that debt is something the durable record has to remember.
///
/// The two owed states differ only in bookkeeping, never in what the agent receives. A switch
/// between agents writes `AgentSwitched` before the transcript is delivered, so an interrupted
/// delivery is still owed after a restart and its settlement writes a matching delivery line. A
/// provider session Ora rebuilt under the *same* agent — because the old one could no longer be
/// restored — writes nothing at all: the rebuilt binding is only persisted once the prompt
/// carrying the transcript is accepted, so an interrupted delivery leaves the old binding in the
/// row and the next prompt rebuilds and re-delivers on its own.
pub(super) enum HandoffDebt {
    /// Nothing owed: the provider behind this binding already holds the conversation.
    Settled,
    /// Owed, and an `AgentSwitched` line is waiting for its `HandoffDelivered`.
    Recorded,
    /// Owed in memory only, for a provider session rebuilt under the same agent.
    Ephemeral,
}

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

/// Builds the provider prompt, injecting the recorded transcript only when one is owed.
pub(super) fn prompt_for_agent(actor: &RuntimeActor, prompt: &[ContentBlock]) -> AgentPrompt {
    if let HandoffDebt::Settled = actor.handoff {
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
