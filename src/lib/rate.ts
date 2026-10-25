/** 10 Gbit/s in KiB/s. The core holds to the same number (the engine counts bytes per second in 32 bits, so 100 Gbit/s would not fit). */
export const MAX_KIB = 1_220_703;

export type Rate = { ok: true; kib: number } | { ok: false; why: string };

/** A speed typed by a person: plain digits with an optional short fraction, nothing else (no signs, exponents, spaces inside, "Infinity"). */
export function parseRate(text: string, mb: boolean): Rate {
  const t = text.trim().replace(",", ".");
  if (t === "") return { ok: false, why: "Type a speed, or choose Unlimited in the list." };
  if (!/^\d{1,7}(\.\d{1,3})?$/.test(t)) return { ok: false, why: "Use digits only, for example 3 or 2.5." };
  if (!mb && t.includes(".")) return { ok: false, why: "In KB/s use a whole number, or switch to MB/s." };
  const kib = Math.round(Number(t) * (mb ? 1024 : 1));
  if (!Number.isFinite(kib) || kib < 1) return { ok: false, why: "That is too small. The lowest speed is 1 KB/s." };
  if (kib > MAX_KIB) return { ok: false, why: `That is too fast. The highest speed is ${Math.floor(MAX_KIB / 1024)} MB/s (10 Gbit/s).` };
  return { ok: true, kib };
}
