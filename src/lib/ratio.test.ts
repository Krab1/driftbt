import { expect, test } from "vitest";
import type { Torrent } from "./api/types";
import { NO_TRACKER, ratioTable } from "./ratio";

const GB = 1024 ** 3;
const t = (id: string, tracker: string | null, size: number, uploaded: number, up = 0): Torrent => ({
  id, name: id, state: "seeding", error: null, total_bytes: size, progress_bytes: size, uploaded_bytes: uploaded, down_bps: 0, up_bps: up,
  peers_live: 0, eta_secs: null, tracker, category: null, tags: [], queue_pos: null, note: null, added_at: null, finished_at: null, limit_down_kib: 0, limit_up_kib: 0,
});

test("groups by tracker and computes ratio and the real upload still needed", () => {
  const rows = ratioTable([t("a", "x.org", 2 * GB, GB), t("b", "x.org", 2 * GB, 3 * GB), t("c", "y.net", GB, 0)], 1);
  const x = rows.find((r) => r.tracker === "x.org")!;
  expect(x.torrents).toBe(2);
  expect(x.ratio).toBe(1);
  expect(x.gap).toBe(0);
  const y = rows.find((r) => r.tracker === "y.net")!;
  expect(y.gap).toBe(GB);
  expect(rows[0].tracker).toBe("y.net"); // biggest gap first
});

test("eta only when something is uploading and there is a gap", () => {
  const [row] = ratioTable([t("a", "x.org", GB, 0, 1024 * 1024)], 1);
  expect(row.etaSecs).toBe(1024);
  expect(ratioTable([t("a", "x.org", GB, 0, 0)], 1)[0].etaSecs).toBeNull();
  expect(ratioTable([t("a", "x.org", GB, 2 * GB, 100)], 1)[0].etaSecs).toBeNull();
});

test("trackerless torrents are grouped, and torrents without metadata are skipped", () => {
  const rows = ratioTable([t("a", null, GB, 0), t("b", "x.org", 0, 0)], 2);
  expect(rows.map((r) => r.tracker)).toEqual([NO_TRACKER]);
  expect(rows[0].gap).toBe(2 * GB);
});

test("a higher target needs more upload", () => {
  const list = [t("a", "x.org", GB, GB / 2)];
  expect(ratioTable(list, 1)[0].gap).toBe(GB / 2);
  expect(ratioTable(list, 2)[0].gap).toBe(1.5 * GB);
});
