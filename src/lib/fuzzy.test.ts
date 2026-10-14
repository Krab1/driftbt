import { expect, test } from "vitest";
import { fuzzy, rank } from "./fuzzy";

test("letters must appear in order, case-insensitively", () => {
  expect(fuzzy("pst", "Pause torrent")).not.toBeNull();
  expect(fuzzy("tsp", "Pause torrent")).toBeNull();
  expect(fuzzy("PAUSE", "pause all")).not.toBeNull();
  expect(fuzzy("", "anything")).toBe(0);
});

test("word starts and consecutive letters rank higher", () => {
  const items = ["Reorder paused", "Pause all", "Set speed limit"];
  expect(rank(items, "pau", (x) => x)[0]).toBe("Pause all");
  expect(rank(items, "zzz", (x) => x)).toEqual([]);
});

test("an empty query keeps the original order", () => {
  const items = ["b", "a", "c"];
  expect(rank(items, "  ", (x) => x)).toEqual(items);
});

test("shorter text wins a tie", () => {
  expect(rank(["Add torrent file", "Add"], "add", (x) => x)[0]).toBe("Add");
});
