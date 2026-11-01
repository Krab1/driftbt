import { expect, test } from "vitest";
import { locale, pick, render, resolveLanguage, setLanguage, t, tn } from "./i18n.svelte";

test("placeholders are filled and unknown ones are left alone", () => {
  expect(render("{n} of {total}", { n: 3, total: "9" })).toBe("3 of 9");
  expect(render("{n} of {total}", { n: 3 })).toBe("3 of {total}");
  expect(render("no holes {}", { n: 1 })).toBe("no holes {}");
});

test("plural forms follow the language's rule", () => {
  const ru = { one: "{n} торрент", few: "{n} торрента", many: "{n} торрентов", other: "{n} торрента" };
  const form = (n: number) => pick(ru, "x", "ru", n);
  expect([1, 2, 5, 11, 21, 22, 25].map(form)).toEqual(["{n} торрент", "{n} торрента", "{n} торрентов", "{n} торрентов", "{n} торрент", "{n} торрента", "{n} торрентов"]);
  expect(pick(undefined, "English", "ru")).toBe("English");
  expect(pick("Привет", "Hello", "ru")).toBe("Привет");
});

test("without a catalog the English text shows, singular or plural", () => {
  expect(t("Add {what}", { what: "x" })).toBe("Add x");
  expect(tn("{n} torrent", "{n} torrents", 1)).toBe("1 torrent");
  expect(tn("{n} torrent", "{n} torrents", 0)).toBe("0 torrents");
  expect(tn("{n} torrent", "{n} torrents", 2)).toBe("2 torrents");
});

test("system follows the computer, a region falls back to its language, anything unknown is English", () => {
  expect(resolveLanguage("system", ["ru-RU", "en-US"])).toBe("ru");
  expect(resolveLanguage("system", ["fr-FR", "de-AT"])).toBe("de");
  expect(resolveLanguage("system", ["fr-FR"])).toBe("en");
  expect(resolveLanguage("de", ["ru"])).toBe("de");
  expect(resolveLanguage("DE-ch", [])).toBe("de");
  expect(resolveLanguage("klingon", [])).toBe("en");
  expect(resolveLanguage("../../x", [])).toBe("en");
});

test("a language file is loaded and used, and switching back to English clears it", async () => {
  await setLanguage("ru");
  expect(t("Cancel")).toBe("Отмена");
  expect(t("Language")).toBe("Язык");
  expect(tn("{n} torrent", "{n} torrents", 3)).toBe("3 торрента");
  expect(tn("{n} torrent", "{n} torrents", 5)).toBe("5 торрентов");
  expect(t("A text nobody translated")).toBe("A text nobody translated");
  await setLanguage("en");
  expect(t("Cancel")).toBe("Cancel");
});

test("two quick changes: the last one wins, whichever file loads first", async () => {
  const slow = setLanguage("ru");
  await setLanguage("en");
  await slow;
  expect(t("Cancel")).toBe("Cancel");
  const first = setLanguage("de");
  await setLanguage("ru");
  await first;
  expect(t("Cancel")).toBe("Отмена");
  await setLanguage("en");
});

test("formats follow the computer while the choice is system, and the chosen language otherwise", async () => {
  await setLanguage("system");
  expect(locale()).toBeUndefined();
  await setLanguage("ru");
  expect(locale()).toBe("ru");
  await setLanguage("en");
});

test("a placeholder never picks up an inherited property", () => {
  expect(render("x {toString} {n}", { n: 1 })).toBe("x {toString} 1");
});
