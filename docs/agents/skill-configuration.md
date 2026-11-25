# Skill plugin configuration

English | [中文](skill-configuration.zh.md)

Skill plugins may declare Settings-only `assets/config.json`. The existing plugin configuration editor stores overrides in the host plugin data directory. Configuration does not change Skill Catalog publication or session Skill catalog delivery.

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

Environment names are `ORA_SKILL_<PLUGIN>_<FIELD>`. In the full plugin ID (including namespace) and field ID, ASCII alphanumerics are uppercased and every other Unicode character becomes one underscore; consecutive underscores are preserved. For example, `official/demo` and `token` produce `ORA_SKILL_OFFICIAL_DEMO_TOKEN`. Example Python:

```python
import os

token = os.environ.get("ORA_SKILL_OFFICIAL_DEMO_TOKEN")
if token is None:
    raise RuntimeError("Configure the demo plugin and restart Ora Desktop")
# Pass token to the service client. Do not print it.
```

Strings are delivered unchanged, numbers use JSON spelling, and booleans are `true`/`false`. Missing optional values are omitted. Incomplete, invalid or unreadable configurations, including any value containing NUL, contribute no variables for that plugin. Its Skill files remain available. An invalid plugin does not stop other Agents or plugins.

Snapshot capture first checks the names of every field in each valid declaration case-insensitively, including within and across plugins. Missing optional values and plugins with missing, corrupt, invalid or NUL-containing values still participate; invalid plugins or declarations do not. Value eligibility is evaluated after name collection. All involved plugins are omitted in full; diagnostics contain plugin and field IDs, never values. The host owns the prefix: matching inherited and caller-supplied spawn variables are removed case-insensitively before snapshot injection; spawn itself is allowed. Custom aliases are unsupported; unknown declaration members such as `env: GITHUB_TOKEN` are rejected.

## Security boundary

Configuration remains in host `store.json`; this feature does not add encryption or an OS keychain. Ora does not copy configured values into workspace files, ACP messages or host logs. Public setting values remain visible in the editor.

Every Agent receives the desktop-wide snapshot. This is **not** per-workspace or per-Skill secret isolation. Agents and scripts can read and print environment values; redaction does not prevent malicious scripts from leaking them. After reset or uninstall, existing processes retain the old snapshot until terminated. Revoke credentials at the external service when immediate revocation is required.

This contract covers Desktop-installed Skill plugins only, not ordinary imported Skills, Cloud or remote execution. Packages declaring `secret` require a host version supporting the field; older strict parsers may reject them.

The safety promise covers only the host delivery path. Values printed by scripts enter ACP messages, plugin logs, debug TRACE logs and session JSONL.
