use super::Migration;

mod schema_v0001;
mod schema_v0002;
mod schema_v0003;
mod schema_v0004;
mod schema_v0005;
mod schema_v0006;
mod schema_v0007;

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
    ]
}
