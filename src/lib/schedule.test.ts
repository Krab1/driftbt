import { expect, test } from "vitest";
import { describeSlot, fromTime, hasDay, toggleDay, toTime } from "./schedule";

test("time conversion round-trips and rejects nonsense", () => {
  expect(toTime(570)).toBe("09:30");
  expect(toTime(0)).toBe("00:00");
  expect(toTime(1439)).toBe("23:59");
  expect(fromTime("09:30")).toBe(570);
  expect(fromTime("9:05")).toBe(545);
  expect(fromTime("24:00")).toBeNull();
  expect(fromTime("12:60")).toBeNull();
  expect(fromTime("noon")).toBeNull();
  for (const m of [0, 1, 59, 60, 725, 1439]) expect(fromTime(toTime(m))).toBe(m);
});

test("day bits", () => {
  let d = 0;
  d = toggleDay(d, 0);
  d = toggleDay(d, 6);
  expect(d).toBe(65);
  expect(hasDay(d, 0) && hasDay(d, 6) && !hasDay(d, 3)).toBe(true);
  expect(toggleDay(d, 0)).toBe(64);
});

test("slots read like a sentence", () => {
  const slot = (days: number, from: number, to: number) => ({ days, from_min: from, to_min: to, down_kib: 0, up_kib: 0 });
  expect(describeSlot(slot(31, 540, 1020))).toBe("Mon–Fri 09:00–17:00");
  expect(describeSlot(slot(127, 1380, 420))).toBe("Every day 23:00–07:00 (overnight)");
  expect(describeSlot(slot(96, 0, 0))).toBe("Sat, Sun all day");
  expect(describeSlot(slot(0, 60, 120))).toBe("No days 01:00–02:00");
  expect(describeSlot(slot(1 | 4 | 16, 60, 120))).toBe("Mon, Wed, Fri 01:00–02:00");
});
