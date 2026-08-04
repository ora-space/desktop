use crate::reader::SessionHistory;
use crate::record::HistoryRecord;
use ora_contracts::acp::content::ContentBlock;
use ora_contracts::acp::prompt::StopReason;
use ora_contracts::acp::session::SessionUpdate;
use ora_contracts::acp::tool_call::ToolCallStatus;
use ora_domain::AgentCli;
use std::fmt::Write as _;

/// Wraps the transcript so the receiving agent can tell it from the user's words.
const OPEN_MARKER: &str = "<ora_session_handoff>";
const CLOSE_MARKER: &str = "</ora_session_handoff>";

/// Renders one session's history as the block handed to a different agent.
///
/// ACP has no way to install a conversation into an agent: `session/new` takes no
/// context and `session/prompt` takes one user turn. Every turn Ora recorded
/// therefore collapses into a single user message, and the receiving agent sees a
/// transcript it is being shown rather than one it took part in. The preamble
/// exists to say so — without it the agent reads the transcript as a request.
///
/// Returns `None` when there is nothing to hand over, so a session switched
/// before it was ever prompted sends the user's message alone.
///
/// Which agent produced the work is read from the record rather than supplied by
/// the caller: the file already states it, and a caller re-deriving it is a
/// second answer that can disagree with the transcript it introduces.
pub fn render_handoff(history: &SessionHistory) -> Option<String> {
    let turns = collect_turns(history);
    if turns.is_empty() {
        return None;
    }

    let mut rendered = String::with_capacity(4096);
    rendered.push_str(OPEN_MARKER);
    rendered.push_str("\nThis conversation was previously handled by a different coding agent");
    if let Some(previous) = previous_agent(history) {
        let _ = write!(rendered, " ({name})", name = previous.executable_name());
    }
    rendered.push_str(
        ". The transcript below is its complete history, and the work it describes has \
         already been done — build on it instead of repeating it. Tool calls are listed \
         by name and outcome only; their inputs and outputs are not included. Continue \
         from where the conversation left off. The user's new message follows this block.\n",
    );

    for (index, turn) in turns.iter().enumerate() {
        let _ = write!(rendered, "\n## Turn {number}\n", number = index + 1);
        turn.render_into(&mut rendered);
    }

    rendered.push_str(CLOSE_MARKER);
    Some(rendered)
}

/// Reports whether the session's current provider binding has yet to be told the history.
///
/// Switching agents replaces the binding eagerly but injects the transcript
/// lazily, so this is the question "has anything been said to the agent now
/// serving this conversation". It is answered from the record rather than a
/// stored flag: a switch followed by no prompt is exactly a trailing
/// `AgentSwitched` with no user message after it, and that survives a restart
/// without anything having to remember it.
pub fn binding_needs_handoff(history: &SessionHistory) -> bool {
    for line in history.lines.iter().rev() {
        match &line.record {
            HistoryRecord::Update { update } => {
                // A prompt already reached whichever agent is serving now.
                if matches!(update, SessionUpdate::UserMessageChunk(_)) {
                    return false;
                }
            }
            HistoryRecord::AgentSwitched(_) => return true,
            HistoryRecord::Meta(_)
            | HistoryRecord::TurnEnded { .. }
            | HistoryRecord::Gap { .. } => {}
        }
    }
    false
}

/// Names the agent whose work the transcript describes.
///
/// The most recent switch says which agent was handling the conversation before
/// it; a session that never switched has only the CLI it opened on.
fn previous_agent(history: &SessionHistory) -> Option<AgentCli> {
    history
        .lines
        .iter()
        .rev()
        .find_map(|line| match &line.record {
            HistoryRecord::AgentSwitched(switch) => Some(switch.from),
            HistoryRecord::Meta(meta) => Some(meta.agent_cli),
            HistoryRecord::Update { .. }
            | HistoryRecord::TurnEnded { .. }
            | HistoryRecord::Gap { .. } => None,
        })
}

/// One prompt turn reduced to the parts worth carrying to another agent.
#[derive(Default)]
struct HandoffTurn {
    user: Vec<String>,
    assistant: Vec<String>,
    tools: Vec<String>,
    notes: Vec<String>,
    stop_reason: Option<StopReason>,
}

impl HandoffTurn {
    /// Reports whether this turn carries anything at all.
    fn is_empty(&self) -> bool {
        self.user.is_empty()
            && self.assistant.is_empty()
            && self.tools.is_empty()
            && self.notes.is_empty()
    }

    fn render_into(&self, rendered: &mut String) {
        if !self.user.is_empty() {
            let _ = write!(rendered, "\n**User:**\n{}\n", self.user.join("\n"));
        }
        if !self.assistant.is_empty() {
            let _ = write!(
                rendered,
                "\n**Assistant:**\n{}\n",
                self.assistant.join("\n")
            );
        }
        if !self.tools.is_empty() {
            let _ = write!(rendered, "\n**Tools:** {}\n", self.tools.join(", "));
        }
        for note in &self.notes {
            let _ = write!(rendered, "\n_{note}_\n");
        }
        // Only an outcome the successor could misread is worth stating: a turn
        // that simply ended needs no annotation, but one that was cut short
        // explains why its work looks unfinished.
        match self.stop_reason {
            Some(StopReason::Cancelled) => {
                rendered.push_str("\n_This turn was cancelled by the user before it finished._\n");
            }
            Some(StopReason::Refusal) => {
                rendered.push_str("\n_The previous agent refused to continue this turn._\n");
            }
            Some(StopReason::MaxTokens) | Some(StopReason::MaxTurnRequests) => {
                rendered.push_str("\n_This turn stopped at the previous agent's limits._\n");
            }
            Some(StopReason::EndTurn) | None => {}
        }
    }
}

/// Groups the recorded timeline into the turns the transcript is rendered from.
fn collect_turns(history: &SessionHistory) -> Vec<HandoffTurn> {
    let mut turns = Vec::new();
    let mut current = HandoffTurn::default();
    for line in &history.lines {
        match &line.record {
            HistoryRecord::Update { update } => absorb_update(&mut current, update),
            HistoryRecord::TurnEnded { stop_reason } => {
                current.stop_reason = Some(*stop_reason);
                if !current.is_empty() {
                    turns.push(std::mem::take(&mut current));
                } else {
                    current = HandoffTurn::default();
                }
            }
            HistoryRecord::AgentSwitched(switch) => current.notes.push(format!(
                "The conversation moved from {from} to {to} at this point.",
                from = switch.from.executable_name(),
                to = switch.to.executable_name(),
            )),
            HistoryRecord::Gap { reason } => current.notes.push(format!(
                "Part of the conversation is missing here because it could not be recorded ({reason})."
            )),
            HistoryRecord::Meta(_) => {}
        }
    }
    // A turn interrupted by a crash never got its boundary, but its work still
    // happened and the successor needs to know about it.
    if !current.is_empty() {
        turns.push(current);
    }
    turns
}

/// Folds one recorded update into the turn being rendered.
fn absorb_update(turn: &mut HandoffTurn, update: &SessionUpdate) {
    match update {
        SessionUpdate::UserMessageChunk(chunk) => {
            turn.user.push(describe_content(&chunk.content));
        }
        SessionUpdate::AgentMessageChunk(chunk) => {
            turn.assistant.push(describe_content(&chunk.content));
        }
        SessionUpdate::ToolCall(call) => {
            turn.tools.push(format!(
                "{title} ({status})",
                title = call.title,
                status = describe_status(call.status),
            ));
        }
        // Reasoning belongs to the agent that produced it and does not transfer.
        // Plans, tool results, and session chrome are either stale on arrival or
        // large enough to crowd out the conversation itself.
        SessionUpdate::AgentThoughtChunk(_)
        | SessionUpdate::ToolCallUpdate(_)
        | SessionUpdate::Plan(_)
        | SessionUpdate::AvailableCommandsUpdate(_)
        | SessionUpdate::CurrentModeUpdate(_)
        | SessionUpdate::ConfigOptionUpdate(_)
        | SessionUpdate::SessionInfoUpdate(_)
        | SessionUpdate::UsageUpdate(_) => {}
    }
}

/// Renders one content block as transcript text.
///
/// Anything that is not text is named rather than reproduced: the receiving agent
/// gains nothing from a re-encoded image, and the marker would be lost in it.
fn describe_content(content: &ContentBlock) -> String {
    match content {
        ContentBlock::Text(text) => neutralize_markers(&text.text),
        ContentBlock::Image(_) => "[image]".to_string(),
        ContentBlock::Audio(_) => "[audio]".to_string(),
        ContentBlock::ResourceLink(link) => format!("[resource link: {uri}]", uri = link.uri),
        ContentBlock::Resource(_) => "[embedded resource]".to_string(),
    }
}

fn describe_status(status: ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Pending => "never started",
        ToolCallStatus::InProgress => "did not finish",
        ToolCallStatus::Completed => "completed",
        ToolCallStatus::Failed => "failed",
    }
}

/// Stops transcript text from closing the block it is wrapped in.
///
/// A user who pasted the marker, or a previous handoff quoted back, would
/// otherwise end the section early and turn the remainder into instructions.
fn neutralize_markers(text: &str) -> String {
    text.replace(CLOSE_MARKER, "</ora_session_handoff\u{200b}>")
        .replace(OPEN_MARKER, "<ora_session_handoff\u{200b}>")
}
