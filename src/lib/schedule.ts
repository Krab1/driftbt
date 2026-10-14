import type { ScheduleSlot } from "./api/types";

export const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;
export const EVERY_DAY = 127;

/** 570 -> "09:30" */
export function toTime(min: number): string {
  const m = ((Math.floor(min) % 1440) + 1440) % 1440;
  return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

/** "09:30" -> 570; anything else -> null */
export function fromTime(text: string): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(text.trim());
  if (!m) return null;
  const [h, mm] = [Number(m[1]), Number(m[2])];
  return h < 24 && mm < 60 ? h * 60 + mm : null;
}

export function hasDay(days: number, day: number): boolean {
  return (days & (1 << day)) !== 0;
}

export function toggleDay(days: number, day: number): number {
  return days ^ (1 << day);
}

/** One line saying when a slot applies, e.g. "Mon–Fri 09:00–17:00" or "Every day 23:00–07:00 (overnight)". */
export function describeSlot(s: ScheduleSlot): string {
  const on = DAYS.map((_, i) => hasDay(s.days, i));
  let days: string;
  if (on.every(Boolean)) days = "Every day";
  else if (!on.some(Boolean)) days = "No days";
  else {
    const parts: string[] = [];
    for (let i = 0; i < 7; i++) {
      if (!on[i]) continue;
      let j = i;
      while (j + 1 < 7 && on[j + 1]) j++;
      parts.push(j - i >= 2 ? `${DAYS[i]}–${DAYS[j]}` : DAYS.slice(i, j + 1).join(", "));
      i = j;
    }
    days = parts.join(", ");
  }
  const time = s.from_min === s.to_min ? "all day" : `${toTime(s.from_min)}–${toTime(s.to_min)}${s.from_min > s.to_min ? " (overnight)" : ""}`;
  return `${days} ${time}`;
}
