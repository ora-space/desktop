//! Pure open/migrate/close state machine for one surface instance.
//!
//! [`apply_command`] and [`apply_completion`] never perform I/O: they return the next state plus
//! the [`SurfaceEffect`]s the host must execute outside any lock. Rejected inputs return an error
//! and leave the caller's state untouched, so the registry can apply transitions atomically.

#[cfg(test)]
mod tests;

use crate::definition::{MountTarget, SurfaceDefinition};
use crate::events::SurfaceEvent;
use crate::ids::{OperationId, SurfaceInstanceId, ViewGeneration};
use thiserror::Error;

/// Lifecycle state of one instance. `Closed` is not a state: an instance that leaves the
/// registry is closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceState {
    /// A webview is being created; `close_requested` remembers a close that arrived meanwhile.
    Opening {
        target: MountTarget,
        operation: OperationId,
        view: ViewGeneration,
        close_requested: bool,
    },
    Embedded {
        visible: bool,
        view: ViewGeneration,
    },
    /// The window label equals the webview label by construction, so it is not repeated here.
    Windowed {
        view: ViewGeneration,
    },
    /// The webview is being reparented; `close_requested` remembers a close that arrived meanwhile.
    Migrating {
        from: MountTarget,
        to: MountTarget,
        operation: OperationId,
        view: ViewGeneration,
        close_requested: bool,
    },
    /// The webview is being destroyed; `from` is where it was mounted (`None` if it never was).
    Closing {
        operation: OperationId,
        from: Option<MountTarget>,
    },
    Failed {
        target: MountTarget,
        reason: String,
    },
}

/// Inputs originating from the user or the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceCommand {
    Close,
    Popout,
    Dock,
    SetVisible(bool),
    /// The webview process crashed or must be recreated.
    Rebuild,
}

/// Inputs reporting the end of an asynchronous operation the host executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceCompletion {
    Opened {
        operation: OperationId,
        outcome: Result<MountTarget, String>,
    },
    Migrated {
        operation: OperationId,
        outcome: Result<MountTarget, String>,
    },
    Closed {
        operation: OperationId,
    },
}

/// Side effect the host must execute; the state machine only decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceEffect {
    CreateWebview {
        target: MountTarget,
        operation: OperationId,
    },
    Reparent {
        to: MountTarget,
        operation: OperationId,
    },
    DestroyWebview {
        operation: OperationId,
    },
    SetNativeVisibility(bool),
    Emit(SurfaceEvent),
}

/// Result of one accepted transition. `next == None` means the instance has ended and must be
/// removed from the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub next: Option<SurfaceState>,
    pub effects: Vec<SurfaceEffect>,
}

/// Identity needed to project events; borrowed from the registry record during a transition.
#[derive(Clone, Copy, Debug)]
pub struct TransitionContext<'a> {
    pub instance: SurfaceInstanceId,
    pub definition: &'a SurfaceDefinition,
}

/// Why a command was refused. The state is unchanged after either variant.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TransitionError {
    #[error("surface is busy ({current}); retry after the pending operation completes")]
    Busy { current: &'static str },
    #[error("command {command} is invalid while the surface is {current}")]
    InvalidForState {
        command: &'static str,
        current: &'static str,
    },
}

/// A completion whose ticket does not match the pending operation; callers only log it.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("stale {completion} completion for operation {received:?} while {current}")]
pub struct StaleCompletion {
    pub completion: &'static str,
    pub received: OperationId,
    pub current: &'static str,
}

/// Applies a command to the current state.
///
/// `next_operation` is only invoked when the transition really starts a new asynchronous
/// operation, so refused commands do not burn tickets.
pub fn apply_command(
    context: TransitionContext<'_>,
    state: &SurfaceState,
    command: SurfaceCommand,
    next_operation: impl FnOnce() -> OperationId,
) -> Result<Transition, TransitionError> {
    let current = state_name(state);
    let invalid = |command: &'static str| TransitionError::InvalidForState { command, current };
    match (state, command) {
        // Close while an operation is pending is remembered and honored on completion.
        (
            SurfaceState::Opening {
                target,
                operation,
                view,
                ..
            },
            SurfaceCommand::Close,
        ) => Ok(stay(SurfaceState::Opening {
            target: *target,
            operation: *operation,
            view: *view,
            close_requested: true,
        })),
        (
            SurfaceState::Migrating {
                from,
                to,
                operation,
                view,
                ..
            },
            SurfaceCommand::Close,
        ) => Ok(stay(SurfaceState::Migrating {
            from: *from,
            to: *to,
            operation: *operation,
            view: *view,
            close_requested: true,
        })),
        (
            SurfaceState::Opening { .. } | SurfaceState::Migrating { .. },
            SurfaceCommand::Popout | SurfaceCommand::Dock,
        ) => Err(TransitionError::Busy { current }),
        (
            SurfaceState::Opening { .. } | SurfaceState::Migrating { .. },
            SurfaceCommand::SetVisible(_) | SurfaceCommand::Rebuild,
        ) => Err(invalid(command_name(command))),

        (SurfaceState::Embedded { .. }, SurfaceCommand::Close) => {
            Ok(close_from(Some(MountTarget::Embedded), next_operation()))
        }
        (SurfaceState::Windowed { .. }, SurfaceCommand::Close) => {
            Ok(close_from(Some(MountTarget::Windowed), next_operation()))
        }
        (SurfaceState::Embedded { view, .. }, SurfaceCommand::Popout) => Ok(migrate(
            MountTarget::Embedded,
            MountTarget::Windowed,
            *view,
            next_operation(),
        )),
        (SurfaceState::Windowed { view }, SurfaceCommand::Dock) => Ok(migrate(
            MountTarget::Windowed,
            MountTarget::Embedded,
            *view,
            next_operation(),
        )),
        (SurfaceState::Embedded { .. }, SurfaceCommand::Dock)
        | (SurfaceState::Windowed { .. }, SurfaceCommand::Popout) => {
            Err(invalid(command_name(command)))
        }
        (SurfaceState::Embedded { view, .. }, SurfaceCommand::SetVisible(visible)) => {
            Ok(Transition {
                next: Some(SurfaceState::Embedded {
                    visible,
                    view: *view,
                }),
                effects: vec![SurfaceEffect::SetNativeVisibility(visible)],
            })
        }
        (SurfaceState::Windowed { .. }, SurfaceCommand::SetVisible(_)) => {
            Err(invalid("set_visible"))
        }
        // A rebuild destroys and recreates under one ticket; the view generation advances so
        // stale page-level callbacks can be discarded by the host.
        (SurfaceState::Embedded { view, .. }, SurfaceCommand::Rebuild) => Ok(rebuild(
            MountTarget::Embedded,
            view.next(),
            next_operation(),
        )),
        (SurfaceState::Windowed { view }, SurfaceCommand::Rebuild) => Ok(rebuild(
            MountTarget::Windowed,
            view.next(),
            next_operation(),
        )),

        // Closing an instance that is already closing is idempotent.
        (SurfaceState::Closing { .. }, SurfaceCommand::Close) => Ok(stay(state.clone())),
        (
            SurfaceState::Closing { .. },
            SurfaceCommand::Popout
            | SurfaceCommand::Dock
            | SurfaceCommand::SetVisible(_)
            | SurfaceCommand::Rebuild,
        ) => Err(invalid(command_name(command))),

        // A failed instance has no webview, so closing it ends it immediately.
        (SurfaceState::Failed { .. }, SurfaceCommand::Close) => Ok(Transition {
            next: None,
            effects: vec![SurfaceEffect::Emit(SurfaceEvent::Closed {
                instance: context.instance.value(),
            })],
        }),
        (SurfaceState::Failed { target, .. }, SurfaceCommand::Rebuild) => {
            let operation = next_operation();
            Ok(Transition {
                next: Some(SurfaceState::Opening {
                    target: *target,
                    operation,
                    view: ViewGeneration::INITIAL,
                    close_requested: false,
                }),
                effects: vec![SurfaceEffect::CreateWebview {
                    target: *target,
                    operation,
                }],
            })
        }
        (
            SurfaceState::Failed { .. },
            SurfaceCommand::Popout | SurfaceCommand::Dock | SurfaceCommand::SetVisible(_),
        ) => Err(invalid(command_name(command))),
    }
}

/// Applies the completion of an asynchronous operation.
///
/// A ticket mismatch yields [`StaleCompletion`] instead of a transition because late callbacks
/// from a superseded operation must never move the instance. `next_operation` is only invoked
/// when a remembered close turns the completion into a destroy request.
pub fn apply_completion(
    context: TransitionContext<'_>,
    state: &SurfaceState,
    completion: SurfaceCompletion,
    next_operation: impl FnOnce() -> OperationId,
) -> Result<Transition, StaleCompletion> {
    let current = state_name(state);
    let stale = |completion: &'static str, received: OperationId| StaleCompletion {
        completion,
        received,
        current,
    };
    let instance = context.instance.value();
    match (state, completion) {
        (
            SurfaceState::Opening {
                operation: pending,
                view,
                close_requested,
                target: requested,
            },
            SurfaceCompletion::Opened { operation, outcome },
        ) if *pending == operation => Ok(match (outcome, *close_requested) {
            // A close requested during opening tears the fresh webview down right away; no
            // `Opened` event is emitted because the frontend never gets to use the instance.
            (Ok(target), true) => close_from(Some(target), next_operation()),
            (Ok(target), false) => Transition {
                next: Some(mounted(target, *view)),
                effects: vec![SurfaceEffect::Emit(SurfaceEvent::Opened {
                    instance,
                    plugin_id: context.definition.plugin_id.to_string(),
                    kind: context.definition.kind(),
                    target,
                    title: context.definition.title.clone(),
                })],
            },
            (Err(_), true) => Transition {
                next: None,
                effects: vec![SurfaceEffect::Emit(SurfaceEvent::Closed { instance })],
            },
            (Err(reason), false) => Transition {
                next: Some(SurfaceState::Failed {
                    target: *requested,
                    reason: reason.clone(),
                }),
                effects: vec![SurfaceEffect::Emit(SurfaceEvent::Failed {
                    instance,
                    reason,
                })],
            },
        }),
        (
            SurfaceState::Migrating {
                operation: pending,
                from,
                view,
                close_requested,
                ..
            },
            SurfaceCompletion::Migrated { operation, outcome },
        ) if *pending == operation => Ok(match (outcome, *close_requested) {
            (Ok(target), true) => close_from(Some(target), next_operation()),
            (Ok(target), false) => Transition {
                next: Some(mounted(target, *view)),
                effects: vec![SurfaceEffect::Emit(SurfaceEvent::Migrated {
                    instance,
                    target,
                })],
            },
            // A failed migration leaves the webview where it was; the host only needs to tell
            // the frontend so it can drop its optimistic layout change.
            (Err(reason), true) => {
                let mut transition = close_from(Some(*from), next_operation());
                transition.effects.insert(
                    0,
                    SurfaceEffect::Emit(SurfaceEvent::MigrateFailed { instance, reason }),
                );
                transition
            }
            (Err(reason), false) => Transition {
                next: Some(mounted(*from, *view)),
                effects: vec![SurfaceEffect::Emit(SurfaceEvent::MigrateFailed {
                    instance,
                    reason,
                })],
            },
        }),
        (
            SurfaceState::Closing {
                operation: pending, ..
            },
            SurfaceCompletion::Closed { operation },
        ) if *pending == operation => Ok(Transition {
            next: None,
            effects: vec![SurfaceEffect::Emit(SurfaceEvent::Closed { instance })],
        }),
        (
            SurfaceState::Opening { .. }
            | SurfaceState::Embedded { .. }
            | SurfaceState::Windowed { .. }
            | SurfaceState::Migrating { .. }
            | SurfaceState::Closing { .. }
            | SurfaceState::Failed { .. },
            SurfaceCompletion::Opened { operation, .. },
        ) => Err(stale("opened", operation)),
        (
            SurfaceState::Opening { .. }
            | SurfaceState::Embedded { .. }
            | SurfaceState::Windowed { .. }
            | SurfaceState::Migrating { .. }
            | SurfaceState::Closing { .. }
            | SurfaceState::Failed { .. },
            SurfaceCompletion::Migrated { operation, .. },
        ) => Err(stale("migrated", operation)),
        (
            SurfaceState::Opening { .. }
            | SurfaceState::Embedded { .. }
            | SurfaceState::Windowed { .. }
            | SurfaceState::Migrating { .. }
            | SurfaceState::Closing { .. }
            | SurfaceState::Failed { .. },
            SurfaceCompletion::Closed { operation },
        ) => Err(stale("closed", operation)),
    }
}

/// Builds a transition that changes state without side effects.
fn stay(next: SurfaceState) -> Transition {
    Transition {
        next: Some(next),
        effects: vec![],
    }
}

/// Builds the steady state for a mounted webview; a freshly mounted embedded view is visible.
fn mounted(target: MountTarget, view: ViewGeneration) -> SurfaceState {
    match target {
        MountTarget::Embedded => SurfaceState::Embedded {
            visible: true,
            view,
        },
        MountTarget::Windowed => SurfaceState::Windowed { view },
    }
}

/// Builds the Closing state together with the destroy request that drives it.
fn close_from(from: Option<MountTarget>, operation: OperationId) -> Transition {
    Transition {
        next: Some(SurfaceState::Closing { operation, from }),
        effects: vec![SurfaceEffect::DestroyWebview { operation }],
    }
}

/// Builds the Migrating state together with its reparent request.
fn migrate(
    from: MountTarget,
    to: MountTarget,
    view: ViewGeneration,
    operation: OperationId,
) -> Transition {
    Transition {
        next: Some(SurfaceState::Migrating {
            from,
            to,
            operation,
            view,
            close_requested: false,
        }),
        effects: vec![SurfaceEffect::Reparent { to, operation }],
    }
}

/// Builds the Opening state of a rebuilt webview: destroy the old one, then create a new one.
fn rebuild(target: MountTarget, view: ViewGeneration, operation: OperationId) -> Transition {
    Transition {
        next: Some(SurfaceState::Opening {
            target,
            operation,
            view,
            close_requested: false,
        }),
        effects: vec![
            SurfaceEffect::DestroyWebview { operation },
            SurfaceEffect::CreateWebview { target, operation },
        ],
    }
}

/// Names a state for diagnostics.
fn state_name(state: &SurfaceState) -> &'static str {
    match state {
        SurfaceState::Opening { .. } => "opening",
        SurfaceState::Embedded { .. } => "embedded",
        SurfaceState::Windowed { .. } => "windowed",
        SurfaceState::Migrating { .. } => "migrating",
        SurfaceState::Closing { .. } => "closing",
        SurfaceState::Failed { .. } => "failed",
    }
}

/// Names a command for diagnostics.
fn command_name(command: SurfaceCommand) -> &'static str {
    match command {
        SurfaceCommand::Close => "close",
        SurfaceCommand::Popout => "popout",
        SurfaceCommand::Dock => "dock",
        SurfaceCommand::SetVisible(_) => "set_visible",
        SurfaceCommand::Rebuild => "rebuild",
    }
}
