# Skill plugin configuration

English | [中文](skill-configuration.zh.md)

Skill plugins may declare Settings-only `assets/config.json`. The existing plugin configuration editor stores overrides in the host plugin data directory. Configuration does not change Skill Catalog publication or workspace materialization.

```json
{
  "schemaVersion": 1,
  "settings": {
    "endpoint": {
      "type": "string",
      "title": "Endpoint",
      "description": "Service URL",
      "default": "https://example.test"
    },
    "token": {
      "type": "string",
      "title": "Token",
      "description": "Service credential",
      "required": true,
      "secret": true
    }
  }
}
```

`secret` defaults to false and supports string, number and boolean settings. Secret values and defaults are hidden in editor responses; unchanged secrets are preserved on save. Reset field removes the stored override; Reset All restores defaults. MCP settings remain redacted regardless of this flag. Never publish real credentials as package defaults or descriptions.

## Reading values from scripts

Ora freezes the complete configuration of valid installed Skill plugins when Desktop starts. All Agent plugin processes and their host-spawned children receive that same snapshot, even across workspaces. Save, reset, install, update and uninstall affect injection only after restarting **Ora Desktop**, not merely opening a new conversation or restarting an Agent. The editor shows this reminder.

Environment names are `ORA_SKILL_P<uppercase UTF-8 hex canonical Plugin ID>_S<uppercase UTF-8 hex Setting ID>`. Use the full installed plugin ID, including its namespace. Example Python:

```python
import os

def setting_key(plugin_id, setting_id):
    return "ORA_SKILL_P" + plugin_id.encode("utf-8").hex().upper() + "_S" + setting_id.encode("utf-8").hex().upper()

token = os.environ.get(setting_key("official/demo", "token"))
if token is None:
    raise RuntimeError("Configure the demo plugin and restart Ora Desktop")
# Pass token to the service client. Do not print it.
```

Strings are delivered unchanged, numbers use JSON spelling, and booleans are `true`/`false`. Missing optional values are omitted. Incomplete, invalid or unreadable configurations, including any value containing NUL, contribute no variables for that plugin. Its Skill files remain available. An invalid plugin does not stop other Agents or plugins.

The prefix is host-owned and case-insensitively reserved. Spawn callers cannot override it. Inherited variables with this prefix are removed before applying the snapshot. The encoding preserves punctuation and case, including on Windows; arbitrary environment aliases are unsupported.

## Security boundary

Configuration remains in host `store.json`; this feature does not add encryption or an OS keychain. Ora does not copy configured values into workspace files, Effect state, ACP messages or host logs. Public setting values remain visible in the editor.

Every Agent receives the desktop-wide snapshot. This is **not** per-workspace or per-Skill secret isolation. Agents and scripts can read and print environment values; redaction does not prevent malicious scripts from leaking them. After reset or uninstall, existing processes retain the old snapshot until terminated. Revoke credentials at the external service when immediate revocation is required.

This contract covers Desktop-installed Skill plugins only, not ordinary imported Skills, Cloud or remote execution. Packages declaring `secret` require a host version supporting the field; older strict parsers may reject them.
