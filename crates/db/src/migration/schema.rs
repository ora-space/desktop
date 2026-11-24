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
mod schema_v0010;
mod schema_v0011;
mod schema_v0012;
mod schema_v0013;
mod schema_v0014;
mod schema_v0015;
mod schema_v0016;
mod schema_v0017;
mod schema_v0018;
mod schema_v0019;
mod schema_v0020;
mod schema_v0021;
mod schema_v0022;
mod schema_v0023;
mod schema_v0024;

/// Initialization applied only to a brand-new database.
///
/// Existing databases never receive this pending seed marker: an empty source list in an old
/// installation is a user choice. Internal distributions may replace this initialization.
pub(super) const FIRST_INSTALL_SQL: &[&str] =
    &["INSERT INTO user_config (key, value) VALUES ('marketplace_sources_seed_pending', 'true');"];

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
        schema_v0010::migration(),
        schema_v0011::migration(),
        schema_v0012::migration(),
        schema_v0013::migration(),
        schema_v0014::migration(),
        schema_v0015::migration(),
        schema_v0016::migration(),
        schema_v0017::migration(),
        schema_v0018::migration(),
        schema_v0019::migration(),
        schema_v0020::migration(),
        schema_v0021::migration(),
        schema_v0022::migration(),
        schema_v0023::migration(),
        schema_v0024::migration(),
    ]
}
