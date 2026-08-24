use super::{
    SurfaceCommand, SurfaceCompletion, SurfaceEffect, SurfaceState, Transition, TransitionContext,
    TransitionError, apply_command, apply_completion,
};
use crate::definition::{MountTarget, RemoteSiteDefinition, SurfaceDefinition, SurfaceSource};
use crate::events::SurfaceEvent;
use crate::ids::{OperationId, SurfaceInstanceId, ViewGeneration};
use crate::navigation::NavigationPolicy;
use crate::state::StaleCompletion;
use ora_domain::PluginId;
use ora_plugin_manifest::DownloadPolicy;
use pretty_assertions::assert_eq;
use url::Url;

const PENDING: OperationId = OperationId::new(10);
const FRESH: OperationId = OperationId::new(11);
const OTHER: OperationId = OperationId::new(99);
const VIEW: ViewGeneration = ViewGeneration::INITIAL.next();

/// Builds a definition fixture shared by all table rows.
fn definition() -> SurfaceDefinition {
    SurfaceDefinition {
        plugin_id: PluginId::new("official", "acme.hub").expect("plugin id"),
        title: "Example Hub".to_owned(),
        source: SurfaceSource::RemoteSite(RemoteSiteDefinition {
            start_url: Url::parse("https://www.example.com").expect("valid url"),
            navigation: NavigationPolicy::remote_site(vec![]),
            download_policy: DownloadPolicy::default(),
        }),
    }
}

/// Runs a command with a fresh-ticket generator that must only fire when a new operation starts.
fn command(state: &SurfaceState, command: SurfaceCommand) -> Result<Transition, TransitionError> {
    let definition = definition();
    let context = TransitionContext {
        instance: SurfaceInstanceId::new(7),
        definition: &definition,
    };
    apply_command(context, state, command, || FRESH)
}

/// Runs a completion with a fresh-ticket generator for remembered closes.
fn completion(
    state: &SurfaceState,
    completion: SurfaceCompletion,
) -> Result<Transition, StaleCompletion> {
    let definition = definition();
    let context = TransitionContext {
        instance: SurfaceInstanceId::new(7),
        definition: &definition,
    };
    apply_completion(context, state, completion, || FRESH)
}

fn opening(target: MountTarget, close_requested: bool) -> SurfaceState {
    SurfaceState::Opening {
        target,
        operation: PENDING,
        view: VIEW,
        close_requested,
    }
}

fn embedded() -> SurfaceState {
    SurfaceState::Embedded {
        visible: true,
        view: VIEW,
    }
}

fn windowed() -> SurfaceState {
    SurfaceState::Windowed { view: VIEW }
}

fn migrating(close_requested: bool) -> SurfaceState {
    SurfaceState::Migrating {
        from: MountTarget::Embedded,
        to: MountTarget::Windowed,
        operation: PENDING,
        view: VIEW,
        close_requested,
    }
}

fn closing() -> SurfaceState {
    SurfaceState::Closing {
        operation: PENDING,
        from: Some(MountTarget::Embedded),
    }
}

fn failed() -> SurfaceState {
    SurfaceState::Failed {
        target: MountTarget::Embedded,
        reason: "crashed".to_owned(),
    }
}

fn closing_fresh(from: MountTarget) -> Transition {
    Transition {
        next: Some(SurfaceState::Closing {
            operation: FRESH,
            from: Some(from),
        }),
        effects: vec![SurfaceEffect::DestroyWebview { operation: FRESH }],
    }
}

fn ended() -> Transition {
    Transition {
        next: None,
        effects: vec![SurfaceEffect::Emit(SurfaceEvent::Closed { instance: 7 })],
    }
}

fn busy(current: &'static str) -> Result<Transition, TransitionError> {
    Err(TransitionError::Busy { current })
}

fn invalid(command: &'static str, current: &'static str) -> Result<Transition, TransitionError> {
    Err(TransitionError::InvalidForState { command, current })
}

fn stay(next: SurfaceState) -> Result<Transition, TransitionError> {
    Ok(Transition {
        next: Some(next),
        effects: vec![],
    })
}

fn opened(outcome: Result<MountTarget, &str>) -> SurfaceCompletion {
    SurfaceCompletion::Opened {
        operation: PENDING,
        outcome: outcome.map_err(str::to_owned),
    }
}

fn migrated(outcome: Result<MountTarget, &str>) -> SurfaceCompletion {
    SurfaceCompletion::Migrated {
        operation: PENDING,
        outcome: outcome.map_err(str::to_owned),
    }
}

fn closed() -> SurfaceCompletion {
    SurfaceCompletion::Closed { operation: PENDING }
}

/// Every (state, command) cell of the transition table.
#[test]
fn command_table() {
    use SurfaceCommand::{Close, Dock, Popout, Rebuild, SetVisible};
    let rebuilt = |target: MountTarget| {
        Ok(Transition {
            next: Some(SurfaceState::Opening {
                target,
                operation: FRESH,
                view: VIEW.next(),
                close_requested: false,
            }),
            effects: vec![
                SurfaceEffect::DestroyWebview { operation: FRESH },
                SurfaceEffect::CreateWebview {
                    target,
                    operation: FRESH,
                },
            ],
        })
    };
    let migrating_fresh = |from: MountTarget, to: MountTarget| {
        Ok(Transition {
            next: Some(SurfaceState::Migrating {
                from,
                to,
                operation: FRESH,
                view: VIEW,
                close_requested: false,
            }),
            effects: vec![SurfaceEffect::Reparent {
                to,
                operation: FRESH,
            }],
        })
    };
    let cases: Vec<(
        &str,
        SurfaceState,
        SurfaceCommand,
        Result<Transition, TransitionError>,
    )> = vec![
        // Opening
        (
            "opening/close",
            opening(MountTarget::Embedded, false),
            Close,
            stay(opening(MountTarget::Embedded, true)),
        ),
        (
            "opening/close twice",
            opening(MountTarget::Embedded, true),
            Close,
            stay(opening(MountTarget::Embedded, true)),
        ),
        (
            "opening/popout",
            opening(MountTarget::Embedded, false),
            Popout,
            busy("opening"),
        ),
        (
            "opening/dock",
            opening(MountTarget::Embedded, false),
            Dock,
            busy("opening"),
        ),
        (
            "opening/set_visible",
            opening(MountTarget::Embedded, false),
            SetVisible(false),
            invalid("set_visible", "opening"),
        ),
        (
            "opening/rebuild",
            opening(MountTarget::Embedded, false),
            Rebuild,
            invalid("rebuild", "opening"),
        ),
        // Embedded
        (
            "embedded/close",
            embedded(),
            Close,
            Ok(closing_fresh(MountTarget::Embedded)),
        ),
        (
            "embedded/popout",
            embedded(),
            Popout,
            migrating_fresh(MountTarget::Embedded, MountTarget::Windowed),
        ),
        (
            "embedded/dock",
            embedded(),
            Dock,
            invalid("dock", "embedded"),
        ),
        (
            "embedded/set_visible",
            embedded(),
            SetVisible(false),
            Ok(Transition {
                next: Some(SurfaceState::Embedded {
                    visible: false,
                    view: VIEW,
                }),
                effects: vec![SurfaceEffect::SetNativeVisibility(false)],
            }),
        ),
        (
            "embedded/rebuild",
            embedded(),
            Rebuild,
            rebuilt(MountTarget::Embedded),
        ),
        // Windowed
        (
            "windowed/close",
            windowed(),
            Close,
            Ok(closing_fresh(MountTarget::Windowed)),
        ),
        (
            "windowed/popout",
            windowed(),
            Popout,
            invalid("popout", "windowed"),
        ),
        (
            "windowed/dock",
            windowed(),
            Dock,
            migrating_fresh(MountTarget::Windowed, MountTarget::Embedded),
        ),
        (
            "windowed/set_visible",
            windowed(),
            SetVisible(true),
            invalid("set_visible", "windowed"),
        ),
        (
            "windowed/rebuild",
            windowed(),
            Rebuild,
            rebuilt(MountTarget::Windowed),
        ),
        // Migrating
        (
            "migrating/close",
            migrating(false),
            Close,
            stay(migrating(true)),
        ),
        (
            "migrating/popout",
            migrating(false),
            Popout,
            busy("migrating"),
        ),
        ("migrating/dock", migrating(false), Dock, busy("migrating")),
        (
            "migrating/set_visible",
            migrating(false),
            SetVisible(true),
            invalid("set_visible", "migrating"),
        ),
        (
            "migrating/rebuild",
            migrating(false),
            Rebuild,
            invalid("rebuild", "migrating"),
        ),
        // Closing
        ("closing/close", closing(), Close, stay(closing())),
        (
            "closing/popout",
            closing(),
            Popout,
            invalid("popout", "closing"),
        ),
        ("closing/dock", closing(), Dock, invalid("dock", "closing")),
        (
            "closing/set_visible",
            closing(),
            SetVisible(true),
            invalid("set_visible", "closing"),
        ),
        (
            "closing/rebuild",
            closing(),
            Rebuild,
            invalid("rebuild", "closing"),
        ),
        // Failed
        ("failed/close", failed(), Close, Ok(ended())),
        (
            "failed/popout",
            failed(),
            Popout,
            invalid("popout", "failed"),
        ),
        ("failed/dock", failed(), Dock, invalid("dock", "failed")),
        (
            "failed/set_visible",
            failed(),
            SetVisible(true),
            invalid("set_visible", "failed"),
        ),
        (
            "failed/rebuild",
            failed(),
            Rebuild,
            Ok(Transition {
                next: Some(SurfaceState::Opening {
                    target: MountTarget::Embedded,
                    operation: FRESH,
                    view: ViewGeneration::INITIAL,
                    close_requested: false,
                }),
                effects: vec![SurfaceEffect::CreateWebview {
                    target: MountTarget::Embedded,
                    operation: FRESH,
                }],
            }),
        ),
    ];
    for (name, state, input, expected) in cases {
        assert_eq!(command(&state, input), expected, "{name}");
    }
}

/// Every (state, completion) cell with a matching ticket.
#[test]
fn completion_table() {
    let opened_event = |target: MountTarget| {
        SurfaceEffect::Emit(SurfaceEvent::Opened {
            instance: 7,
            plugin_id: "official/acme.hub".to_owned(),
            kind: crate::definition::SurfaceKind::Webview,
            target,
            title: "Example Hub".to_owned(),
        })
    };
    let migrate_failed = SurfaceEffect::Emit(SurfaceEvent::MigrateFailed {
        instance: 7,
        reason: "denied".to_owned(),
    });
    let stale = |completion: &'static str, current: &'static str| {
        Err(StaleCompletion {
            completion,
            received: PENDING,
            current,
        })
    };
    let cases: Vec<(
        &str,
        SurfaceState,
        SurfaceCompletion,
        Result<Transition, StaleCompletion>,
    )> = vec![
        // Opening
        (
            "opening/opened ok embedded",
            opening(MountTarget::Embedded, false),
            opened(Ok(MountTarget::Embedded)),
            Ok(Transition {
                next: Some(embedded()),
                effects: vec![opened_event(MountTarget::Embedded)],
            }),
        ),
        (
            "opening/opened ok windowed",
            opening(MountTarget::Windowed, false),
            opened(Ok(MountTarget::Windowed)),
            Ok(Transition {
                next: Some(windowed()),
                effects: vec![opened_event(MountTarget::Windowed)],
            }),
        ),
        (
            "opening/opened ok with close remembered",
            opening(MountTarget::Embedded, true),
            opened(Ok(MountTarget::Embedded)),
            Ok(closing_fresh(MountTarget::Embedded)),
        ),
        (
            "opening/opened err",
            opening(MountTarget::Embedded, false),
            opened(Err("crashed")),
            Ok(Transition {
                next: Some(failed()),
                effects: vec![SurfaceEffect::Emit(SurfaceEvent::Failed {
                    instance: 7,
                    reason: "crashed".to_owned(),
                })],
            }),
        ),
        (
            "opening/opened err with close remembered",
            opening(MountTarget::Embedded, true),
            opened(Err("crashed")),
            Ok(ended()),
        ),
        (
            "opening/migrated",
            opening(MountTarget::Embedded, false),
            migrated(Ok(MountTarget::Windowed)),
            stale("migrated", "opening"),
        ),
        (
            "opening/closed",
            opening(MountTarget::Embedded, false),
            closed(),
            stale("closed", "opening"),
        ),
        // Embedded
        (
            "embedded/opened",
            embedded(),
            opened(Ok(MountTarget::Embedded)),
            stale("opened", "embedded"),
        ),
        (
            "embedded/migrated",
            embedded(),
            migrated(Ok(MountTarget::Windowed)),
            stale("migrated", "embedded"),
        ),
        (
            "embedded/closed",
            embedded(),
            closed(),
            stale("closed", "embedded"),
        ),
        // Windowed
        (
            "windowed/opened",
            windowed(),
            opened(Ok(MountTarget::Windowed)),
            stale("opened", "windowed"),
        ),
        (
            "windowed/migrated",
            windowed(),
            migrated(Ok(MountTarget::Embedded)),
            stale("migrated", "windowed"),
        ),
        (
            "windowed/closed",
            windowed(),
            closed(),
            stale("closed", "windowed"),
        ),
        // Migrating
        (
            "migrating/opened",
            migrating(false),
            opened(Ok(MountTarget::Windowed)),
            stale("opened", "migrating"),
        ),
        (
            "migrating/migrated ok",
            migrating(false),
            migrated(Ok(MountTarget::Windowed)),
            Ok(Transition {
                next: Some(windowed()),
                effects: vec![SurfaceEffect::Emit(SurfaceEvent::Migrated {
                    instance: 7,
                    target: MountTarget::Windowed,
                })],
            }),
        ),
        (
            "migrating/migrated ok with close remembered",
            migrating(true),
            migrated(Ok(MountTarget::Windowed)),
            Ok(closing_fresh(MountTarget::Windowed)),
        ),
        (
            "migrating/migrated err",
            migrating(false),
            migrated(Err("denied")),
            Ok(Transition {
                next: Some(embedded()),
                effects: vec![migrate_failed.clone()],
            }),
        ),
        (
            "migrating/migrated err with close remembered",
            migrating(true),
            migrated(Err("denied")),
            Ok(Transition {
                next: Some(SurfaceState::Closing {
                    operation: FRESH,
                    from: Some(MountTarget::Embedded),
                }),
                effects: vec![
                    migrate_failed,
                    SurfaceEffect::DestroyWebview { operation: FRESH },
                ],
            }),
        ),
        (
            "migrating/closed",
            migrating(false),
            closed(),
            stale("closed", "migrating"),
        ),
        // Closing
        (
            "closing/opened",
            closing(),
            opened(Ok(MountTarget::Embedded)),
            stale("opened", "closing"),
        ),
        (
            "closing/migrated",
            closing(),
            migrated(Ok(MountTarget::Embedded)),
            stale("migrated", "closing"),
        ),
        ("closing/closed", closing(), closed(), Ok(ended())),
        // Failed
        (
            "failed/opened",
            failed(),
            opened(Ok(MountTarget::Embedded)),
            stale("opened", "failed"),
        ),
        (
            "failed/migrated",
            failed(),
            migrated(Ok(MountTarget::Embedded)),
            stale("migrated", "failed"),
        ),
        (
            "failed/closed",
            failed(),
            closed(),
            stale("closed", "failed"),
        ),
    ];
    for (name, state, input, expected) in cases {
        assert_eq!(completion(&state, input), expected, "{name}");
    }
}

/// A matching completion kind with a different ticket is stale in every pending state.
#[test]
fn mismatched_tickets_are_stale() {
    let cases = [
        (
            opening(MountTarget::Embedded, false),
            SurfaceCompletion::Opened {
                operation: OTHER,
                outcome: Ok(MountTarget::Embedded),
            },
            "opened",
            "opening",
        ),
        (
            migrating(false),
            SurfaceCompletion::Migrated {
                operation: OTHER,
                outcome: Ok(MountTarget::Windowed),
            },
            "migrated",
            "migrating",
        ),
        (
            closing(),
            SurfaceCompletion::Closed { operation: OTHER },
            "closed",
            "closing",
        ),
    ];
    for (state, input, kind, current) in cases {
        assert_eq!(
            completion(&state, input),
            Err(StaleCompletion {
                completion: kind,
                received: OTHER,
                current
            }),
            "{kind}"
        );
    }
}
