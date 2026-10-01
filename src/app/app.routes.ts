import { Routes } from "@angular/router";
import { EnvironmentsPageComponent } from "./features/environments/environments-page.component";
import { ToolPlaygroundComponent } from "./features/playground/tool-playground.component";
import { HistoryPageComponent } from "./features/history/history-page.component";
import { ImportPageComponent } from "./features/import/import-page.component";
import { PricesPageComponent } from "./features/prices/prices-page.component";
import { TracesPageComponent } from "./features/traces/traces-page.component";
import { ServerDetailComponent } from "./features/servers/server-detail.component";
import { ServerFormComponent } from "./features/servers/server-form.component";
import { WelcomeComponent } from "./features/welcome/welcome.component";

export const routes: Routes = [
  { path: "", pathMatch: "full", component: WelcomeComponent },
  { path: "environments", component: EnvironmentsPageComponent },
  { path: "history", component: HistoryPageComponent },
  { path: "prices", component: PricesPageComponent },
  { path: "traces", component: TracesPageComponent },
  { path: "servers/import", component: ImportPageComponent },
  { path: "servers/new", component: ServerFormComponent },
  { path: "servers/:id/edit", component: ServerFormComponent },
  { path: "servers/:id/tools/:name", component: ToolPlaygroundComponent },
  { path: "servers/:id", component: ServerDetailComponent },
  { path: "**", redirectTo: "" },
];
