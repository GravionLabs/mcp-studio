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
- **Inspect**: a live, filterable timeline of every JSON-RPC message with durations, sizes, a JSON
  detail view, and a structural diff between two messages.
- **Record real clients**: route Claude Code, Claude Desktop, or an IDE through MCP Studio with a local
  stdio proxy or HTTP proxy and watch the traffic. Ready-made configuration snippets included.
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
