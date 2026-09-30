import { describe, expect, it } from "vitest";
import { DialogService } from "./dialog.service";

describe("DialogService", () => {
  it("resolves confirm with the user's answer", async () => {
    const service = new DialogService();
    const yes = service.confirm("Delete?", { danger: true });
    expect(service.current()).toMatchObject({ kind: "confirm", message: "Delete?", danger: true });
    service.respond(true);
    await expect(yes).resolves.toBe(true);
    expect(service.current()).toBeNull();

    const no = service.confirm("Again?");
    service.respond(false);
    await expect(no).resolves.toBe(false);
  });

  it("treats a dismissed confirm as no", async () => {
    const service = new DialogService();
    const answer = service.confirm("?");
    service.respond(null);
    await expect(answer).resolves.toBe(false);
  });

  it("returns trimmed prompt text, null when empty or cancelled", async () => {
    const service = new DialogService();
    const named = service.prompt("Name?", "old");
    expect(service.current()?.initial).toBe("old");
    service.respond("  New name ");
    await expect(named).resolves.toBe("New name");

    const empty = service.prompt("Name?");
    service.respond("   ");
    await expect(empty).resolves.toBeNull();

    const cancelled = service.prompt("Name?");
    service.respond(null);
    await expect(cancelled).resolves.toBeNull();
  });

  it("queues dialogs and shows them one after another", async () => {
    const service = new DialogService();
    const first = service.confirm("one");
    const second = service.confirm("two");
    expect(service.current()?.message).toBe("one");
    service.respond(true);
    expect(service.current()?.message).toBe("two");
    service.respond(false);
    await expect(first).resolves.toBe(true);
    await expect(second).resolves.toBe(false);
  });

  it("ignores answers when nothing is open", () => {
    expect(() => new DialogService().respond(true)).not.toThrow();
  });
});
