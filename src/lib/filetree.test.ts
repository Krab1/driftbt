import { expect, test } from "vitest";
import { buildTree, checkState, isMedia, toggleNode } from "./filetree";

const files = [
  { index: 0, path: "docs/manual.pdf", size: 10, progress_bytes: 5 },
  { index: 1, path: "README.txt", size: 1 },
  { index: 2, path: "docs/licenses/GPL-3.txt", size: 4 },
  { index: 3, path: "images\\boot.img", size: 100 },
];

test("groups into folders, folders first, sizes and progress roll up", () => {
  const tree = buildTree(files);
  expect(tree.map((n) => n.name)).toEqual(["docs", "images", "README.txt"]);
  const docs = tree[0];
  expect(docs.size).toBe(14);
  expect(docs.progress).toBe(5);
  expect(docs.indices.sort()).toEqual([0, 2]);
  expect(docs.children.map((n) => n.name)).toEqual(["licenses", "manual.pdf"]);
  expect(tree[1].children[0].name).toBe("boot.img"); // backslash paths split too
});

test("natural ordering: file2 before file10", () => {
  const t = buildTree([
    { index: 0, path: "file10.bin", size: 1 },
    { index: 1, path: "file2.bin", size: 1 },
  ]);
  expect(t.map((n) => n.name)).toEqual(["file2.bin", "file10.bin"]);
});

test("tri-state and toggling", () => {
  const docs = buildTree(files)[0];
  expect(checkState(docs, new Set([0, 2]))).toBe("all");
  expect(checkState(docs, new Set([0]))).toBe("some");
  expect(checkState(docs, new Set([3]))).toBe("none");
  expect([...toggleNode(docs, new Set([0, 2, 3]))]).toEqual([3]); // all -> none, others untouched
  expect([...toggleNode(docs, new Set([0])).values()].sort()).toEqual([0, 2]); // some -> all
  expect([...toggleNode(docs, new Set()).values()].sort()).toEqual([0, 2]); // none -> all
});

test("isMedia is extension-based and case-insensitive", () => {
  expect(isMedia("Show/E01.MKV")).toBe(true);
  expect(isMedia("a.flac")).toBe(true);
  expect(isMedia("readme.txt")).toBe(false);
  expect(isMedia("noext")).toBe(false);
  expect(isMedia("archive.mkv.part")).toBe(false);
});
