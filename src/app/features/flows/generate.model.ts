import type { FlowIssue } from "../../core/bindings";

export const MAX_GOAL_CHARS = 4000;

/** What stops a generation from starting, or `null` when it may start. */
export function blocker(goal: string, serverIds: readonly string[]): string | null {
  const text = goal.trim();
  if (text === "") return "Describe what the flow should do.";
  if (text.length > MAX_GOAL_CHARS) return `The goal is longer than ${MAX_GOAL_CHARS} characters.`;
  if (serverIds.length === 0) return "Choose at least one server whose tools the flow may use.";
  return null;
}

/** Adds or removes `id`, keeping the order of the others. */
export function toggled(ids: readonly string[], id: string): string[] {
  return ids.includes(id) ? ids.filter((i) => i !== id) : [...ids, id];
}

/** One line per issue, with the step it belongs to. */
export function issueLines(issues: readonly Pick<FlowIssue, "stepId" | "message">[]): string[] {
  return issues.map((i) => (i.stepId ? `${i.stepId}: ${i.message}` : i.message));
}

/** What to tell the user after a generation. */
export function outcome(generated: { issues: readonly unknown[]; attempts: number }): string {
  const repaired = generated.attempts > 1 ? " after one repair" : "";
  return generated.issues.length === 0
    ? `Generated a valid flow${repaired}.`
    : `The model's flow has ${generated.issues.length} problem${generated.issues.length === 1 ? "" : "s"}${repaired}, so it was not opened.`;
}
