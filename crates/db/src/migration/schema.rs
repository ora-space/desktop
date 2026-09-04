use super::Migration;

mod schema_v0001;
mod schema_v0002;
mod schema_v0003;
mod schema_v0004;
mod schema_v0005;
mod schema_v0006;
mod schema_v0007;
mod schema_v0008;
mod schema_v0009;

/// Distribution-specific initialization applied only to a brand-new database.
///
/// Public builds intentionally ship no data initialization. Internal distributions may patch this
/// list without changing versioned migration snapshots or the runner.
pub(super) const FIRST_INSTALL_SQL: &[&str] = &[];

/// Returns the ordered schema migrations shipped with the database crate.
pub(super) fn migrations() -> Vec<Migration> {
    vec![
        schema_v0001::migration(),
        schema_v0002::migration(),
        schema_v0003::migration(),
        schema_v0004::migration(),
        schema_v0005::migration(),
        schema_v0006::migration(),
        schema_v0007::migration(),
        schema_v0008::migration(),
        schema_v0009::migration(),
    ]
}
