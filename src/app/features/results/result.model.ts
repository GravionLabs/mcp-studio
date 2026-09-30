/** A piece of content in an MCP tool result, prompt message, or resource, normalized for display. */
export type ResultBlock =
  | { kind: "text"; text: string; json: boolean }
  | { kind: "image"; src: string; mimeType: string }
  | { kind: "audio"; src: string; mimeType: string }
  | {
      kind: "resource";
      uri: string;
      mimeType: string | null;
      text: string | null;
      blob: string | null;
    }
  | { kind: "resourceLink"; uri: string; name: string; description: string | null }
  | { kind: "unknown"; value: unknown };

export interface ParsedResult {
  blocks: ResultBlock[];
  /** `structuredContent`, when the tool returned it. */
  structured: unknown;
  isError: boolean;
}

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const str = (value: unknown): string | null => (typeof value === "string" ? value : null);

function isJsonText(text: string): boolean {
  const trimmed = text.trim();
  if (!(trimmed.startsWith("{") || trimmed.startsWith("["))) return false;
  try {
    JSON.parse(trimmed);
    return true;
  } catch {
    return false;
  }
}

/** Only `image/*` and `audio/*` data is embedded; anything else must not become an <img> source. */
function dataUrl(
  mimeType: string | null,
  data: string | null,
  family: "image" | "audio",
): string | null {
  if (!data || !mimeType?.toLowerCase().startsWith(`${family}/`)) return null;
  if (mimeType.toLowerCase().includes("svg")) return null;
  if (!/^[A-Za-z0-9+/=\s]+$/.test(data)) return null;
  return `data:${mimeType};base64,${data.replace(/\s+/g, "")}`;
}

/** Normalizes one MCP content item (`type` = text | image | audio | resource | resource_link). */
export function parseContent(item: unknown): ResultBlock {
  if (!isObject(item)) return { kind: "unknown", value: item };
  switch (item["type"]) {
    case "text": {
      const text = str(item["text"]) ?? "";
      return { kind: "text", text, json: isJsonText(text) };
    }
    case "image": {
      const mimeType = str(item["mimeType"]) ?? "";
      const src = dataUrl(mimeType, str(item["data"]), "image");
      return src ? { kind: "image", src, mimeType } : { kind: "unknown", value: item };
    }
    case "audio": {
      const mimeType = str(item["mimeType"]) ?? "";
      const src = dataUrl(mimeType, str(item["data"]), "audio");
      return src ? { kind: "audio", src, mimeType } : { kind: "unknown", value: item };
    }
    case "resource": {
      const resource = item["resource"];
      if (!isObject(resource)) return { kind: "unknown", value: item };
      return {
        kind: "resource",
        uri: str(resource["uri"]) ?? "",
        mimeType: str(resource["mimeType"]),
        text: str(resource["text"]),
        blob: str(resource["blob"]),
      };
    }
    case "resource_link":
      return {
        kind: "resourceLink",
        uri: str(item["uri"]) ?? "",
        name: str(item["name"]) ?? str(item["uri"]) ?? "",
        description: str(item["description"]),
      };
    default:
      return { kind: "unknown", value: item };
  }
}

/** Parses a `CallToolResult` as returned by the backend. */
export function parseToolResult(result: unknown): ParsedResult {
  if (!isObject(result)) return { blocks: [], structured: undefined, isError: false };
  const content = Array.isArray(result["content"]) ? result["content"] : [];
  return {
    blocks: content.map(parseContent),
    structured: result["structuredContent"],
    isError: result["isError"] === true,
  };
}

/** Pretty-prints JSON text; leaves anything else untouched. */
export function prettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}
