# ADR 0002 — Record at the rmcp transport layer

## Status

Accepted (result of the M0 spike, #32)

## Context

Inspector, tracing, and token metering all need every JSON-RPC message that crosses a connection.
The open question from the product spec was whether `rmcp` allows a tap "underneath" its parser, or
whether the byte stream (child stdin/stdout, HTTP bodies) has to be wrapped instead.

## Options

1. **Wrap the raw byte stream.** Exact bytes, but needs separate code per transport (line framing for
   stdio, SSE and POST bodies for Streamable HTTP) and re-implements what `rmcp` already does.
2. **Wrap `rmcp::transport::Transport<R>`.** One generic wrapper for every transport and both roles.
   Messages are seen as parsed JSON-RPC values and serialized back to `serde_json::Value`.

## Decision

Use option 2: `RecordingTransport<T, R>` in `mcp-studio-core::recording`.

Verified in the spike (`crates/mcp-studio-core/tests/rmcp_spike.rs`) against the reference server
`mcp-studio-testserver`:

- stdio through `TokioChildProcess` and Streamable HTTP through `StreamableHttpClientTransport` both
  work unchanged when wrapped; `initialize`, `tools/list`, `tools/call`, and their responses are all
  recorded with direction, timestamp, and size.
- The wrapper does not alter, reorder, or delay messages. A failing recorder cannot fail the
  connection because recording is fire-and-forget.
- `Transport::send` returns a `'static` future, so recording happens when `send` is called, before the
  message is actually written.

## Consequences

- Payloads are the re-serialized JSON, not the exact wire bytes: whitespace and key order can differ.
  For debugging MCP this is acceptable; the inspector shows the JSON structure.
- The same wrapper works on the server side (`RoleServer`), which the proxy mode will use.
- The proxy binary for stdio (#88) forwards lines between the real client and the app and needs no
  byte-exact tap either; if byte-exactness is ever required, a raw tap can be added there without
  changing this design.
- Reference server: `mcp-studio-testserver` (stdio, or `--http <port>` for Streamable HTTP) is the
  fixture for all integration tests.
