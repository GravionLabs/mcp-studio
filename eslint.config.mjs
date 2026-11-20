import tseslint from "typescript-eslint";
import angular from "angular-eslint";

/**
 * ESLint lints the Angular templates only: oxlint cannot parse them (its JS plugins take no custom
 * parser). TypeScript, including the Angular class rules, is linted by oxlint (`.oxlintrc.json`).
 * Every component template is a `.html` file (`mcp-studio/no-inline-template`), so no
 * inline-template processor is needed. ADR 0006 of GravionLabs/skills, #321.
 */
export default tseslint.config(
  {
    ignores: [
      "dist/**",
      ".angular/**",
      "node_modules/**",
      ".claude/**",
      ".agents/**",
      "src-tauri/**",
      "crates/**",
      "target/**",
      ".worktrees/**",
      // The lint probe holds deliberate violations; src/testing/lint-probe.spec.ts lints it.
      "src/testing/lint-probe/**",
    ],
  },
  {
    files: ["**/*.html"],
    extends: [...angular.configs.templateRecommended, ...angular.configs.templateAccessibility],
  },
);
