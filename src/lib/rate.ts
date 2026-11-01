import { t } from "./i18n.svelte";

/** 10 Gbit/s in KiB/s. The core holds to the same number (the engine counts bytes per second in 32 bits, so 100 Gbit/s would not fit). */
export const MAX_KIB = 1_220_703;

export type Rate = { ok: true; kib: number } | { ok: false; why: string };

/** A speed typed by a person: plain digits with an optional short fraction, nothing else (no signs, exponents, spaces inside, "Infinity"). */
export function parseRate(text: string, mb: boolean): Rate {
  const v = text.trim().replace(",", ".");
  if (v === "") return { ok: false, why: t("Type a speed, or choose Unlimited in the list.") };
  if (!/^\d{1,7}(\.\d{1,3})?$/.test(v)) return { ok: false, why: t("Use digits only, for example 3 or 2.5.") };
  if (!mb && v.includes(".")) return { ok: false, why: t("In KB/s use a whole number, or switch to MB/s.") };
  const kib = Math.round(Number(v) * (mb ? 1024 : 1));
  if (!Number.isFinite(kib) || kib < 1) return { ok: false, why: t("That is too small. The lowest speed is 1 KB/s.") };
  if (kib > MAX_KIB) return { ok: false, why: t("That is too fast. The highest speed is {max} MB/s (10 Gbit/s).", { max: Math.floor(MAX_KIB / 1024) }) };
  return { ok: true, kib };
}
