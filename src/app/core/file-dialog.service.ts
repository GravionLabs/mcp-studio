import { Injectable } from "@angular/core";

/** Native file pickers (via the Tauri dialog plugin). */
@Injectable({ providedIn: "root" })
export class FileDialogService {
  /** Asks where to save a JSON file; null when cancelled. */
  async pickSavePath(defaultName: string): Promise<string | null> {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({ defaultPath: defaultName, filters: [{ name: "JSON", extensions: ["json"] }] });
  }

  /** Asks for a JSON file to open; null when cancelled. */
  async pickOpenPath(): Promise<string | null> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    return typeof selected === "string" ? selected : null;
  }
}

/** A safe file name for a folder title. */
export function fileNameFor(title: string, extension = "json"): string {
  const base = title
    .trim()
    .replace(/[^\w.-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .toLowerCase();
  return `${base || "collection"}.${extension}`;
}
