# Ora Plugin Configuration

`ora-plugin-config` owns the host rules for immutable plugin Setting Declarations and mutable,
plugin-global Stored Setting Values. Its public API compiles `assets/config.json`, resolves
effective values and Configuration Completeness, and persists revisioned `store.json` files.

The crate does not render UI, expose filesystem paths to frontend callers, start plugins, or pass
configuration to Agent processes. Callers supply package identity and roots; lifecycle and backend
layers map the resulting value-oriented types into their own contracts.

Declaration parsing is strict and bounded. Stored values are independent of installed versions,
writes replace the complete explicit override set atomically, and optimistic revision plus
declaration-fingerprint checks prevent stale editors from overwriting newer state.
Recovery preserves malformed files under a collision-free local-time backup name; if replacement
or restoration cannot complete, the caller receives an explicit failure instead of a recovered
snapshot.
