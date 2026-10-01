import type { ExportStatus } from "../../core/bindings";
import { formatClock } from "../inspector/inspector.model";

/** One line that tells whether spans are reaching the collector. */
export function describeExport(enabled: boolean, status: ExportStatus): string {
  if (!enabled) return "Export is off. Nothing leaves this computer.";
  if (status.lastError) return `The last export failed: ${status.lastError}`;
  if (status.lastSuccessAt !== null) {
    const spans = status.exported === 1 ? "1 span" : `${status.exported} spans`;
    return `Sent ${spans}; the last export was at ${formatClock(status.lastSuccessAt)}.`;
  }
  return "Waiting for spans to export. Only spans recorded after turning this on are sent.";
}
