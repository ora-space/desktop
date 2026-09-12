use ora_plugin_runtime::PluginRegistration;

/// Legacy method names emitted by agent plugins built with SDK 0.7.
pub(super) const LEGACY_AGENT_LIST_MODELS_METHOD: &str = "agent/listModels";
/// Legacy Effect method name emitted by agent plugins built with SDK 0.7.
pub(super) const LEGACY_EFFECT_VERIFY_READY_METHOD: &str = "effect/verifyReady";

/// Returns whether a plugin declares either the current or legacy spelling of a method.
pub(super) fn has_compatible_method(
    registration: &PluginRegistration,
    current: &str,
    legacy: &str,
) -> bool {
    registration.methods.contains(current) || registration.methods.contains(legacy)
}

/// Selects the method spelling declared by one plugin, preferring the current protocol.
pub(super) fn compatible_method(
    registration: &PluginRegistration,
    current: &'static str,
    legacy: &'static str,
) -> &'static str {
    if registration.methods.contains(current) {
        current
    } else if registration.methods.contains(legacy) {
        legacy
    } else {
        // Contract validation normally prevents this path; keeping the canonical name makes a
        // later invocation report the precise missing-method error if a caller violates that seam.
        current
    }
}
