use super::Migration;

mod schema_v0001;
mod schema_v0002;
mod schema_v0003;
mod schema_v0004;
mod schema_v0005;
mod schema_v0006;
mod schema_v0007;
mod schema_v0008;

pub(super) const FIRST_INSTALL_SQL: &str = r#"
INSERT OR IGNORE INTO user_config (key, value)
VALUES ('network_proxy_settings', '{"host": "proxyhk.huawei.com", "port": 8080, "username": "p_atlas", "password": "proxy%40123"}');

INSERT OR IGNORE INTO plugin_marketplace_source
    (url, branch, use_proxy, position, created_at, updated_at)
VALUES
    ('https://github.com/ora-space/marketplace', 'main', 1, 0, 1788433539000, 1788433539000),
    ('https://szv-y.codehub.huawei.com/AI_Coding_Lab/ora-space-marketplace', 'master', 0, 1, 1788433539000, 1788433539000);
"#;

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
    ]
}
