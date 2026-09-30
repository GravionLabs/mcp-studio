import type { ImportCandidate } from "../../core/bindings";

/** Indexes that start checked: everything usable and new. */
export function defaultSelection(candidates: ImportCandidate[]): Set<number> {
  return new Set(
    candidates.flatMap((candidate, index) =>
      !candidate.duplicate && !candidate.unsupported ? [index] : [],
    ),
  );
}

/** Indexes that can be selected at all. */
export function selectable(candidates: ImportCandidate[]): number[] {
  return candidates.flatMap((candidate, index) => (candidate.unsupported ? [] : [index]));
}

/** One line describing what the server runs or where it lives. */
export function describeCandidate(candidate: ImportCandidate): string {
  const { input } = candidate;
  if (input.transport === "http") return input.url ?? "";
  return [input.command, ...input.args].filter(Boolean).join(" ");
}

/** How many environment variables or headers look like secrets and will move to the keyring. */
export function secretCount(candidate: ImportCandidate): number {
  const looksSecret = (name: string) =>
    /key|token|secret|password|passwd|auth|credential|cookie/i.test(name);
  return [...Object.keys(candidate.input.env), ...Object.keys(candidate.input.headers)].filter(
    looksSecret,
  ).length;
}

/** The inputs to send for the selected indexes, in file order. */
export function selectedInputs(candidates: ImportCandidate[], selected: ReadonlySet<number>) {
  return candidates.filter((_, index) => selected.has(index)).map((c) => c.input);
}
