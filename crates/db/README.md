# ora-db

`ora-db` is Ora's SQLite infrastructure crate. It owns database location, connection setup, schema reconciliation, and concrete implementations of the repository ports defined by `ora-application`.

## Module map

- [migration](src/migration/README.md) defines the ordered migration catalog and transactional reconciliation algorithm.
- [repository](src/repository/README.md) implements application repositories, pooled connection access, and aggregate soft-delete transactions.

## Bootstrap and boundaries

`DatabaseBootstrapper` opens either a path-backed database or a shared in-memory database, configures connection behavior, reconciles the schema, and returns a `RepositoryPool`. File-backed parent directories must already be prepared by the composition root.

The crate stores domain values and implements application ports; it does not own use-case policy, contract mapping, transport errors, Git cleanup, or provider history. Task diff comments are stored as either root threads with complete anchors or replies with only a parent id; the repository maps those mutually exclusive shapes without exposing SQL nullability. Timestamps are supplied through `TimestampSource`, with production time coming from Ora's local logging clock.

SQLite failures, invalid migration history, and bootstrap errors are normalized as `DatabaseError`.
Repository adapters preserve those concrete failures as the source of `ora-application`'s shared
`RepositoryError`; they do not stringify or log failures that an outer request seam will complete.
Repositories hide SQL rows and soft-delete columns from callers.

See [Database Migrations](../../docs/database-migrations.md) and [Application and Contracts Boundary](../../docs/application-contracts-boundary.md).
