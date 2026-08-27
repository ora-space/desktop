# ora-user-config

`ora-user-config` is the storage-neutral user-configuration boundary. It models
the `user_config` table as a small string key/value store without depending on
SQLite, Desktop, Tauri, or application-specific behavior.

The crate provides:

- `ConfigKey`, the stable set of persisted keys;
- `ConfigValue`, a raw value with reusable parsing helpers;
- `UserConfigRepository`, the generic get/set/delete persistence port;
- `UserConfigStore`, a thin facade for typed callers.

Callers own policy. `ora-application` interprets developer mode, runtime log
level, and network proxy settings, while Desktop Backend validates and resolves `worktree_root`. The SQLite
adapter in `ora-db` only stores raw values. This keeps adding a new setting from
requiring a database-specific repository method.
