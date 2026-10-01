import type { CaseResult, TestSuite, Variant, VariantResult } from "../../core/bindings";

export const BASELINE: Variant = {
  id: "baseline",
  label: "Current",
  systemPrompt: null,
  toolDescriptions: {},
};

/** How many model calls running `variants` over the cases of `suite` makes. */
export function callCount(suite: Pick<TestSuite, "cases">, variants: readonly unknown[]): number {
  return suite.cases.length * variants.length;
}

export const accuracyOf = (r: Pick<VariantResult, "passed" | "total">): number =>
  r.total === 0 ? 0 : r.passed / r.total;

export const formatPercent = (share: number): string => `${Math.round(share * 100)} %`;

export const tokensOf = (r: Pick<VariantResult, "inputTokens" | "outputTokens">): number =>
  r.inputTokens + r.outputTokens;

export interface SummaryRow {
  id: string;
  label: string;
  passed: number;
  total: number;
  accuracy: number;
  inputTokens: number;
  outputTokens: number;
  /** Estimated tokens of the tool definitions in every request. */
  definitionTokens: number;
  /** The best variant: most accurate, and among those the cheapest. */
  best: boolean;
}

/** One row per variant for the side-by-side table; the best one is marked. */
export function summarize(results: readonly VariantResult[]): SummaryRow[] {
  const rows = results.map((r) => ({
    id: r.variant.id,
    label: r.variant.label,
    passed: r.passed,
    total: r.total,
    accuracy: accuracyOf(r),
    inputTokens: r.inputTokens,
    outputTokens: r.outputTokens,
    definitionTokens: r.definitionTokens,
    best: false,
  }));
  if (rows.length < 2) return rows;
  let best = rows[0];
  for (const row of rows) {
    if (!best) break;
    if (
      row.accuracy > best.accuracy ||
      (row.accuracy === best.accuracy &&
        row.inputTokens + row.outputTokens < best.inputTokens + best.outputTokens)
    ) {
      best = row;
    }
  }
  return rows.map((r) => ({ ...r, best: r === best && r.accuracy > 0 }));
}

export interface MatrixCell {
  passed: boolean;
  /** What the model did: the tool it called, or a note. */
  note: string;
}

export interface MatrixRow {
  caseId: string;
  input: string;
  /** One cell per variant, in the order of `results`; `null` when the case is not in that result. */
  cells: (MatrixCell | null)[];
}

function noteOf(result: CaseResult): string {
  if (result.error) return `error: ${result.error}`;
  if (result.calledTool) return `called ${result.calledTool}`;
  return result.answer === "" ? "no tool, no text" : "answered";
}

/** Cases as rows and variants as columns. */
export function matrix(results: readonly VariantResult[]): MatrixRow[] {
  const rows = new Map<string, MatrixRow>();
  results.forEach((result, column) => {
    for (const c of result.results) {
      const row = rows.get(c.caseId) ?? {
        caseId: c.caseId,
        input: c.input,
        cells: results.map(() => null),
      };
      row.cells[column] = { passed: c.passed, note: noteOf(c) };
      rows.set(c.caseId, row);
    }
  });
  return [...rows.values()];
}

/** The cases a result got wrong, to aim proposals at. Cancelled cases are left out. */
export function failingOf(result: VariantResult): CaseResult[] {
  return result.results.filter((r) => !r.passed && r.error !== "cancelled");
}

/** What a variant changes, as Markdown to copy. */
export function describeVariant(variant: Variant): string {
  const parts: string[] = [`## ${variant.label}`];
  if (variant.systemPrompt) parts.push("### System prompt", variant.systemPrompt);
  const entries = Object.entries(variant.toolDescriptions);
  if (entries.length > 0) {
    parts.push(
      "### Tool descriptions",
      entries.map(([tool, description]) => `- \`${tool}\`: ${description}`).join("\n"),
    );
  }
  return parts.join("\n\n");
}

/** Messages for the progress line: `3 of 12 cases done`. */
export function progressText(done: number, total: number): string {
  return `${done} of ${total} cases done`;
}
