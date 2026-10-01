# MCP Studio

A desktop tool for registering, testing, debugging, and orchestrating
[Model Context Protocol](https://modelcontextprotocol.io) servers — "Postman for MCP".

Built with Tauri 2, a Rust core, and Angular 22. Everything runs locally; secrets live in your OS
keyring.

> Status: MVP (v0.1). Flows, token metering, tracing, and AI assistance are planned for later
> milestones — see the [product spec](docs/specs/product-spec.md).

## Features

- **Register servers**: stdio (command, arguments, environment, working directory) and Streamable
  HTTP (URL, headers), with `{{variable}}` placeholders and environments. Secrets are stored in the OS
  keyring and never in the database.
- **Explore**: capabilities, tools (with a parameter table and schemas), resources (including URI
  templates), and prompts of a connected server; lists refresh when the server announces changes.
- **Test**: call tools with a form generated from the JSON Schema (or raw JSON in an editor), with
  validation, progress, and cancellation. Read resources and get prompts. Results are rendered by
  content type (text, JSON, images, audio, resources, structured content).
- **Inspect**: a live, filterable timeline of every JSON-RPC message with durations, sizes, estimated
  token counts for tool definitions, arguments, and results, a JSON
  detail view, and a structural diff between two messages.
- **Flows**: a flow combines LLM calls and tool calls. The **Flows** page keeps a library and
  imports and exports flows as YAML files for sharing in Git Run a flow from the Flows page: every tool call asks you first, and every run is stored and traced. The
  **Providers** page sets up the models flows use: Anthropic with your own API key, any
  OpenAI-compatible endpoint, and local Ollama.
- **Trace**: every session is a trace and every tool call a span. The **Traces** page shows a
  waterfall with duration, estimated tokens, and errors per span. Optionally export spans to an
  OpenTelemetry collector (off by default; only names, timing, status, and token estimates leave
  the app).
- **Cost**: every server shows how much context its tool definitions take, and calls and sessions
  show their estimated cost. You enter the price per million tokens of the models you use on the
  **Prices** page; none are built in because they change often. Token counts are estimates; with an
  Anthropic API key, **Count exactly** in the inspector replaces a message's estimate with the exact
  count (only when you click).
- **Record real clients**: route Claude Code, Claude Desktop, GitHub Copilot, OpenCode, or another
  client through MCP Studio with a local stdio proxy or HTTP proxy and watch the traffic.
  Ready-made configuration snippets included (see [Record a real client](#record-a-real-client)).
- **Updates**: **Check for updates** in the status bar installs signed updates from GitHub releases.
  Nothing is checked in the background.
- **Collections and history**: save requests in folders, share them as JSON files, and rerun anything
  from the history.

## Getting started

See [CONTRIBUTING.md](CONTRIBUTING.md) for the toolchain and the Linux system packages. Then:

```bash
pnpm install
cargo build --workspace
pnpm tauri dev
```

`pnpm bundle` builds installers including the proxy program as a sidecar.

## Record a real client

MCP Studio can sit between a real client and a server, record every JSON-RPC message in both
directions, and forward the bytes unchanged. Use it to see exactly what Claude Code, Claude Desktop,
GitHub Copilot, or OpenCode send to your server.

1. **Start MCP Studio and keep it running.** The proxy only works while the app is open. In
   development, `cargo build --workspace` also builds the proxy program, then run `pnpm tauri dev`.
2. **Register the real server** (or import it from a Claude config with the import button).
3. **Open the server's detail page** and scroll to **Record a real client**.
4. **Choose your client**, pick a setup variant (for Claude: the `claude mcp add` command,
   `.mcp.json`, or Claude Desktop), and press **Copy**.
5. **Put the snippet into the client's configuration** and restart the client if it needs it.
6. **Use the client as usual**, for example let it call a tool of the server.
7. **Watch the traffic** in the inspector. Recorded sessions are stored with origin `proxy`.

How the two transports work:

- **stdio servers**: the client starts `mcp-studio-proxy --server <name>` instead of the real server.
  That program connects to the running app, which starts the real server and records and forwards
  everything.
- **HTTP servers**: the client talks to `http://127.0.0.1:<port>/mcp/<server>`; MCP Studio forwards
  to the real URL and adds the server's headers.

If the panel says that `mcp-studio-proxy` was not found, run `cargo build -p mcp-studio-proxy` or
set `MCP_STUDIO_PROXY_BIN` to its path.

To try it without a real client, register the reference server
`target/debug/mcp-studio-testserver` (stdio; `--http <port>` for HTTP) and call its `echo` and `add`
tools. The design is described in
[Recording, proxy, token metering, and tracing](docs/specs/recording-and-observability.md).

## Documents

- [Product spec](docs/specs/product-spec.md)
- [Architecture](docs/specs/architecture.md)
- [Data model](docs/specs/data-model.md)
- [Recording, proxy, token metering, and tracing](docs/specs/recording-and-observability.md)
- [Prompt flows](docs/specs/flows.md)
- [ADR 0001 — Stack and architecture](docs/adr/0001-stack-and-architecture.md)
- [ADR 0002 — Record at the rmcp transport layer](docs/adr/0002-recording-at-the-rmcp-transport-layer.md)

## Issue hierarchy

Work is tracked as Epic → Feature → PBI → Task using GitHub's native sub-issues.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
