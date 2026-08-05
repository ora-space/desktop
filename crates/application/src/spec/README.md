# Specification source application module

This module owns project-wide specification source override use cases. It validates transport input,
checks project ownership, creates audited domain snapshots, and asks the repository port to replace
the active set atomically. Filesystem discovery and target checkout resolution remain backend and
filesystem-adapter responsibilities.

An empty replacement clears every active override. Paths are stored as normalized workspace-relative
slash paths, and custom workflows must carry a non-blank display name.
