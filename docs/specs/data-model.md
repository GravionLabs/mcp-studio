# Data Model

All state lives in one SQLite database in the app data directory, accessed via `sqlx` with
versioned migrations. Secrets are stored only as keyring references.

## Tables

| Table          | Key fields                                                                                                                                                                  |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `servers`      | id, name, transport (`stdio` \| `http`), command, args (JSON), env (JSON, secrets as `keyring:` references), cwd, url, headers (JSON), tags, created_at, updated_at         |
| `environments` | id, name, variables (JSON) — for `{{var}}` placeholders in args, headers, and tool arguments                                                                                |
| `collections`  | id, parent_id, name, sort_order — a folder tree                                                                                                                             |
| `requests`     | id, collection_id, server_id, method, tool_name, arguments (JSON), notes                                                                                                    |
| `history`      | id, server_id, method, target, arguments (JSON, as entered), is_error, cancelled, duration_ms, result (JSON, secrets masked; large results replaced by a marker), error, ts |
| `sessions`     | id, server_id, origin (`studio` \| `proxy`), started_at, ended_at, protocol_version, server_info (JSON), capabilities (JSON)                                                |
| `messages`     | id, session_id, span_id, direction (`out` \| `in`), jsonrpc_id, method, payload (JSON), bytes, is_error, tokens, token_source (`estimate` \| `exact`), ts                   |
| `spans`        | id, trace_id, parent_id, kind (`session` \| `flow` \| `step` \| `llm` \| `tool`), name, started_at, ended_at, status, attributes (JSON)                                     |
| `flows`        | id, name, graph (JSON), version, updated_at                                                                                                                                 |
| `prices`       | model, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok, currency                                                                                 |

## Rules

- `messages.payload` stores the raw message as received, so the inspector can show exactly what was
  on the wire.
- Placeholders (`{{var}}`) are resolved at call time; the resolved value is recorded in `messages`,
  but secrets are masked before storage.
- A secret in `servers.env` or `servers.headers` is written as `keyring:<service>/<key>` and resolved
  only in Rust when the process or request starts.
- History retention is configurable on the Settings page (default: 30 days or 100,000 messages, whichever comes first). The limits are stored in the `settings` table under `retention` and applied at startup and when saved.

## Export and import

- Collections export as JSON files and flows as YAML files, so teams can share them in Git.
- Server definitions export without secrets; keyring references are kept as placeholders.
- The whole workspace (servers, environments, collections, saved requests, flows, test suites,
  prices) exports to one JSON file (`workspace.rs`). The file holds the stored rows, so it names
  secrets (`keyring:<name>`) but never their values; history, sessions and settings stay local.
  Import shows what would be added or replaced, applies it in one transaction, never deletes, and
  lists the secrets that are missing from the keyring. Environments match by name, everything else
  by id.

## Migrations

| Migration                       | Adds                                          |
| ------------------------------- | --------------------------------------------- |
| `0001_init`                     | All tables of the first version               |
| `0002_message_flags`            | `messages.is_error` for cheap error filtering |
| `0003_history`                  | The `history` table                           |
| `0004_server_oauth`             | `servers.oauth`                               |
| `0009_server_azure_credentials` | `servers.azure_credentials`                   |

## Keyring entries

Secrets are stored under the service `dev.gravionlabs.mcp-studio`:

- `keyring:<name>` references in `servers.env`, `servers.headers`, and `environments.variables`;
- `oauth/<server id>`: the OAuth credentials (JSON) of one server;
- names beginning with `imported/` come from the client import.
