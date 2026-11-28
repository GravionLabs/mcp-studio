import type { OnDestroy } from "@angular/core";
import { Component, EventEmitter, Output } from "@angular/core";

class ProbeService {
  async load(): Promise<string> {
    return "probe";
  }
}

/**
 * Lint probe (#321): every violation here is deliberate. `src/testing/lint-probe.spec.ts` runs
 * oxlint and ESLint on this folder and expects each rule id, so a broken oxlint JS plugin, a lost
 * type-aware rule or a lost template rule fails a test instead of passing silently. Excluded from
 * the build and from `pnpm lint`.
 */
@Component({
  // @angular-eslint/component-selector: not the app prefix.
  selector: "probe-component",
  templateUrl: "./probe.component.html",
})
export class ProbeComponent implements OnDestroy {
  // @angular-eslint/no-output-on-prefix
  @Output() readonly onPick = new EventEmitter<string>();

  // @angular-eslint/prefer-inject
  constructor(readonly service: ProbeService) {}

  // @angular-eslint/use-lifecycle-interface: no `implements OnInit`.
  ngOnInit(): void {
    // typescript/no-floating-promises (type-aware, oxlint-tsgolint)
    this.service.load();
  }

  // @angular-eslint/no-empty-lifecycle-method
  ngOnDestroy(): void {}

  pick(): void {
    this.onPick.emit("pick");
  }
}
