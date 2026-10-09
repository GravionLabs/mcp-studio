import { Routes } from "@angular/router";
import { EnvironmentsPage } from "./features/environments/environments-page";
import { ToolPlayground } from "./features/playground/tool-playground";
import { HistoryPage } from "./features/history/history-page";
import { ImportPage } from "./features/import/import-page";
import { ClientsPage } from "./features/clients/clients-page";
import { PricesPage } from "./features/prices/prices-page";
import { SettingsPage } from "./features/settings/settings-page";
import { TracesPage } from "./features/traces/traces-page";
import { FlowsPage } from "./features/flows/flows-page";
import { ProvidersPage } from "./features/providers/providers-page";
import { ComparePage } from "./features/compare/compare-page";
import { SuitesPage } from "./features/suites/suites-page";
import { ServerDetail } from "./features/servers/server-detail";
import { ServerForm } from "./features/servers/server-form";
import { Welcome } from "./features/welcome/welcome";

export const routes: Routes = [
  { path: "", pathMatch: "full", component: Welcome },
  { path: "environments", component: EnvironmentsPage },
  { path: "history", component: HistoryPage },
  { path: "flows", component: FlowsPage },
  { path: "suites", component: SuitesPage },
  { path: "compare", component: ComparePage },
  {
    // The graph library is large, so the editor is loaded when it is opened.
    path: "flows/:id/edit",
    loadComponent: () => import("./features/flows/flow-editor").then((m) => m.FlowEditor),
  },
  { path: "providers", component: ProvidersPage },
  { path: "prices", component: PricesPage },
  { path: "settings", component: SettingsPage },
  { path: "clients", component: ClientsPage },
  { path: "traces", component: TracesPage },
  { path: "servers/import", component: ImportPage },
  { path: "servers/new", component: ServerForm },
  { path: "servers/:id/edit", component: ServerForm },
  { path: "servers/:id/tools/:name", component: ToolPlayground },
  { path: "servers/:id", component: ServerDetail },
  { path: "**", redirectTo: "" },
];
