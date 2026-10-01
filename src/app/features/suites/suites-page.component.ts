import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  effect,
  inject,
  signal,
} from "@angular/core";
import type { TestSuite } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { ExplorerStore } from "../explorer/explorer.store";
import { ServersStore } from "../servers/servers.store";
import {
  CaseDraft,
  ExpectationKind,
  SuiteDraft,
  describeExpectation,
  draftOf,
  newCase,
  newSuite,
  problems,
  toInput,
} from "./suites.model";

/** Test suites of a server: inputs with the tool or answer a model should give. */
@Component({
  selector: "app-suites-page",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./suites-page.component.html",
  styleUrl: "./suites-page.component.scss",
})
export class SuitesPageComponent implements OnInit {
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly dialogs = inject(DialogService);
  private readonly tabs = inject(WorkspaceTabsService);
  private readonly explorer = inject(ExplorerStore);
  protected readonly servers = inject(ServersStore);

  protected readonly serverId = signal<string | null>(null);
  protected readonly suites = signal<TestSuite[]>([]);
  protected readonly draft = signal<SuiteDraft | null>(null);
  protected readonly toolNames = computed(() => {
    const id = this.serverId();
    return id ? this.explorer.snapshot(id).tools.map((t) => t.name) : [];
  });
  protected readonly issues = computed(() => {
    const draft = this.draft();
    return draft ? problems(draft) : [];
  });
  protected readonly kinds: { value: ExpectationKind; label: string }[] = [
    { value: "tool", label: "calls the tool" },
    { value: "noTool", label: "calls no tool" },
    { value: "answer", label: "answers with" },
  ];
  protected describe = describeExpectation;

  constructor() {
    this.tabs.open({ id: "suites", title: "Test suites", route: "/suites" });
    effect(() => {
      const first = this.servers.servers()[0];
      if (this.serverId() === null && first) void this.chooseServer(first.id);
    });
  }

  ngOnInit(): void {
    this.draft.set(null);
  }

  protected async chooseServer(id: string): Promise<void> {
    this.serverId.set(id);
    this.draft.set(null);
    await this.refresh();
  }

  protected async refresh(): Promise<void> {
    const id = this.serverId();
    if (id === null) return;
    try {
      this.suites.set(await this.ipc.testSuiteList(id));
    } catch (error) {
      this.toasts.fail("Could not load the suites", error);
    }
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value;
  }

  protected create(): void {
    this.draft.set(newSuite());
  }

  protected edit(suite: TestSuite): void {
    this.draft.set(draftOf(suite));
  }

  protected patch(patch: Partial<SuiteDraft>): void {
    this.draft.update((d) => (d ? { ...d, ...patch } : d));
  }

  protected patchCase(key: number, patch: Partial<CaseDraft>): void {
    this.draft.update((d) =>
      d ? { ...d, cases: d.cases.map((c) => (c.key === key ? { ...c, ...patch } : c)) } : d,
    );
  }

  protected addCase(): void {
    this.draft.update((d) => (d ? { ...d, cases: [...d.cases, newCase()] } : d));
  }

  protected removeCase(key: number): void {
    this.draft.update((d) => (d ? { ...d, cases: d.cases.filter((c) => c.key !== key) } : d));
  }

  protected async save(): Promise<void> {
    const draft = this.draft();
    const serverId = this.serverId();
    if (!draft || serverId === null || this.issues().length > 0) return;
    try {
      const saved = await this.ipc.testSuiteSave(draft.id, toInput(serverId, draft));
      this.draft.set(draftOf(saved));
      await this.refresh();
      this.toasts.success(`Saved ${saved.name}`);
    } catch (error) {
      this.toasts.fail("Could not save the suite", error);
    }
  }

  protected async remove(suite: { id: string; name: string }): Promise<void> {
    const confirmed = await this.dialogs.confirm(`Delete the suite ${suite.name}?`, {
      confirmLabel: "Delete",
      danger: true,
    });
    if (!confirmed) return;
    try {
      await this.ipc.testSuiteDelete(suite.id);
      if (this.draft()?.id === suite.id) this.draft.set(null);
      await this.refresh();
    } catch (error) {
      this.toasts.fail("Could not delete the suite", error);
    }
  }
}
