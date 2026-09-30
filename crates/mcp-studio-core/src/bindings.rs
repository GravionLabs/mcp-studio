//! TypeScript bindings for every type that crosses the IPC boundary.
//!
//! Register new IPC types in [`ipc_types`]. The `bindings` integration test fails when the checked-in
//! `src/app/core/bindings.ts` is stale; run `pnpm bindings` to regenerate it.

use specta::Types;
use specta_typescript::Typescript;

use crate::{
    model::AppInfo,
    registry::{ServerDefinition, ServerInput, TransportKind},
};

/// All types exposed to the frontend.
pub fn ipc_types() -> Types {
    Types::default()
        .register::<AppInfo>()
        .register::<TransportKind>()
        .register::<ServerInput>()
        .register::<ServerDefinition>()
}

/// Renders the TypeScript bindings file.
pub fn typescript_bindings() -> String {
    Typescript::default()
        .header("// Generated from Rust by `pnpm bindings`. DO NOT EDIT.")
        .export(&ipc_types(), specta_serde::Format)
        .expect("IPC types must be exportable to TypeScript")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_contain_app_info() {
        let ts = typescript_bindings();
        assert!(ts.contains("AppInfo"), "{ts}");
        assert!(ts.contains("version: string"), "{ts}");
    }
}
