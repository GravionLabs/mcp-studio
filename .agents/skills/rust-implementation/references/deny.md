# cargo-deny

`templates/deny.toml` goes next to the workspace `Cargo.toml` (in a Tauri app: `src-tauri/deny.toml`). It checks `advisories`, `licenses`, `bans` and `sources` with all features. Run `cargo deny check`.

## Network

`cargo deny check` fetches the RustSec advisory database: it needs the network. Offline runs use `--disable-fetch` and fail when the cached database is older than `maximum-db-staleness` (90 days).

## Adding an ignore

Ignore an advisory only when it is not reachable or no fix exists. Every entry in `[advisories] ignore` is a table with all three keys:

```toml
ignore = [
  { id = "RUSTSEC-0000-0000", reason = "we do not call the affected API; upstream fix pending", expiry = "2027-06-30" },
]
```

- `reason` is mandatory: why it is safe and what ends the ignore
- `expiry` is mandatory: an absolute date `YYYY-MM-DD`, or a duration such as `"P90D"` counted from the publish date of the advisory. After it the advisory is reported again with an `advisory-ignore-expired` diagnostic
- A bare `"RUSTSEC-…"` string is not allowed
- `ignore-expiry` under `[advisories]` is the default for entries without their own `expiry`; do not rely on it

## Stale ignores

An ignore that no crate in the graph matches is reported (`unused-ignored-advisory`, default `warn`). Remove it when the dependency is gone or fixed.

## Licenses

Only the permissive allow list of the template. A new license is added to `allow` after a review, never by an exception per crate without a comment.
