import { expect, test } from "vitest";
import { MAX_KIB, parseRate } from "./rate";

const kib = (t: string, mb: boolean) => {
  const r = parseRate(t, mb);
  return r.ok ? r.kib : null;
};

test("plain numbers and short fractions are taken, in either unit", () => {
  expect(kib("3", true)).toBe(3072);
  expect(kib(" 2.5 ", true)).toBe(2560);
  expect(kib("2,5", true)).toBe(2560);
  expect(kib("800", false)).toBe(800);
  expect(kib("1", false)).toBe(1);
});

test("everything else is refused", () => {
  for (const bad of ["", " ", "0", "0.0", "-5", "+5", "1e3", "0x10", "Infinity", "NaN", "5 5", "1.2345", ".5", "5.", "12345678", "1_000", "٣", "5MB", "<b>", "1;2"]) {
    expect(parseRate(bad, true).ok, bad).toBe(false);
  }
  expect(parseRate("0.0001", true).ok).toBe(false);
  expect(parseRate("1.5", false).ok, "fractions of a KB/s").toBe(false);
});

test("the engine's ceiling holds", () => {
  expect(kib(String(MAX_KIB), false)).toBe(MAX_KIB);
  expect(parseRate(String(MAX_KIB + 1), false).ok).toBe(false);
  expect(parseRate("1193", true).ok).toBe(false);
  expect(kib("1192", true)).toBe(1192 * 1024);
});
