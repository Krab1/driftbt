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

const MEDIA = new Set(["mkv", "mp4", "m4v", "avi", "mov", "webm", "wmv", "ts", "mpg", "mpeg", "flv", "mp3", "flac", "ogg", "opus", "wav", "m4a", "aac", "wma"]);

/** Same list as the backend's `playlist::is_media`. */
export function isMedia(path: string): boolean {
  const dot = path.lastIndexOf(".");
  return dot >= 0 && MEDIA.has(path.slice(dot + 1).toLowerCase());
}

export type CheckState = "all" | "none" | "some";

// One collator for every comparison: `localeCompare` with options sets one up per call.
const names = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
const byName = (a: TreeNode, b: TreeNode) => (a.isFile === b.isFile ? names.compare(a.name, b.name) : a.isFile ? 1 : -1);

/** Groups flat `a/b/c.txt` paths into folders (folders first, then files, natural order). */
export function buildTree(files: FileLike[]): TreeNode[] {
  const root: TreeNode = { name: "", key: "", size: 0, progress: 0, indices: [], children: [], isFile: false };
  // Every node by its key, so finding a file's folder doesn't walk the folder's children (20,000 files in one
  // folder took 600 ms that way, and the tree is rebuilt on every details update).
  const byKey = new Map<string, TreeNode>();
  for (const f of files) {
    const parts = f.path.split(/[\\/]/).filter(Boolean);
    let node = root;
    let key = "";
    parts.forEach((part, i) => {
      key = i ? `${key}/${part}` : part;
      let child = byKey.get(key);
      if (!child) {
        child = { name: part, key, size: 0, progress: 0, indices: [], children: [], isFile: i === parts.length - 1 };
        byKey.set(key, child);
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
