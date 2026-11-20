/**
 * MCP Studio's own oxlint rules (an oxlint JS plugin, `jsPlugins` in `.oxlintrc.json`).
 *
 * `mcp-studio/no-inline-template` (#321): a `@Component` keeps its template in a sibling `.html`
 * file (`templateUrl`), so ESLint, which lints `**\/*.html` only, sees every template.
 * `.oxlintrc.json` turns it off for spec files, whose test hosts may keep inline templates.
 */
const noInlineTemplate = {
  meta: {
    type: "problem",
    docs: { description: "Disallow an inline template in a @Component decorator." },
    messages: {
      inline: "Put the component template in a sibling .html file and use templateUrl (#321).",
    },
    schema: [],
  },
  create(context) {
    return {
      Decorator(node) {
        const call = node.expression;
        if (call?.type !== "CallExpression" || call.callee?.name !== "Component") return;
        const meta = call.arguments[0];
        if (meta?.type !== "ObjectExpression") return;
        for (const prop of meta.properties) {
          const key = prop.type === "Property" ? prop.key : null;
          const name = key?.type === "Identifier" ? key.name : key?.value;
          if (name === "template") context.report({ node: prop, messageId: "inline" });
        }
      },
    };
  },
};

export default {
  meta: { name: "mcp-studio" },
  rules: { "no-inline-template": noInlineTemplate },
};
