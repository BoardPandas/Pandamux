<!-- PAGE_ID: pandamux_08_configuration -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [crates/pandamux-core/src/settings.rs:1-440](../../crates/pandamux-core/src/settings.rs#L1-L440)
- [crates/pandamux-core/src/organization.rs:8-194](../../crates/pandamux-core/src/organization.rs#L8-L194)
- [crates/pandamux-core/src/environment.rs:1-68](../../crates/pandamux-core/src/environment.rs#L1-L68)
- [crates/pandamux-core/src/provider_config.rs:1-149](../../crates/pandamux-core/src/provider_config.rs#L1-L149)
- [crates/pandamux-core/src/ssh.rs:1-341](../../crates/pandamux-core/src/ssh.rs#L1-L341)
- [crates/pandamux-providers/src/profiles.rs:1-145](../../crates/pandamux-providers/src/profiles.rs#L1-L145)
- [crates/pandamux-remote/src/config.rs:1-93](../../crates/pandamux-remote/src/config.rs#L1-L93)
- [crates/pandamux-server/src/runtime.rs:1-127](../../crates/pandamux-server/src/runtime.rs#L1-L127)

</details>

# Configuration

> **Related Pages**: [Core Domain](CORE_DOMAIN.md), [SSH Remote Nodes](../features/SSH_REMOTE.md), [Provider Routing and Capacity](../features/PROVIDER_ROUTING.md)

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_storage -->
## Configuration and Runtime Storage

PandaMUX separates durable user configuration from the short-lived runtime discovery record. `UserSettings` is the camelCase schema shared by the Settings UI and settings RPC helpers, while `server.json` tells clients where and how to authenticate to a running server ([settings.rs:1-8](../../crates/pandamux-core/src/settings.rs#L1-L8), [runtime.rs:7-27](../../crates/pandamux-server/src/runtime.rs#L7-L27)).

| Record | Contents | Lifecycle and protection |
|---|---|---|
| `UserSettings` | UI, terminal, keyboard, organizations, failover pools, tier mapping, provider instances, environments, and advanced settings ([settings.rs:20-39](../../crates/pandamux-core/src/settings.rs#L20-L39)) | Versioned and normalized after loading or mutation ([settings.rs:14-18](../../crates/pandamux-core/src/settings.rs#L14-L18), [settings.rs:222-292](../../crates/pandamux-core/src/settings.rs#L222-L292)) |
| `server.json` | PID, role, protocol and server versions, environment, endpoint, optional proxy path, control token, MCP token, ownership flag, and start time ([runtime.rs:9-27](../../crates/pandamux-server/src/runtime.rs#L9-L27)) | Atomically replaced, read only after validation, and removed on shutdown ([runtime.rs:29-80](../../crates/pandamux-server/src/runtime.rs#L29-L80)) |
| Provider profile directory | Per-instance provider login and tool state | Relative configured paths resolve beneath the PandaMUX base directory; absolute configured paths remain authoritative ([profiles.rs:16-39](../../crates/pandamux-providers/src/profiles.rs#L16-L39)) |

On Unix, runtime directories are created with mode `0700`, runtime records with mode `0600`, and reads reject symlinks, non-files, unsafe ownership, or group/other permission bits ([runtime.rs:31-59](../../crates/pandamux-server/src/runtime.rs#L31-L59), [runtime.rs:83-126](../../crates/pandamux-server/src/runtime.rs#L83-L126)).

Sources: [settings.rs:1-39](../../crates/pandamux-core/src/settings.rs#L1-L39), [runtime.rs:7-126](../../crates/pandamux-server/src/runtime.rs#L7-L126), [profiles.rs:16-39](../../crates/pandamux-providers/src/profiles.rs#L16-L39)
<!-- END:AUTOGEN pandamux_08_configuration_storage -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_settings -->
## Application Settings

The settings schema is at version 9. Every top-level field participates in Serde defaults, allowing older or partial serialized settings to gain current defaults before normalization ([settings.rs:14-39](../../crates/pandamux-core/src/settings.rs#L14-L39), [settings.rs:205-220](../../crates/pandamux-core/src/settings.rs#L205-L220)).

| Group | Important fields | Defaults and validation |
|---|---|---|
| `ui` | `theme`, `accent`, `showStatusBar`, sounds, notifications | Dark theme, teal accent, and all three booleans enabled ([settings.rs:295-320](../../crates/pandamux-core/src/settings.rs#L295-L320)) |
| `terminal` | Scrollback, welcome chooser, right-click paste, close confirmation, preferred shell | 10,000 lines, chooser enabled, right-click paste disabled, confirmation enabled, no preferred shell ([settings.rs:322-348](../../crates/pandamux-core/src/settings.rs#L322-L348)) |
| `keyboard` | `passThrough`, `overrides` | Empty collections; overrides map action IDs to a chord or `null` to unbind ([settings.rs:381-390](../../crates/pandamux-core/src/settings.rs#L381-L390)) |
| `advanced` | Global concurrency, checkpoint and attachment limits, usage scanning/sync, capacity routing, failover | Concurrency 8, 50 checkpoints, and all four Phase 4 switches enabled ([settings.rs:350-378](../../crates/pandamux-core/src/settings.rs#L350-L378)) |

`terminal.scrollbackLines` is clamped to 1,000 through 200,000. Zero global concurrency becomes 1, every environment execution limit is clamped from 1 through the global limit, and zero checkpoint retention becomes 10 during normalization ([settings.rs:16-18](../../crates/pandamux-core/src/settings.rs#L16-L18), [settings.rs:282-318](../../crates/pandamux-core/src/settings.rs#L282-L318)).

Settings RPCs use dotted camelCase paths. Reads reject unknown segments; writes reject an empty key, the `version` field, unknown paths, and values that cannot deserialize back into the full schema. Only `keyboard.overrides` permits insertion of a new leaf ([settings.rs:392-440](../../crates/pandamux-core/src/settings.rs#L392-L440)).

```rust
let value = settings_get(&settings, "terminal.scrollbackLines")?;
settings_set(&mut settings, "ui.theme", serde_json::json!("light"))?;
```

Sources: [settings.rs:14-39](../../crates/pandamux-core/src/settings.rs#L14-L39), [settings.rs:205-220](../../crates/pandamux-core/src/settings.rs#L205-L220), [settings.rs:282-440](../../crates/pandamux-core/src/settings.rs#L282-L440)
<!-- END:AUTOGEN pandamux_08_configuration_settings -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_organizations -->
## Organizations and Environments

Organizations define policy and budget boundaries, while environments identify the machine on which threads, worktrees, and schedules run. Both live inside `UserSettings` ([settings.rs:27-38](../../crates/pandamux-core/src/settings.rs#L27-L38)).

| Model | Key fields | Enforcement |
|---|---|---|
| `OrganizationPolicy` | Projects, allowed provider instances, allowed subscriptions, failover pools, tier mapping, concurrency, daily budget, Broadside policy, revision | Requires identifiers, display name, nonzero revision, nonempty provider and subscription allowlists, unique references, and valid tier targets ([organization.rs:168-194](../../crates/pandamux-core/src/organization.rs#L168-L194), [organization.rs:360-419](../../crates/pandamux-core/src/organization.rs#L360-L419)) |
| `FailoverPool` | Members, capability requirements, revision | Rejects empty pools, duplicate instances, missing models, preferences above 1,000, and capability mismatches ([organization.rs:36-62](../../crates/pandamux-core/src/organization.rs#L36-L62), [organization.rs:112-165](../../crates/pandamux-core/src/organization.rs#L112-L165)) |
| `Environment` | Kind, display name, status, platform, server version, schedule count, last error, execution limit, provider overrides | Local and SSH environments share one routing record. The execution limit defaults to 4 and is clamped to the trusted global limit ([environment.rs:4-35](../../crates/pandamux-core/src/environment.rs#L4-L35), [settings.rs:305-313](../../crates/pandamux-core/src/settings.rs#L305-L313)) |

The default environment is `env_local`, named `Local Machine`, with ready status, detected OS and architecture, current server version, an execution limit of 4, and no provider overrides ([environment.rs:37-54](../../crates/pandamux-core/src/environment.rs#L37-L54)). Environment kinds are `local` or `ssh` with a stable SSH profile ID; status distinguishes connecting, bootstrapping, ready, degraded, offline, disconnected, unreachable, and explicit error states ([environment.rs:57-79](../../crates/pandamux-core/src/environment.rs#L57-L79)).

Organization authorization is fail closed: provider instances and subscription keys must be allowlisted before routing, and failover candidates are hard-filtered by those allowlists before scoring ([organization.rs:421-492](../../crates/pandamux-core/src/organization.rs#L421-L492)).

Sources: [settings.rs:27-38](../../crates/pandamux-core/src/settings.rs#L27-L38), [settings.rs:282-318](../../crates/pandamux-core/src/settings.rs#L282-L318), [organization.rs:36-194](../../crates/pandamux-core/src/organization.rs#L36-L194), [organization.rs:360-492](../../crates/pandamux-core/src/organization.rs#L360-L492), [environment.rs:4-79](../../crates/pandamux-core/src/environment.rs#L4-L79)
<!-- END:AUTOGEN pandamux_08_configuration_organizations -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_providers -->
## Provider Instances and Profiles

A provider instance names one concrete provider configuration. Its identity is separate from the provider kind, enabling multiple profiles, environment-specific overrides, independent concurrency limits, and explicit disabled reasons ([provider_config.rs:6-27](../../crates/pandamux-core/src/provider_config.rs#L6-L27), [provider_config.rs:59-81](../../crates/pandamux-core/src/provider_config.rs#L59-L81)).

The built-in Codex default illustrates the serialized shape ([settings.rs:109-123](../../crates/pandamux-core/src/settings.rs#L109-L123)):

```json
{
  "id": "codex-default",
  "provider": "codex",
  "displayName": "Codex CLI",
  "profileDir": "profiles/codex/default",
  "concurrencyLimit": 3,
  "settings": { "defaultModel": "default" },
  "enabled": true
}
```

| Field | Meaning |
|---|---|
| `id` and `provider` | Stable instance identity and backend kind |
| `displayName` | User-facing instance label |
| `profileDir` | Relative-to-base or absolute isolated provider state directory |
| `environmentId` | Optional environment binding |
| `binaryPath` | Optional executable override |
| `concurrencyLimit` | Per-instance process limit, defaulting to 2 in the generic schema |
| `envOverrides` and `settings` | Provider-specific environment and JSON settings |
| `enabled` and `disabledReason` | Explicit availability plus persisted remediation context |

Environment overrides may replace enabled state, binary path, concurrency, environment variables, and provider settings. Resolution clones the base instance, layers the override, and binds the result to the requested environment ([provider_config.rs:43-57](../../crates/pandamux-core/src/provider_config.rs#L43-L57), [provider_config.rs:83-113](../../crates/pandamux-core/src/provider_config.rs#L83-L113)).

Profile environment variables are provider-specific:

| Provider | Isolated directory variable |
|---|---|
| Claude | `CLAUDE_CONFIG_DIR` |
| Codex | `CODEX_HOME` |
| Antigravity | `GEMINI_HOME` |
| Cursor | `CURSOR_CONFIG_DIR` |
| Grok | `GROK_HOME` |
| OpenCode | `OPENCODE_CONFIG_DIR` |

Antigravity additionally forces file storage and removes ambient Google credential variables before applying explicit instance overrides ([profiles.rs:49-89](../../crates/pandamux-providers/src/profiles.rs#L49-L89)).

Sources: [provider_config.rs:6-149](../../crates/pandamux-core/src/provider_config.rs#L6-L149), [profiles.rs:8-89](../../crates/pandamux-providers/src/profiles.rs#L8-L89)
<!-- END:AUTOGEN pandamux_08_configuration_providers -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_remote -->
## Remote Nodes

Saved SSH profiles are secretless connection descriptions. A profile records its stable ID, alias, host, port, user, authentication mode, and optional ProxyJump alias, but password mode records only that a prompt is required ([ssh.rs:1-24](../../crates/pandamux-core/src/ssh.rs#L1-L24), [ssh.rs:26-64](../../crates/pandamux-core/src/ssh.rs#L26-L64)).

| Saved auth mode | Persisted material | Runtime form |
|---|---|---|
| Agent | No key or password | Agent pipe or socket path ([ssh.rs:15-23](../../crates/pandamux-core/src/ssh.rs#L15-L23), [config.rs:11-14](../../crates/pandamux-remote/src/config.rs#L11-L14)) |
| Key file | Identity-file path | Key path plus an optional in-memory passphrase ([ssh.rs:20-21](../../crates/pandamux-core/src/ssh.rs#L20-L21), [config.rs:6-10](../../crates/pandamux-remote/src/config.rs#L6-L10)) |
| Password | No password | Password supplied to the runtime connection object ([ssh.rs:22-23](../../crates/pandamux-core/src/ssh.rs#L22-L23), [config.rs:15-17](../../crates/pandamux-remote/src/config.rs#L15-L17)) |

OpenSSH import accepts both `key value` and `key=value`, skips wildcard hosts, recognizes `HostName`, `User`, `Port`, `IdentityFile`, and `ProxyJump`, and defaults imported profiles to agent authentication when no identity file is present ([ssh.rs:129-200](../../crates/pandamux-core/src/ssh.rs#L129-L200)). Import into environments preserves existing records with the same display name or generated environment ID and creates new SSH environments in `Disconnected` state ([ssh.rs:281-341](../../crates/pandamux-core/src/ssh.rs#L281-L341)).

The runtime `SshConfig` adds connection-only state such as optional remote working directory, unknown-host trust, and a recursively configured jump host. New runtime configs default to port 22 and do not trust unknown hosts ([config.rs:20-63](../../crates/pandamux-remote/src/config.rs#L20-L63)).

Sources: [ssh.rs:1-200](../../crates/pandamux-core/src/ssh.rs#L1-L200), [ssh.rs:281-341](../../crates/pandamux-core/src/ssh.rs#L281-L341), [config.rs:3-63](../../crates/pandamux-remote/src/config.rs#L3-L63)
<!-- END:AUTOGEN pandamux_08_configuration_remote -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_security -->
## Secrets and Security Boundaries

Configuration records intentionally minimize persisted secrets, but they do not turn same-user provider execution into a hostile-process sandbox.

- Password SSH profiles persist only the authentication mode; the connect path must prompt for the secret ([ssh.rs:11-24](../../crates/pandamux-core/src/ssh.rs#L11-L24)).
- Provider login state is isolated by instance-specific directories and provider-specific environment variables ([profiles.rs:16-39](../../crates/pandamux-providers/src/profiles.rs#L16-L39), [profiles.rs:49-89](../../crates/pandamux-providers/src/profiles.rs#L49-L89)).
- The runtime record contains separate control and MCP tokens, so it must remain inside the private runtime directory ([runtime.rs:9-27](../../crates/pandamux-server/src/runtime.rs#L9-L27)).
- Unix runtime record reads reject symlinks, non-files, owner mismatches, and group/other access ([runtime.rs:100-126](../../crates/pandamux-server/src/runtime.rs#L100-L126)).
- Remote host-key failures distinguish unknown keys from changed keys and can carry fingerprints and known-hosts line numbers for explicit handling ([config.rs:66-93](../../crates/pandamux-remote/src/config.rs#L66-L93)).
- Remote clipboard reads are deny-by-default per host, while clipboard writes have a one MiB default cap ([ssh.rs:236-268](../../crates/pandamux-core/src/ssh.rs#L236-L268)).

Do not place raw credentials in `envOverrides` merely because the schema accepts strings. Those values are applied directly to provider child processes after isolation-specific variables are prepared ([profiles.rs:49-89](../../crates/pandamux-providers/src/profiles.rs#L49-L89)).

Sources: [ssh.rs:11-24](../../crates/pandamux-core/src/ssh.rs#L11-L24), [ssh.rs:236-268](../../crates/pandamux-core/src/ssh.rs#L236-L268), [profiles.rs:16-89](../../crates/pandamux-providers/src/profiles.rs#L16-L89), [runtime.rs:9-126](../../crates/pandamux-server/src/runtime.rs#L9-L126), [config.rs:66-93](../../crates/pandamux-remote/src/config.rs#L66-L93)
<!-- END:AUTOGEN pandamux_08_configuration_security -->

---
