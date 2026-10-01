import type { LintFinding, LintReport, LintRule, Severity } from "../../core/bindings";

export interface ToolFindings {
  /** The tool, or `null` for findings about the whole server. */
  tool: string | null;
  findings: LintFinding[];
  /** The most severe finding of the tool. */
  severity: Severity;
}

const ORDER: Record<Severity, number> = { error: 2, warning: 1, info: 0 };

export function worst(findings: readonly LintFinding[]): Severity {
  return findings.reduce<Severity>(
    (worstSoFar, f) => (ORDER[f.severity] > ORDER[worstSoFar] ? f.severity : worstSoFar),
    "info",
  );
}

/** Findings grouped by tool in the order they came in (the server first, then tool by tool). */
export function groupByTool(report: LintReport): ToolFindings[] {
  const groups = new Map<string | null, LintFinding[]>();
  for (const finding of report.findings) {
    groups.set(finding.tool, [...(groups.get(finding.tool) ?? []), finding]);
  }
  return [...groups].map(([tool, findings]) => ({ tool, findings, severity: worst(findings) }));
}

export function countBySeverity(report: LintReport): Record<Severity, number> {
  const counts: Record<Severity, number> = { error: 0, warning: 0, info: 0 };
  for (const finding of report.findings) counts[finding.severity] += 1;
  return counts;
}

const RULE_TITLES: Record<LintRule, string> = {
  missingDescription: "No description",
  vagueDescription: "Vague description",
  undescribedParameter: "Undescribed parameters",
  missingRequired: "No required parameters",
  unknownRequired: "Unknown required parameter",
  overlappingTools: "Overlapping tools",
  oversizedDefinition: "Oversized definition",
  oversizedServer: "Oversized server",
};

export const ruleTitle = (rule: LintRule): string => RULE_TITLES[rule];

/** One line for the header of the panel. */
export function summarize(report: LintReport): string {
  const { error, warning, info } = countBySeverity(report);
  const tools = `${report.toolsChecked} ${report.toolsChecked === 1 ? "tool" : "tools"}`;
  if (error + warning + info === 0) return `${tools} checked: no problems found`;
  const parts = [
    error > 0 ? `${error} ${error === 1 ? "error" : "errors"}` : null,
    warning > 0 ? `${warning} ${warning === 1 ? "warning" : "warnings"}` : null,
    info > 0 ? `${info} ${info === 1 ? "hint" : "hints"}` : null,
  ].filter((p): p is string => p !== null);
  return `${tools} checked: ${parts.join(", ")}`;
}
