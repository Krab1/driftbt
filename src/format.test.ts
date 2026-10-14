import { expect, test } from "vitest";
import { formatBytes, formatEta, formatSpeed, percent } from "./format";

test("formatBytes", () => {
  expect(formatBytes(0)).toBe("0 B");
  expect(formatBytes(1536)).toBe("1.5 KB");
  expect(formatBytes(6.2 * 1024 ** 3)).toBe("6.2 GB");
  expect(formatBytes(-1)).toBe("0 B");
});

test("formatSpeed", () => {
  expect(formatSpeed(0)).toBe("—");
  expect(formatSpeed(14.2 * 1024 * 1024)).toBe("14 MB/s");
});

test("formatEta", () => {
  expect(formatEta(null)).toBe("—");
  expect(formatEta(-5)).toBe("—");
  expect(formatEta(45)).toBe("45s");
  expect(formatEta(240)).toBe("4m");
  expect(formatEta(4320)).toBe("1h 12m");
  expect(formatEta(2 * 86400 + 3 * 3600)).toBe("2d 3h");
});

test("percent never rounds up to 100", () => {
  expect(percent(0, 0)).toBe(0);
  expect(percent(999, 1000)).toBe(99);
  expect(percent(1000, 1000)).toBe(100);
});
