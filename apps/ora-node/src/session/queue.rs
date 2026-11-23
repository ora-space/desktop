//! What a session does next, read from the commands its ledger holds.

use super::ports::{QueuedCommand, SessionCommand};
use ora_node_protocol::{CommandId, EndSessionReason, UserTurn};

/// What the ledger's queue asks the session to do next.
pub(super) enum Plan {
    /// Run this queued user turn.
    Turn {
        command_id: CommandId,
        turn: UserTurn,
    },
    /// End the session; every turn queued ahead of the end is discarded.
    End {
        command_id: CommandId,
        reason: EndSessionReason,
        discarded: Vec<CommandId>,
    },
    Wait,
}

/// Reads the queue in acceptance order; an `EndSession` anywhere in it takes precedence.
///
/// `CancelTurn` entries are not turns: the conversation settles them itself, and an end settles
/// the ones still queued as `session_ending` together with its decision, so they are neither run
/// nor discarded here.
pub(super) fn plan(queued: Vec<QueuedCommand>) -> Plan {
    let queued: Vec<_> = queued
        .into_iter()
        .filter(|queued| !matches!(queued.command, SessionCommand::CancelTurn(_)))
        .collect();
    let end = queued
        .iter()
        .position(|queued| matches!(queued.command, SessionCommand::EndSession(_)));
    let mut queued = queued.into_iter();
    if let Some(position) = end {
        let discarded = queued
            .by_ref()
            .take(position)
            .map(|queued| queued.command_id)
            .collect();
        if let Some(QueuedCommand {
            command_id,
            command: SessionCommand::EndSession(reason),
        }) = queued.next()
        {
            return Plan::End {
                command_id,
                reason,
                discarded,
            };
        }
    }
    match queued.next() {
        Some(QueuedCommand {
            command_id,
            command: SessionCommand::SubmitUserTurn(turn),
        }) => Plan::Turn { command_id, turn },
        Some(QueuedCommand {
            command: SessionCommand::EndSession(_) | SessionCommand::CancelTurn(_),
            ..
        })
        | None => Plan::Wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Builds a queued user turn.
    fn submit(command_id: &str) -> QueuedCommand {
        QueuedCommand {
            command_id: CommandId::new(command_id),
            command: SessionCommand::SubmitUserTurn(UserTurn {
                turn_id: ora_node_protocol::TurnId::new(command_id),
                content: Vec::new(),
            }),
        }
    }

    /// Builds a queued end.
    fn end(command_id: &str) -> QueuedCommand {
        QueuedCommand {
            command_id: CommandId::new(command_id),
            command: SessionCommand::EndSession(EndSessionReason::UserEnded),
        }
    }

    /// Reduces a plan to the command identities it acts on, for comparison.
    fn identities(plan: Plan) -> (&'static str, Vec<String>) {
        match plan {
            Plan::Turn { command_id, .. } => ("turn", vec![command_id.to_string()]),
            Plan::End {
                command_id,
                discarded,
                ..
            } => (
                "end",
                std::iter::once(command_id)
                    .chain(discarded)
                    .map(|id| id.to_string())
                    .collect(),
            ),
            Plan::Wait => ("wait", Vec::new()),
        }
    }

    /// The earliest turn runs first; an end anywhere in the queue wins and discards the turns
    /// accepted before it, while turns accepted after it are left for the ledger to settle.
    #[test]
    fn an_end_anywhere_in_the_queue_takes_precedence() {
        assert_eq!(
            [
                identities(plan(Vec::new())),
                identities(plan(vec![submit("a"), submit("b")])),
                identities(plan(vec![submit("a"), submit("b"), end("c"), submit("d")])),
            ],
            [
                ("wait", Vec::new()),
                ("turn", vec!["a".to_string()]),
                (
                    "end",
                    vec!["c".to_string(), "a".to_string(), "b".to_string()]
                ),
            ],
        );
    }
}
