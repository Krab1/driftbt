import { expect, test } from "vitest";
import type { Torrent } from "../api/types";
import { visibleTorrents } from "./view";

const t = (id: string, name: string, state: Torrent["state"], size = 100, eta: number | null = null): Torrent => ({
  id, name, state, error: null, total_bytes: size, progress_bytes: 0, uploaded_bytes: 0, down_bps: 0, up_bps: 0, peers_live: 0, eta_secs: eta, tracker: null, category: null, tags: [], queue_pos: null, note: null, added_at: null, finished_at: null, limit_down_kib: 0, limit_up_kib: 0,
});

const all = [t("a", "Zeta.iso", "seeding", 300), t("b", "alpha.iso", "downloading", 100, 50), t("c", "Mid.iso", "paused", 200)];

test("filters by state, groups checking with downloading", () => {
  expect(visibleTorrents(all, "seeding", "", "name", 1).map((x) => x.id)).toEqual(["a"]);
  expect(visibleTorrents([...all, t("d", "x", "checking")], "downloading", "", "name", 1).map((x) => x.id)).toEqual(["b", "d"]);
});

test("search is case-insensitive and trims", () => {
  expect(visibleTorrents(all, "all", "  ALPHA ", "name", 1).map((x) => x.id)).toEqual(["b"]);
});

test("sorts by name ignoring case, either direction", () => {
  expect(visibleTorrents(all, "all", "", "name", 1).map((x) => x.id)).toEqual(["b", "c", "a"]);
  expect(visibleTorrents(all, "all", "", "name", -1).map((x) => x.id)).toEqual(["a", "c", "b"]);
});

test("torrents without an ETA sort last when ascending", () => {
  expect(visibleTorrents(all, "all", "", "eta", 1).map((x) => x.id)[0]).toBe("b");
});

test("tag filter keeps only torrents carrying that tag, and combines with state", () => {
  const tagged = [{ ...all[0], tags: ["iso", "linux"] }, { ...all[1], tags: ["linux"] }, all[2]];
  expect(visibleTorrents(tagged, "all", "", "name", 1, null, "linux").map((x) => x.id)).toEqual(["b", "a"]);
  expect(visibleTorrents(tagged, "seeding", "", "name", 1, null, "linux").map((x) => x.id)).toEqual(["a"]);
  expect(visibleTorrents(tagged, "all", "", "name", 1, null, null)).toHaveLength(3);
});
