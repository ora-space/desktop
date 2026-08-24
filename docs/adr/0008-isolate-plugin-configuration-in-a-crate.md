# Isolate Plugin Configuration in a crate

Ora places declaration compilation, value persistence, resolution, and their failure semantics behind the public API of `ora-plugin-config`. Installation management, lifecycle, backend transport, and UI orchestration remain consumers, because distributing file parsing and readiness rules among them would duplicate policy and make the runtime consumer disagree with the configuration editor.
