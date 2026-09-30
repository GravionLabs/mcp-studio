/** Error thrown when a Tauri command fails. */
export class IpcError extends Error {
  constructor(
    readonly command: string,
    readonly detail: string,
  ) {
    super(`${command} failed: ${detail}`);
    this.name = "IpcError";
  }
}

/** Extracts a readable message from whatever a command rejected with. */
export function describeError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return JSON.stringify(error);
}
