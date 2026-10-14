// Pure helpers behind the file tree used by the Add dialog and the Files tab.

export interface FileLike {
  index: number;
  path: string;
  size: number;
  progress_bytes?: number;
}

export interface TreeNode {
  name: string;
  /** Unique within the tree: folder path or file path. */
  key: string;
  size: number;
  progress: number;
  /** File indices at or below this node. */
  indices: number[];
  children: TreeNode[];
  isFile: boolean;
}

export type CheckState = "all" | "none" | "some";

const byName = (a: TreeNode, b: TreeNode) =>
  a.isFile === b.isFile ? a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }) : a.isFile ? 1 : -1;

/** Groups flat `a/b/c.txt` paths into folders (folders first, then files, natural order). */
export function buildTree(files: FileLike[]): TreeNode[] {
  const root: TreeNode = { name: "", key: "", size: 0, progress: 0, indices: [], children: [], isFile: false };
  for (const f of files) {
    const parts = f.path.split(/[\\/]/).filter(Boolean);
    let node = root;
    parts.forEach((part, i) => {
      const last = i === parts.length - 1;
      const key = parts.slice(0, i + 1).join("/");
      let child = node.children.find((c) => c.key === key);
      if (!child) {
        child = { name: part, key, size: 0, progress: 0, indices: [], children: [], isFile: last };
        node.children.push(child);
      }
      child.size += f.size;
      child.progress += f.progress_bytes ?? 0;
      child.indices.push(f.index);
      node = child;
    });
  }
  const sort = (n: TreeNode) => (n.children.sort(byName), n.children.forEach(sort));
  sort(root);
  return root.children;
}

export function checkState(node: TreeNode, selected: ReadonlySet<number>): CheckState {
  const n = node.indices.filter((i) => selected.has(i)).length;
  return n === 0 ? "none" : n === node.indices.length ? "all" : "some";
}

/** Clicking a fully-checked node unchecks everything under it; anything else checks everything. */
export function toggleNode(node: TreeNode, selected: ReadonlySet<number>): Set<number> {
  const next = new Set(selected);
  const allOn = checkState(node, selected) === "all";
  for (const i of node.indices) allOn ? next.delete(i) : next.add(i);
  return next;
}
