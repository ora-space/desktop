# Workspace Effect application module

This module maps transport-neutral contracts onto `ora-effect` domain values and delegates durable
Desired, status, and retry operations to the injected Effect repository.

Desired writes are complete replacements guarded by generation compare-and-swap. Exact no-op
replacements return the current generation, unavailable sources remain typed conflicts, and retry
only coalesces a wakeup at the current generation. Filesystem reconciliation, source validation,
and consumer coordination remain in `ora-effect`; SQLite remains in `ora-db`.
