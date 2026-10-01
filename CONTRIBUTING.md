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

1. Pick an open PBI (or a bug) and create a branch named `<type>/<issue>-<short-description>` from
   the epic's release branch (see [Releases](#releases)).
2. Write tests with the change: happy path, edge cases, and error cases. Logic that does not need
   Angular or Tauri belongs in plain functions or in `mcp-studio-core`, where it is easy to test.
3. Use [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, ...);
   the changelog is generated from them.
4. Open one pull request per PBI, **against the epic's release branch**. Reference the PBI and its
   tasks with `Closes #N`.

## Releases

Releases are cut per epic, not per pull request. There is one pipeline (`.github/workflows/ci.yml`):
every pull request runs the lint, test, and Rust checks, and every merge to `main` runs the same
checks, then builds installers for all platforms, then publishes a GitHub release. A failing check
stops the release. The version comes from GitVersion. So `main` only receives finished epics:

- Each epic has a **release branch** named `epic/<issue>-<short-description>`, created from `main`.
- **Every pull request targets the release branch**, not `main`. CI runs on all pull requests.
- When all PBIs of the epic are merged, open one pull request from the release branch into `main`
  and merge it with a **merge commit** (not squash) so the conventional commits reach the
  changelog. That merge publishes the release.
- Put `+semver: major` (or `minor`) in a commit message to bump that part.
- Merge `main` into the release branch if `main` changed in the meantime, and delete the release
  branch once it is merged.
- `CHANGELOG.md` is generated (`pnpm changelog`, git-cliff). After each release the pipeline
  regenerates it and commits it to `main` (`chore: update the changelog`).

The app updates itself from the latest release (`latest.json`, checked only when the user clicks
**Check for updates**). Update packages are signed with a Tauri updater key:

- the public key is in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`)
- the private key and its password are the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- **keep a backup of the private key and password**: if they are lost, installed apps can never
  update again and a new public key needs a manual reinstall

To make a new key: `pnpm tauri signer generate -w <file>`, then update the public key and both secrets.
Linux `.deb` and `.rpm` installs are updated through the package manager; the in-app updater
replaces AppImage, Windows, and macOS installs.

## Principles

- **Local first**: no telemetry, no network calls except to the servers and LLM providers the user
  configured, the update check the user starts themselves, the OpenTelemetry export the user turns
  on, and the exact token counts the user asks Anthropic for.
- **Secrets stay in the OS keyring**: the database and logs only ever contain `keyring:` references or
  masked values.
- **Rust owns connections**: the webview never spawns processes or sees credentials.
- **Nothing runs without consent**: tool calls from generated or automated flows need confirmation.

## License

By contributing you agree that your contribution is licensed under the same terms as the project:
[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at the user's option.
