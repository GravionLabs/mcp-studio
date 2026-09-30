import { Routes } from "@angular/router";
import { EnvironmentsPageComponent } from "./features/environments/environments-page.component";
import { ServerDetailComponent } from "./features/servers/server-detail.component";
import { ServerFormComponent } from "./features/servers/server-form.component";
import { WelcomeComponent } from "./features/welcome/welcome.component";

export const routes: Routes = [
  { path: "", pathMatch: "full", component: WelcomeComponent },
  { path: "environments", component: EnvironmentsPageComponent },
  { path: "servers/new", component: ServerFormComponent },
  { path: "servers/:id/edit", component: ServerFormComponent },
  { path: "servers/:id", component: ServerDetailComponent },
  { path: "**", redirectTo: "" },
];
