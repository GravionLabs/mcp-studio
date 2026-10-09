/** The sections of a server's page, shown as tabs. */
export type ServerTab =
  "explorer" | "context" | "quality" | "docs" | "settings" | "client" | "logs";

export interface ServerTabInfo {
  id: ServerTab;
  label: string;
  /** Shown only while the server is connected, because it reads the server's tools. */
  needsConnection: boolean;
}

export const SERVER_TABS: readonly ServerTabInfo[] = [
  { id: "explorer", label: "Explorer", needsConnection: true },
  { id: "context", label: "Context cost", needsConnection: true },
  { id: "quality", label: "Tool quality", needsConnection: true },
  { id: "docs", label: "Documentation", needsConnection: true },
  { id: "settings", label: "Settings", needsConnection: false },
  { id: "client", label: "Record a client", needsConnection: false },
  { id: "logs", label: "Logs", needsConnection: false },
];

export function visibleTabs(connected: boolean): readonly ServerTabInfo[] {
  return SERVER_TABS.filter((tab) => connected || !tab.needsConnection);
}

/** The picked tab while it is shown; otherwise the explorer when connected, else the settings. */
export function activeTab(selected: ServerTab | null, connected: boolean): ServerTab {
  if (selected && visibleTabs(connected).some((tab) => tab.id === selected)) return selected;
  return connected ? "explorer" : "settings";
}

/** Whether a connection error means that the Azure login is missing or has to be redone. */
export function needsAzureLogin(message: string | undefined): boolean {
  return (
    !!message && /az login|azd auth login|Sign in with Azure|not signed in to Azure/i.test(message)
  );
}

/** Whether the Azure CLI has to sign in to a specific tenant (multi-factor authentication). */
export function needsTenant(message: string | undefined): boolean {
  return !!message && /multi-factor|AADSTS50076/i.test(message);
}
