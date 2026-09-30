import { describe, expect, it } from "vitest";
import type { CollectionNode, CollectionTree, SavedRequest } from "../../core/bindings";
import { buildTree, countRequests, descendantIds, folderOptions } from "./collections.model";

const folder = (id: string, parentId: string | null, name: string): CollectionNode => ({
  id,
  parentId,
  name,
  sortOrder: 0,
});

const request = (id: string, collectionId: string): SavedRequest => ({
  id,
  collectionId,
  serverId: "s",
  method: "tools/call",
  name: id,
  toolName: "echo",
  arguments: {},
  notes: "",
  sortOrder: 0,
  updatedAt: 1,
});

const tree: CollectionTree = {
  collections: [folder("a", null, "Smoke"), folder("b", "a", "Echo"), folder("c", null, "Other")],
  requests: [request("r1", "b"), request("r2", "a"), request("r3", "b")],
};

describe("buildTree", () => {
  it("nests folders and attaches requests", () => {
    const roots = buildTree(tree);
    expect(roots.map((r) => r.node.id)).toEqual(["a", "c"]);
    expect(roots[0]?.folders[0]?.node.id).toBe("b");
    expect(roots[0]?.requests.map((r) => r.id)).toEqual(["r2"]);
    expect(roots[0]?.folders[0]?.requests.map((r) => r.id)).toEqual(["r1", "r3"]);
  });

  it("treats folders with an unknown parent as roots and ignores stray requests", () => {
    const roots = buildTree({
      collections: [folder("x", "missing", "Orphan")],
      requests: [request("lost", "nowhere")],
    });
    expect(roots.map((r) => r.node.id)).toEqual(["x"]);
    expect(roots[0]?.requests).toEqual([]);
  });

  it("handles an empty tree", () => {
    expect(buildTree({ collections: [], requests: [] })).toEqual([]);
  });
});

describe("helpers", () => {
  it("lists folder paths in tree order", () => {
    expect(folderOptions(buildTree(tree))).toEqual([
      { id: "a", label: "Smoke" },
      { id: "b", label: "Smoke / Echo" },
      { id: "c", label: "Other" },
    ]);
  });

  it("counts requests including subfolders", () => {
    const [smoke] = buildTree(tree);
    expect(smoke && countRequests(smoke)).toBe(3);
  });

  it("collects descendant ids", () => {
    const [smoke] = buildTree(tree);
    expect(smoke && descendantIds(smoke)).toEqual(["a", "b"]);
  });
});
