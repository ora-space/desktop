/**
 * Opaque authority attached by Ora to one plugin invocation.
 *
 * A page cannot create or widen this value. The host binds it to the calling plugin process,
 * process generation, and Ora session, and host APIs reject it outside that binding.
 */
export type { PluginInvocationContext } from "./protocol/index.ts";
