//! TypeScript bindings for every type that crosses the IPC boundary.
//!
//! Register new IPC types in [`ipc_types`]. The `bindings` integration test fails when the checked-in
//! `src/app/core/bindings.ts` is stale; run `pnpm bindings` to regenerate it.

use specta::Types;
use specta_typescript::Typescript;

use crate::{
    collections::{CollectionNode, CollectionTree, ImportReport, SavedRequest, SavedRequestInput},
    environments::{Environment, EnvironmentInput},
    events::ProgressEvent,
    events::{
        ConnectionState, ListChangedEvent, ListKind, LogEvent, LogSource, MessageRecord,
        StatusEvent,
    },
    explorer::{
        PromptArgumentInfo, PromptInfo, ResourceInfo, ResourceTemplateInfo, ServerDetails, ToolInfo,
    },
    message_store::MessageFilter,
    model::AppInfo,
    registry::{ServerDefinition, ServerInput, TransportKind},
    session::{ToolCallRequest, ToolCallResult},
};

/// All types exposed to the frontend.
pub fn ipc_types() -> Types {
    Types::default()
        .register::<AppInfo>()
        .register::<TransportKind>()
        .register::<ServerInput>()
        .register::<ServerDefinition>()
        .register::<EnvironmentInput>()
        .register::<Environment>()
        .register::<ConnectionState>()
        .register::<StatusEvent>()
        .register::<MessageRecord>()
        .register::<LogSource>()
        .register::<LogEvent>()
        .register::<MessageFilter>()
        .register::<ListKind>()
        .register::<ListChangedEvent>()
        .register::<ToolInfo>()
        .register::<ResourceInfo>()
        .register::<ResourceTemplateInfo>()
        .register::<PromptArgumentInfo>()
        .register::<PromptInfo>()
        .register::<ServerDetails>()
        .register::<ProgressEvent>()
        .register::<ToolCallRequest>()
        .register::<ToolCallResult>()
        .register::<CollectionNode>()
        .register::<SavedRequestInput>()
        .register::<SavedRequest>()
        .register::<CollectionTree>()
        .register::<ImportReport>()
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
