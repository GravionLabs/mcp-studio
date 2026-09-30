import type { CollectionNode, CollectionTree, SavedRequest } from "../../core/bindings";

export interface FolderNode {
  node: CollectionNode;
  folders: FolderNode[];
  requests: SavedRequest[];
}

/** Turns the flat lists from the backend into a nested tree. Orphans (unknown parent) become roots. */
export function buildTree(tree: CollectionTree): FolderNode[] {
  const byId = new Map<string, FolderNode>(
    tree.collections.map((node) => [node.id, { node, folders: [], requests: [] }]),
  );
  const roots: FolderNode[] = [];
  for (const folder of byId.values()) {
    const parent = folder.node.parentId ? byId.get(folder.node.parentId) : undefined;
    (parent ? parent.folders : roots).push(folder);
  }
  for (const request of tree.requests) {
    byId.get(request.collectionId)?.requests.push(request);
  }
  return roots;
}

export interface FolderOption {
  id: string;
  /** Path such as `Smoke tests / Echo`. */
  label: string;
}

/** All folders as a flat list with their path, in tree order (for pickers). */
export function folderOptions(roots: FolderNode[]): FolderOption[] {
  const out: FolderOption[] = [];
  const visit = (folder: FolderNode, prefix: string) => {
    const label = prefix ? `${prefix} / ${folder.node.name}` : folder.node.name;
    out.push({ id: folder.node.id, label });
    folder.folders.forEach((child) => visit(child, label));
  };
  roots.forEach((root) => visit(root, ""));
  return out;
}

/** Number of requests in a folder including all subfolders. */
export function countRequests(folder: FolderNode): number {
  return (
    folder.requests.length + folder.folders.reduce((sum, child) => sum + countRequests(child), 0)
  );
}

/** Ids of a folder and all folders below it (used to prevent moving a folder into itself in the UI). */
export function descendantIds(folder: FolderNode): string[] {
  return [folder.node.id, ...folder.folders.flatMap(descendantIds)];
}
