# Clippy lint set

All Rust repositories share one lint set in `[workspace.lints]`. Copy `templates/workspace-lints.toml` into the root `Cargo.toml`; each member crate opts in with `[lints]` and `workspace = true`. Keep the repository's own `[workspace.lints.rust]` entries (`unsafe_code = "forbid"`).

## The set

- `pedantic` as `warn` with `priority = -1`, so single lints below can override it
- `unwrap_used`, `expect_used`, `panic`: no panics on fallible paths; return `Result` instead
- `allow_attributes`, `allow_attributes_without_reason`: no `#[allow]`; every suppression is an `#[expect]` with a reason

The gate turns every warning into an error: `cargo clippy --all-targets --all-features -- -D warnings`.

## Suppressing a lint

Use `#[expect(lint, reason = "…")]`, never `#[allow]`:

```rust
#[expect(clippy::cast_possible_truncation, reason = "length is checked above")]
let small = len as u8;
```

- The `reason` is required
- An `expect` that is no longer met warns as `unfulfilled_lint_expectations`: remove it
- Scope it to the smallest item

## clippy.toml

`templates/clippy.toml` goes next to the `Cargo.toml` of the workspace (or of the crate, as `src-tauri/clippy.toml` in a Tauri app). It allows `unwrap`, `expect`, `panic` and indexing in tests only, and holds a commented `disallowed-methods` / `disallowed-types` example.

## Migrating an existing repository

1. Add the lint set (`templates/workspace-lints.toml`, `[lints]` with `workspace = true` per crate) and `templates/clippy.toml`
2. Preview: `pwsh scripts/Convert-AllowToExpect.ps1 -Path <repo> -WhatIf` lists every change and changes nothing
3. Run it without `-WhatIf`: `#[allow(x)]` and `#![allow(x)]` become `expect` with a reason (an existing `reason` or a `//` comment directly above is kept; else "legacy suppression, migrated from allow"); `target/` and generated files are skipped; `Cargo.toml` is never edited
4. The script then runs `cargo clippy --all-targets --all-features` and lists every `unfulfilled_lint_expectations` (the lint no longer fires, or only under another `cfg`): remove that attribute, or put `allow` back with a comment
5. Review the reasons: replace each "legacy suppression" with the real one
