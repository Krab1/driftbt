import { expect, test } from "vitest";
import { formatBytes } from "./format";

test("formatBytes", () => {
  expect(formatBytes(0)).toBe("0 B");
  expect(formatBytes(1536)).toBe("1.5 KB");
  expect(formatBytes(6.2 * 1024 ** 3)).toBe("6.2 GB");
  expect(formatBytes(-1)).toBe("0 B");
});
