use crate::reader::{HistoryIntegrity, SessionHistory};
use crate::record::HistoryRecord;
use agent_client_protocol_schema::v1::ContentBlock;
use agent_client_protocol_schema::v1::SessionUpdate;
use agent_client_protocol_schema::v1::StopReason;
use agent_client_protocol_schema::v1::ToolCallStatus;
use ora_domain::AgentRef;
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
    if turns.is_empty() && history.integrity == HistoryIntegrity::Complete {
        return None;
    }

    let mut rendered = String::with_capacity(4096);
    rendered.push_str(OPEN_MARKER);
    rendered.push_str("\nThis conversation was previously handled by a different coding agent");
    if let Some(previous) = previous_agent(history) {
        let _ = write!(rendered, " ({previous})");
    }
    match history.integrity {
        HistoryIntegrity::Complete => rendered.push_str(
            ". The transcript below is its complete history, and the work it describes has \
             already been done — build on it instead of repeating it. Tool calls are listed \
             by name and outcome only; their inputs and outputs are not included. Continue \
             from where the conversation left off. The user's new message follows this block.\n",
        ),
        HistoryIntegrity::Damaged { unreadable_lines } => {
            let count = unreadable_lines.get();
            let noun = if count == 1 { "record" } else { "records" };
            let _ = writeln!(
                rendered,
                ". This transcript is incomplete: {count} history {noun} could not be decoded, \
                 and the original positions are unknown. Do not assume continuity. The surviving \
                 work has already been done — build on it instead of repeating it. Tool calls are \
                 listed by name and outcome only; their inputs and outputs are not included. \
                 Continue from where the conversation left off. The user's new message follows \
                 this block.",
            );
        }
    }

    for (index, turn) in turns.iter().enumerate() {
        let _ = write!(rendered, "\n## Turn {number}\n", number = index + 1);
        turn.render_into(&mut rendered);
    }

    rendered.push_str(CLOSE_MARKER);
    Some(rendered)
}

/// Reports whether the session's current provider binding has yet to be given the history.
///
/// Switching agents replaces the binding eagerly but injects the transcript
/// lazily, so this is the question "has the agent now serving this conversation
/// been shown what came before it". It is answered from the record rather than a
/// stored flag, because the flag lives in an actor that a lost connection or a
/// restart discards while the debt it tracked survives.
///
/// The two records it reads are the only ones that can settle the question, and
/// the most recent of them wins: `HandoffDelivered` is written after a provider
/// accepted the transcript, and `AgentSwitched` opens a debt that nothing has
/// paid yet. A user turn deliberately settles nothing — Ora records a prompt
/// before sending it, so one sitting after a switch is equally consistent with a
/// delivered transcript and with a `session/prompt` that never left the process.
///
/// A switch whose delivery the file cannot prove therefore reads as still owed,
/// which is the safe direction: handing a transcript over twice costs the
/// successor a repeated history, while skipping it leaves the agent answering a
/// conversation it cannot see. Reaching the start of the file is not that case —
/// a session that never switched has no binding to bring up to date.
pub fn binding_needs_handoff(history: &SessionHistory) -> bool {
    for line in history.lines.iter().rev() {
        match &line.record {
            HistoryRecord::HandoffDelivered { .. } => return false,
            HistoryRecord::AgentSwitched(_) => return true,
            HistoryRecord::Meta(_)
            | HistoryRecord::Update { .. }
            | HistoryRecord::TurnEnded { .. }
            | HistoryRecord::Gap { .. } => {}
        }
    }
    false
}

/// Names the agent whose work the transcript describes.
///
/// The most recent switch says which agent was handling the conversation before
/// it; a session that never switched has only the agent it opened on.
fn previous_agent(history: &SessionHistory) -> Option<AgentRef> {
    history
        .lines
        .iter()
        .rev()
        .find_map(|line| match &line.record {
            HistoryRecord::AgentSwitched(switch) => Some(switch.from.clone()),
            HistoryRecord::Meta(meta) => Some(meta.agent_ref.clone()),
            HistoryRecord::Update { .. }
            | HistoryRecord::TurnEnded { .. }
            | HistoryRecord::HandoffDelivered { .. }
            | HistoryRecord::Gap { .. } => None,
        })
}

/// One prompt turn reduced to the parts worth carrying to another agent.
#[derive(Default)]
struct HandoffTurn {
    user: Vec<String>,
    assistant: Vec<String>,
    /// Tool titles paired with the status recorded for them.
    ///
    /// The status is kept rather than rendered on arrival because how it reads
    /// depends on how the turn ended, which is only known once the turn closes.
    tools: Vec<(String, ToolCallStatus)>,
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
            let tools = self
                .tools
                .iter()
                .map(|(title, status)| {
                    format!(
                        "{title} ({status})",
                        status = describe_status(*status, self.stop_reason),
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(rendered, "\n**Tools:** {tools}\n");
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
            // StopReason is non-exhaustive; retaining a neutral note avoids claiming
            // completion semantics for a reason introduced by a newer ACP artifact.
            Some(_) => {
                rendered.push_str(
                    "\n_This turn ended for a reason this Ora version does not recognize._\n",
                );
            }
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
                from = switch.from,
                to = switch.to,
            )),
            HistoryRecord::Gap { reason } => current.notes.push(format!(
                "Part of the conversation is missing here because it could not be recorded ({reason})."
            )),
            // Bookkeeping about how this record reached an agent, not part of the
            // conversation the next one is being shown.
            HistoryRecord::Meta(_) | HistoryRecord::HandoffDelivered { .. } => {}
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
            turn.tools.push((call.title.clone(), call.status));
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
        // Future ACP updates are preserved in history but cannot be projected into a
        // handoff until this Ora version understands their semantics.
        _ => {}
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
        // ContentBlock is non-exhaustive, so name future displayable content without
        // guessing at a representation that could change its meaning.
        _ => "[unsupported content]".to_string(),
    }
}

/// Names one tool's outcome as the successor should read it.
///
/// ACP does not require an agent to report a terminal status, so a call left at
/// `pending` or `in_progress` is not evidence that it never ran — an agent that
/// simply moved on leaves exactly that behind. Reading it back as "never started"
/// would state as fact something the record never said, so an unfinished call is
/// reported as unreported instead.
///
/// How the turn ended is what separates the two ways that happens: a turn cut
/// short may genuinely have interrupted the call, while one the agent ended on
/// its own terms means the work almost certainly ran and only its result went
/// unsaid. Either way the successor is told what is unknown rather than being
/// handed a result nobody observed.
fn describe_status(status: ToolCallStatus, stop_reason: Option<StopReason>) -> &'static str {
    match status {
        ToolCallStatus::Completed => "completed",
        ToolCallStatus::Failed => "failed",
        ToolCallStatus::Pending | ToolCallStatus::InProgress => match stop_reason {
            Some(
                StopReason::Cancelled
                | StopReason::Refusal
                | StopReason::MaxTokens
                | StopReason::MaxTurnRequests,
            ) => "interrupted, outcome unknown",
            Some(StopReason::EndTurn) | None => "outcome not reported",
            Some(_) => "outcome not reported",
        },
        // A newer status is evidence that the provider reported something, but this
        // Ora version cannot safely label it completed or failed.
        _ => "status not recognized",
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
