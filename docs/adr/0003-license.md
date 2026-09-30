# ADR 0003 — License

## Status

Accepted (can be revisited before the repository is made public)

## Context

The product spec decided: open source to start, with a possible Pro offering (open core) later. The
concrete license was left open. The Rust ecosystem (including `rmcp`, `tauri`, and most crates we depend
on) is overwhelmingly licensed `MIT OR Apache-2.0`, and Cargo metadata already said so.

## Decision

License the whole repository under **`MIT OR Apache-2.0`**, at the user's option:

- Maximum adoption and compatibility: users and downstream projects can pick the license that fits.
- Apache-2.0 contributes an explicit patent grant; MIT keeps it simple for small integrations.
- Matches the dependency ecosystem, so no license friction.

Contributions are accepted under the same terms (stated in `CONTRIBUTING.md`).

## Consequences

- `LICENSE-MIT` and `LICENSE-APACHE` live in the repository root; `Cargo.toml` uses
  `license = "MIT OR Apache-2.0"`.
- A future Pro offering can live in separate, differently licensed code (open core). Relicensing
  existing contributions would require contributor consent, so proprietary features must be added as
  new modules rather than by changing the license of this code.
- The repository is still private. Making it public is a separate, deliberate step.
