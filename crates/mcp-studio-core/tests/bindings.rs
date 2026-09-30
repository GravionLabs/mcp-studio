use std::{env, fs, path::PathBuf};

use mcp_studio_core::bindings::typescript_bindings;

fn bindings_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/app/core/bindings.ts")
}

#[test]
fn checked_in_bindings_are_up_to_date() {
    let expected = typescript_bindings();
    let path = bindings_path();
    if env::var_os("UPDATE_BINDINGS").is_some() {
        fs::write(&path, &expected).unwrap();
        return;
    }
    let actual = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        actual.replace("\r\n", "\n"),
        expected,
        "bindings.ts is stale, run `pnpm bindings`"
    );
}
