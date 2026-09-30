/** The JSON Schema subset MCP tools use for `inputSchema`. */
export interface JsonSchema {
  type?: string | string[];
  title?: string;
  description?: string;
  properties?: Record<string, JsonSchema>;
  required?: string[];
  items?: JsonSchema;
  enum?: unknown[];
  const?: unknown;
  default?: unknown;
  format?: string;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  pattern?: string;
  minItems?: number;
  maxItems?: number;
  anyOf?: JsonSchema[];
  oneOf?: JsonSchema[];
  allOf?: JsonSchema[];
  additionalProperties?: boolean | JsonSchema;
  $ref?: string;
  $defs?: Record<string, JsonSchema>;
  definitions?: Record<string, JsonSchema>;
}

export type FieldKind =
  "string" | "number" | "integer" | "boolean" | "enum" | "array" | "object" | "json";

/** Resolves local `$ref`s (`#/$defs/x`, `#/definitions/x`) and unwraps nullable `anyOf`/`oneOf`. */
export function resolve(schema: JsonSchema, root: JsonSchema, depth = 0): JsonSchema {
  if (depth > 20) return {};
  let current = schema;
  if (current.$ref) {
    const target = lookupRef(current.$ref, root);
    if (!target) return {};
    current = { ...resolve(target, root, depth + 1), ...without(current, "$ref") };
  }
  const alternatives = current.anyOf ?? current.oneOf;
  if (alternatives) {
    const nonNull = alternatives.filter((a) => a.type !== "null");
    if (nonNull.length === 1 && nonNull[0]) {
      current = {
        ...resolve(nonNull[0], root, depth + 1),
        ...without(current, "anyOf", "oneOf"),
      };
    }
  }
  return current;
}

function without(schema: JsonSchema, ...keys: (keyof JsonSchema)[]): JsonSchema {
  const copy = { ...schema };
  for (const key of keys) delete copy[key];
  return copy;
}

function lookupRef(ref: string, root: JsonSchema): JsonSchema | undefined {
  if (!ref.startsWith("#/")) return undefined;
  let node: unknown = root;
  for (const part of ref.slice(2).split("/")) {
    if (typeof node !== "object" || node === null) return undefined;
    node = (node as Record<string, unknown>)[part.replace(/~1/g, "/").replace(/~0/g, "~")];
  }
  return typeof node === "object" && node !== null ? (node as JsonSchema) : undefined;
}

/** Decides which editor a schema gets. Anything the form cannot express falls back to raw JSON. */
export function kindOf(schema: JsonSchema, root: JsonSchema = schema): FieldKind {
  const s = resolve(schema, root);
  if (s.enum && s.enum.length > 0) return "enum";
  if (s.anyOf ?? s.oneOf ?? s.allOf) return "json";
  const types = Array.isArray(s.type) ? s.type.filter((t) => t !== "null") : s.type ? [s.type] : [];
  if (types.length !== 1) return s.properties ? "object" : "json";
  switch (types[0]) {
    case "string":
      return "string";
    case "number":
      return "number";
    case "integer":
      return "integer";
    case "boolean":
      return "boolean";
    case "array":
      return s.items ? "array" : "json";
    case "object":
      return s.properties && Object.keys(s.properties).length > 0 ? "object" : "json";
    default:
      return "json";
  }
}

/** The initial value for a schema. Required fields are pre-filled so the form is never empty. */
export function defaultValue(schema: JsonSchema, root: JsonSchema = schema, depth = 0): unknown {
  const s = resolve(schema, root);
  if (s.default !== undefined) return structuredClone(s.default);
  if (s.const !== undefined) return s.const;
  if (depth > 6) return undefined;
  switch (kindOf(s, root)) {
    case "boolean":
      return false;
    case "array":
      return [];
    case "object": {
      const out: Record<string, unknown> = {};
      for (const name of s.required ?? []) {
        const property = s.properties?.[name];
        if (property) {
          const value = defaultValue(property, root, depth + 1);
          if (value !== undefined) out[name] = value;
        }
      }
      return out;
    }
    case "enum":
      return s.enum?.[0];
    default:
      return undefined;
  }
}

function typeName(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value;
}

/** Validates a value against the schema; returns messages prefixed with the JSON pointer of the value. */
export function validate(
  schema: JsonSchema,
  value: unknown,
  root: JsonSchema = schema,
  path = "",
): string[] {
  const s = resolve(schema, root);
  const where = path === "" ? "value" : path;
  if (value === undefined || value === null) {
    const nullable = Array.isArray(s.type) && s.type.includes("null");
    return value === null && !nullable && s.type ? [`${where}: must not be null`] : [];
  }
  const errors: string[] = [];
  if (s.enum && !s.enum.some((option) => JSON.stringify(option) === JSON.stringify(value))) {
    errors.push(`${where}: must be one of ${s.enum.map((o) => JSON.stringify(o)).join(", ")}`);
    return errors;
  }
  switch (kindOf(s, root)) {
    case "string": {
      if (typeof value !== "string") return [`${where}: must be a string`];
      if (s.minLength !== undefined && value.length < s.minLength) {
        errors.push(`${where}: must be at least ${s.minLength} characters`);
      }
      if (s.maxLength !== undefined && value.length > s.maxLength) {
        errors.push(`${where}: must be at most ${s.maxLength} characters`);
      }
      if (s.pattern) {
        try {
          if (!new RegExp(s.pattern).test(value)) errors.push(`${where}: must match ${s.pattern}`);
        } catch {
          // an invalid pattern in the schema cannot be checked
        }
      }
      break;
    }
    case "number":
    case "integer": {
      if (typeof value !== "number" || Number.isNaN(value)) return [`${where}: must be a number`];
      if (kindOf(s, root) === "integer" && !Number.isInteger(value)) {
        errors.push(`${where}: must be an integer`);
      }
      if (s.minimum !== undefined && value < s.minimum)
        errors.push(`${where}: must be ≥ ${s.minimum}`);
      if (s.maximum !== undefined && value > s.maximum)
        errors.push(`${where}: must be ≤ ${s.maximum}`);
      break;
    }
    case "boolean":
      if (typeof value !== "boolean") return [`${where}: must be true or false`];
      break;
    case "array": {
      if (!Array.isArray(value)) return [`${where}: must be an array`];
      if (s.minItems !== undefined && value.length < s.minItems) {
        errors.push(`${where}: needs at least ${s.minItems} items`);
      }
      if (s.maxItems !== undefined && value.length > s.maxItems) {
        errors.push(`${where}: allows at most ${s.maxItems} items`);
      }
      if (s.items) {
        value.forEach((item, index) =>
          errors.push(...validate(s.items as JsonSchema, item, root, `${path}/${index}`)),
        );
      }
      break;
    }
    case "object": {
      if (typeName(value) !== "object") return [`${where}: must be an object`];
      const record = value as Record<string, unknown>;
      for (const name of s.required ?? []) {
        if (record[name] === undefined || record[name] === "") {
          errors.push(`${path}/${name}: is required`);
        }
      }
      for (const [name, property] of Object.entries(s.properties ?? {})) {
        errors.push(...validate(property, record[name], root, `${path}/${name}`));
      }
      break;
    }
    default:
      break;
  }
  return errors;
}

/**
 * Removes what the user left empty so it is not sent: empty strings and undefined for optional
 * fields, and objects that end up empty for optional properties.
 */
export function prune(schema: JsonSchema, value: unknown, root: JsonSchema = schema): unknown {
  const s = resolve(schema, root);
  if (Array.isArray(value)) {
    return value.map((item) => (s.items ? prune(s.items, item, root) : item));
  }
  if (typeof value === "object" && value !== null) {
    const record = value as Record<string, unknown>;
    const required = new Set(s.required ?? []);
    const out: Record<string, unknown> = {};
    for (const [name, item] of Object.entries(record)) {
      const property = s.properties?.[name];
      const cleaned = property ? prune(property, item, root) : item;
      const empty =
        cleaned === undefined ||
        cleaned === "" ||
        (typeof cleaned === "object" &&
          cleaned !== null &&
          !Array.isArray(cleaned) &&
          Object.keys(cleaned).length === 0 &&
          !required.has(name) &&
          property !== undefined &&
          kindOf(property, root) === "object");
      if (!empty || required.has(name))
        out[name] = cleaned === "" && !required.has(name) ? undefined : cleaned;
    }
    return Object.fromEntries(Object.entries(out).filter(([, v]) => v !== undefined));
  }
  return value;
}
