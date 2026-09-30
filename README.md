# MCP Studio

A desktop tool for registering, testing, debugging, and orchestrating
[Model Context Protocol](https://modelcontextprotocol.io) servers — "Postman for MCP".

Built with Tauri 2, a Rust core, and Angular 22.

> Status: planning. No code yet — see the specs and the issue backlog.

## Planned features

- Register MCP servers (stdio, Streamable HTTP) and import them from Claude Desktop / Claude Code configs
- Explore and call tools, resources, and prompts
- Inspect every JSON-RPC message; record real client sessions through a local proxy
- Measure token usage and context cost per server and per call
- Trace sessions and flows, export via OTLP
- Build prompt flows across servers (visual editor + YAML)
- AI assistance: tool documentation, prompt optimization, flow generation

## Documents

- [Product spec](docs/specs/product-spec.md)
- [Architecture](docs/specs/architecture.md)
- [Data model](docs/specs/data-model.md)
- [Recording, proxy, token metering, and tracing](docs/specs/recording-and-observability.md)
- [Prompt flows](docs/specs/flows.md)
- [ADR 0001 — Stack and architecture](docs/adr/0001-stack-and-architecture.md)

## Issue hierarchy

Work is tracked as Epic → Feature → PBI → Task using GitHub's native sub-issues.
