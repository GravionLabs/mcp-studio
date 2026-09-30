# Data Model

All state lives in one SQLite database in the app data directory, accessed via `sqlx` with
versioned migrations. Secrets are stored only as keyring references.

## Tables

| Table | Key fields |
|---|---|
| `servers` | id, name, transport (`stdio` \| `http`), command, args (JSON), env (JSON, secrets as `keyring:` references), cwd, url, headers (JSON), tags, created_at, updated_at |
| `environments` | id, name, variables (JSON) — for `{{var}}` placeholders in args, headers, and tool arguments |
| `collections` | id, parent_id, name, sort_order — a folder tree |
| `requests` | id, collection_id, server_id, method, tool_name, arguments (JSON), notes |
| `sessions` | id, server_id, origin (`studio` \| `proxy`), started_at, ended_at, protocol_version, server_info (JSON), capabilities (JSON) |
| `messages` | id, session_id, span_id, direction (`out` \| `in`), jsonrpc_id, method, payload (JSON), bytes, tokens, token_source (`estimate` \| `exact`), ts |
| `spans` | id, trace_id, parent_id, kind (`session` \| `flow` \| `step` \| `llm` \| `tool`), name, start, end, status, attributes (JSON) |
| `flows` | id, name, graph (JSON), version, updated_at |
| `prices` | model, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok, currency |

## Rules

- `messages.payload` stores the raw message as received, so the inspector can show exactly what was
  on the wire.
- Placeholders (`{{var}}`) are resolved at call time; the resolved value is recorded in `messages`,
  but secrets are masked before storage.
- A secret in `servers.env` or `servers.headers` is written as `keyring:<service>/<key>` and resolved
  only in Rust when the process or request starts.
- History retention is configurable (default: 30 days or 100,000 messages, whichever comes first).

## Export and import

- Collections export as JSON files and flows as YAML files, so teams can share them in Git.
- Server definitions export without secrets; keyring references are kept as placeholders.
