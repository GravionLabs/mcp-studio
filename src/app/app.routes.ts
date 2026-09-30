import { Routes } from "@angular/router";
import { WelcomeComponent } from "./features/welcome/welcome.component";

export const routes: Routes = [
  { path: "", pathMatch: "full", component: WelcomeComponent },
  { path: "**", redirectTo: "" },
];
