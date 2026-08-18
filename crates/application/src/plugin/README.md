# Plugin Application Module

This module owns the persistence port for plugin lifecycle intent. Filesystem discovery remains
the authority for installed plugin identity; the repository stores only whether a discovered
plugin is eligible to run plus its audit timestamps.

Lifecycle orchestration, process ownership, and reconciliation belong to
`ora-plugin-lifecycle`. Concrete event publication composition belongs to `ora-backend`, while
SQLite details belong to `ora-db`.
