import { Routes } from "@angular/router";
import { EnvironmentsPageComponent } from "./features/environments/environments-page.component";
import { ToolPlaygroundComponent } from "./features/playground/tool-playground.component";
import { HistoryPageComponent } from "./features/history/history-page.component";
import { ImportPageComponent } from "./features/import/import-page.component";
import { PricesPageComponent } from "./features/prices/prices-page.component";
import { TracesPageComponent } from "./features/traces/traces-page.component";
import { FlowsPageComponent } from "./features/flows/flows-page.component";
import { ProvidersPageComponent } from "./features/providers/providers-page.component";
import { ComparePageComponent } from "./features/compare/compare-page.component";
import { SuitesPageComponent } from "./features/suites/suites-page.component";
import { ServerDetailComponent } from "./features/servers/server-detail.component";
import { ServerFormComponent } from "./features/servers/server-form.component";
import { WelcomeComponent } from "./features/welcome/welcome.component";

export const routes: Routes = [
  { path: "", pathMatch: "full", component: WelcomeComponent },
  { path: "environments", component: EnvironmentsPageComponent },
  { path: "history", component: HistoryPageComponent },
  { path: "flows", component: FlowsPageComponent },
  { path: "suites", component: SuitesPageComponent },
  { path: "compare", component: ComparePageComponent },
  {
    // The graph library is large, so the editor is loaded when it is opened.
    path: "flows/:id/edit",
    loadComponent: () =>
      import("./features/flows/flow-editor.component").then((m) => m.FlowEditorComponent),
  },
  { path: "providers", component: ProvidersPageComponent },
  { path: "prices", component: PricesPageComponent },
  { path: "traces", component: TracesPageComponent },
  { path: "servers/import", component: ImportPageComponent },
  { path: "servers/new", component: ServerFormComponent },
  { path: "servers/:id/edit", component: ServerFormComponent },
  { path: "servers/:id/tools/:name", component: ToolPlaygroundComponent },
  { path: "servers/:id", component: ServerDetailComponent },
  { path: "**", redirectTo: "" },
];
