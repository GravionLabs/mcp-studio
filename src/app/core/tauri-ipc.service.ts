import { Injectable, Signal, inject, signal } from "@angular/core";
import type {
  AppInfo,
  CollectionNode,
  CollectionTree,
  ConfigSource,
  ContextCost,
  ExportConfig,
  ExportStatus,
  HistoryEntry,
  HistoryFilter,
  ImportCandidate,
  ImportReport,
  ImportSummary,
  SavedRequest,
  SavedRequestInput,
  Environment,
  EnvironmentInput,
  LogEvent,
  MessageFilter,
  MessageRecord,
  ProxyInfo,
  PromptInfo,
  Price,
  ResourceInfo,
  ResourceTemplateInfo,
  ServerDefinition,
  ServerDetails,
  ServerInput,
  SessionUsage,
  Span,
  SpanFilter,
  ToolCallRequest,
  ToolCallResult,
  ToolInfo,
  UpdateInfo,
} from "./bindings";
import { IpcError, describeError } from "./ipc-error";
import { IPC_INVOKE, IPC_LISTEN } from "./ipc.tokens";

/** Typed wrapper around Tauri commands and events. Every command has one method here. */
@Injectable({ providedIn: "root" })
export class TauriIpcService {
  private readonly invokeFn = inject(IPC_INVOKE);
  private readonly listenFn = inject(IPC_LISTEN);

  /** Calls a command and converts rejections into {@link IpcError}. */
  async call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return await this.invokeFn<T>(command, args);
    } catch (error) {
      throw new IpcError(command, describeError(error));
    }
  }

  /**
   * Exposes the latest payload of an event as a signal. The subscription lives as long as the
   * app; call the returned `stop` to unsubscribe earlier.
   */
  eventSignal<T>(event: string): { value: Signal<T | undefined>; stop: () => void } {
    const value = signal<T | undefined>(undefined);
    let unlisten: (() => void) | undefined;
    let stopped = false;
    void this.listenFn<T>(event, (payload) => value.set(payload)).then((fn) => {
      if (stopped) fn();
      else unlisten = fn;
    });
    return {
      value: value.asReadonly(),
      stop: () => {
        stopped = true;
        unlisten?.();
      },
    };
  }

  /** Subscribes to a Tauri event for the lifetime of the app (or until the returned function is called). */
  listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
    return this.listenFn<T>(event, handler);
  }

  appInfo(): Promise<AppInfo> {
    return this.call<AppInfo>("app_info");
  }

  /** The newest available version, or `null` when this one is current. */
  updateCheck(): Promise<UpdateInfo | null> {
    return this.call<UpdateInfo | null>("update_check");
  }

  /** Downloads and installs the update found by the last check; the app restarts afterwards. */
  updateInstall(): Promise<void> {
    return this.call("update_install");
  }

  spansQuery(filter: SpanFilter): Promise<Span[]> {
    return this.call("spans_query", { filter });
  }

  traceExportConfig(): Promise<ExportConfig> {
    return this.call("trace_export_config");
  }

  traceExportSetConfig(config: ExportConfig): Promise<ExportConfig> {
    return this.call("trace_export_set_config", { config });
  }

  traceExportStatus(): Promise<ExportStatus> {
    return this.call("trace_export_status");
  }

  /** Sends the waiting spans now. */
  traceExportNow(): Promise<ExportStatus> {
    return this.call("trace_export_now");
  }

  priceList(): Promise<Price[]> {
    return this.call("price_list");
  }

  priceSet(price: Price): Promise<Price> {
    return this.call("price_set", { price });
  }

  priceRemove(model: string): Promise<void> {
    return this.call("price_remove", { model });
  }

  /** Tokens of the given tool definitions; with `model`, also the cost of sending them once. */
  toolsContextCost(tools: ToolInfo[], model: string | null): Promise<ContextCost> {
    return this.call("tools_context_cost", { tools, model });
  }

  sessionUsage(sessionId: string, model: string | null): Promise<SessionUsage> {
    return this.call("session_usage", { sessionId, model });
  }

  serverList(): Promise<ServerDefinition[]> {
    return this.call("server_list");
  }

  serverGet(id: string): Promise<ServerDefinition> {
    return this.call("server_get", { id });
  }

  serverAdd(input: ServerInput): Promise<ServerDefinition> {
    return this.call("server_add", { input });
  }

  serverUpdate(id: string, input: ServerInput): Promise<ServerDefinition> {
    return this.call("server_update", { id, input });
  }

  serverRemove(id: string): Promise<void> {
    return this.call("server_remove", { id });
  }

  secretSet(name: string, value: string): Promise<void> {
    return this.call("secret_set", { name, value });
  }

  secretDelete(name: string): Promise<void> {
    return this.call("secret_delete", { name });
  }

  environmentList(): Promise<Environment[]> {
    return this.call("environment_list");
  }

  environmentAdd(input: EnvironmentInput): Promise<Environment> {
    return this.call("environment_add", { input });
  }

  environmentUpdate(id: string, input: EnvironmentInput): Promise<Environment> {
    return this.call("environment_update", { id, input });
  }

  environmentRemove(id: string): Promise<void> {
    return this.call("environment_remove", { id });
  }

  messagesQuery(filter: Partial<MessageFilter> = {}): Promise<MessageRecord[]> {
    return this.call("messages_query", { filter });
  }

  serverConnect(id: string, environmentId: string | null): Promise<void> {
    return this.call("server_connect", { id, environmentId });
  }

  serverDisconnect(id: string): Promise<void> {
    return this.call("server_disconnect", { id });
  }

  serverLogs(id: string): Promise<LogEvent[]> {
    return this.call("server_logs", { id });
  }

  serverDetails(id: string): Promise<ServerDetails> {
    return this.call("server_details", { id });
  }

  toolsList(id: string): Promise<ToolInfo[]> {
    return this.call("tools_list", { id });
  }

  resourcesList(id: string): Promise<ResourceInfo[]> {
    return this.call("resources_list", { id });
  }

  resourceTemplatesList(id: string): Promise<ResourceTemplateInfo[]> {
    return this.call("resource_templates_list", { id });
  }

  promptsList(id: string): Promise<PromptInfo[]> {
    return this.call("prompts_list", { id });
  }

  toolCall(request: ToolCallRequest): Promise<ToolCallResult> {
    return this.call("tool_call", { request });
  }

  /** Cancels a running call; resolves to whether it was still running. */
  requestCancel(callId: string): Promise<boolean> {
    return this.call("request_cancel", { callId });
  }

  resourceRead(id: string, uri: string): Promise<unknown> {
    return this.call("resource_read", { id, uri });
  }

  promptGet(id: string, name: string, args: Record<string, string>): Promise<unknown> {
    return this.call("prompt_get", { id, name, arguments: args });
  }

  collectionsTree(): Promise<CollectionTree> {
    return this.call("collections_tree");
  }

  collectionCreate(parentId: string | null, name: string): Promise<CollectionNode> {
    return this.call("collection_create", { parentId, name });
  }

  collectionRename(id: string, name: string): Promise<void> {
    return this.call("collection_rename", { id, name });
  }

  collectionMove(id: string, parentId: string | null): Promise<void> {
    return this.call("collection_move", { id, parentId });
  }

  collectionDelete(id: string): Promise<void> {
    return this.call("collection_delete", { id });
  }

  requestSave(input: SavedRequestInput): Promise<SavedRequest> {
    return this.call("request_save", { input });
  }

  requestUpdate(id: string, input: SavedRequestInput): Promise<void> {
    return this.call("request_update", { id, input });
  }

  requestDelete(id: string): Promise<void> {
    return this.call("request_delete", { id });
  }

  collectionExport(id: string, path: string): Promise<void> {
    return this.call("collection_export", { id, path });
  }

  collectionImport(path: string, parentId: string | null): Promise<ImportReport> {
    return this.call("collection_import", { path, parentId });
  }

  historyList(filter: HistoryFilter = {}): Promise<HistoryEntry[]> {
    return this.call("history_list", { filter });
  }

  historyClear(serverId: string | null): Promise<number> {
    return this.call("history_clear", { serverId });
  }

  proxyInfo(): Promise<ProxyInfo> {
    return this.call("proxy_info");
  }

  proxySetEnvironment(id: string | null): Promise<void> {
    return this.call("proxy_set_environment", { id });
  }

  /** Opens the browser for OAuth sign-in; resolves when the tokens are stored. */
  oauthSignIn(id: string): Promise<void> {
    return this.call("oauth_sign_in", { id });
  }

  oauthSignOut(id: string): Promise<void> {
    return this.call("oauth_sign_out", { id });
  }

  oauthStatus(id: string): Promise<boolean> {
    return this.call("oauth_status", { id });
  }

  importSources(): Promise<ConfigSource[]> {
    return this.call("import_sources");
  }

  importPreview(path: string): Promise<ImportCandidate[]> {
    return this.call("import_preview", { path });
  }

  importApply(servers: ServerInput[]): Promise<ImportSummary> {
    return this.call("import_apply", { servers });
  }
}
