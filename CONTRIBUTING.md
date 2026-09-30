# Contributing to MCP Studio

Thanks for helping! This guide covers the setup, the workflow, and what a good pull request looks like.

## Setup

You need Node.js (see `.nvmrc`), pnpm, and a current stable Rust toolchain.

On Linux, Tauri needs system libraries:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libssl-dev \
  libdbus-1-dev libappindicator3-dev librsvg2-dev patchelf
```

```bash
pnpm install
cargo build --workspace          # also builds the reference server and the proxy program
pnpm tauri dev                   # starts the app with hot reload
```

## Layout

| Path                            | Contents                                                                      |
| ------------------------------- | ----------------------------------------------------------------------------- |
| `src/`                          | Angular frontend (`core/`, `features/`, `ui/`)                                |
| `src-tauri/`                    | Thin Tauri layer: commands, events, plugins                                   |
| `crates/mcp-studio-core/`       | Domain logic without Tauri: registry, sessions, recording, proxies, storage   |
| `crates/mcp-studio-llm/`        | LLM provider abstraction (later milestones)                                   |
| `crates/mcp-studio-proxy/`      | Program a client starts to reach a server through MCP Studio                  |
| `crates/mcp-studio-testserver/` | Deterministic reference MCP server (stdio and HTTP) used by integration tests |
| `docs/`                         | Product spec, architecture, data model, and architecture decision records     |

Read `docs/specs/product-spec.md` and `docs/specs/architecture.md` first.

## Checks

Run these before opening a pull request; CI runs the same:

```bash
pnpm lint && pnpm format:check && pnpm test && pnpm build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Types that cross the IPC boundary derive `specta::Type`. After changing them, run `pnpm bindings`
to regenerate `src/app/core/bindings.ts` (a test fails when it is stale).

## Workflow

Work is tracked as Epic → Feature → PBI → Task using GitHub sub-issues.

1. Pick an open PBI (or a bug) and create a branch named `<type>/<issue>-<short-description>`.
2. Write tests with the change: happy path, edge cases, and error cases. Logic that does not need
   Angular or Tauri belongs in plain functions or in `mcp-studio-core`, where it is easy to test.
3. Use [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, ...);
   the changelog is generated from them.
4. Open one pull request per PBI. Reference the PBI and its tasks with `Closes #N`.

## Principles

- **Local first**: no telemetry, no network calls except to the servers and LLM providers the user
  configured.
- **Secrets stay in the OS keyring**: the database and logs only ever contain `keyring:` references or
  masked values.
- **Rust owns connections**: the webview never spawns processes or sees credentials.
- **Nothing runs without consent**: tool calls from generated or automated flows need confirmation.

## License

By contributing you agree that your contribution is licensed under the same terms as the project:
[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at the user's option.
