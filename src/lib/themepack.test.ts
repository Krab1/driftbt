import { describe, expect, it } from "vitest";
import { contrast, opaque, parsePack, warnings } from "./themepack";

const ok = (tokens: Record<string, string>, base = "dark") => JSON.stringify({ drift_theme: 1, name: "T", base, tokens });

describe("theme packs", () => {
  it("accepts a pack and accepts names with or without the dashes", () => {
    const r = parsePack(ok({ bg: "#101820", "--accent": "rgb(10, 200, 30)" }));
    expect(r.error).toBeUndefined();
    expect(r.pack?.tokens).toEqual({ bg: "#101820", accent: "rgb(10, 200, 30)" });
  });

  it("refuses what isn't a pack, with a plain message", () => {
    expect(parsePack("nope").error).toMatch(/valid JSON/);
    expect(parsePack("[]").error).toMatch(/isn't a theme pack/);
    expect(parsePack(JSON.stringify({ name: "x" })).error).toMatch(/drift_theme/);
    expect(parsePack(ok({ bg: "red; background: url(x)" })).error).toMatch(/isn't valid/);
    expect(parsePack(ok({ "font": "#fff" })).error).toMatch(/isn't a colour/);
    expect(parsePack(ok({})).error).toMatch(/doesn't change/);
    expect(parsePack(JSON.stringify({ drift_theme: 1, name: "T", base: "dark", tokens: { bg: "#000" }, script: "x" })).error).toMatch(/doesn't know/);
    expect(parsePack(ok({ bg: "#000" }, "purple")).error).toMatch(/base/);
    expect(parsePack("x".repeat(30000)).error).toMatch(/too big/);
  });

  it("measures contrast like WCAG", () => {
    expect(contrast([0, 0, 0], [255, 255, 255])).toBeCloseTo(21, 0);
    expect(opaque("#fff")).toEqual([255, 255, 255]);
    expect(opaque("#ffffff80")).toBeNull();
    expect(opaque("rgba(1,2,3,0.5)")).toBeNull();
  });

  it("warns under 4.5:1 and uses the base colours for tokens the pack leaves out", () => {
    const low = parsePack(ok({ bg: "#808080" })).pack!; // grey behind the dark theme's light text
    expect(warnings(low).some((w) => w.startsWith("Main text on the background"))).toBe(true);
    const fine = parsePack(ok({ accent: "#5e94ff" })).pack!;
    expect(warnings(fine)).toEqual([]);
    const seeThrough = parsePack(ok({ bg: "rgba(128,128,128,0.5)" })).pack!;
    expect(warnings(seeThrough).some((w) => w.includes("on the background"))).toBe(false);
  });
});
