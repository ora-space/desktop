//! Desktop bindings for app events.

use super::Binding;

pub(super) const BINDINGS: &[Binding] = &[Binding::Stream {
    operation: "watchAppEvents",
    handler: "commands::app_events::start_watch",
}];
