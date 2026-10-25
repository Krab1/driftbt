<script lang="ts">
  import type { ScheduleSlot } from "../api";
  import { MAX_KIB } from "../rate";
  import { DAYS, describeSlot, fromTime, hasDay, toggleDay, toTime } from "../schedule";
  import { ui } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  const schedule = $derived(ui.settings!.schedule);

  const save = (slots: ScheduleSlot[], enabled = schedule.enabled) => ui.saveSettings({ schedule: { enabled, slots } });
  const patch = (i: number, p: Partial<ScheduleSlot>) => save(schedule.slots.map((s, j) => (j === i ? { ...s, ...p } : s)));
  const kib = (e: Event) => {
    const n = Math.floor(Number((e.currentTarget as HTMLInputElement).value));
    return Number.isFinite(n) && n > 0 ? Math.min(n, MAX_KIB) : 0;
  };
  const time = (e: Event, fallback: number) => fromTime((e.currentTarget as HTMLInputElement).value) ?? fallback;
</script>

<div class="field">
  <span>Scheduled speed limits</span>
  <label class="check"><input type="checkbox" checked={schedule.enabled} onchange={(e) => save(schedule.slots, e.currentTarget.checked)} /> Use different limits at certain times</label>
  <small>While a slot is active its limits replace the ones above, even when "Limit transfer speed" is off. Zero means no limit in that direction.</small>

  {#each schedule.slots as s, i (i)}
    <div class="slot" role="group" aria-label="Slot {i + 1}: {describeSlot(s)}">
      <div class="days" role="group" aria-label="Days">
        {#each DAYS as d, day (d)}
          <button class="day" class:on={hasDay(s.days, day)} aria-pressed={hasDay(s.days, day)} aria-label={d} onclick={() => patch(i, { days: toggleDay(s.days, day) })}>{d[0]}</button>
        {/each}
      </div>
      <label>From <input type="time" value={toTime(s.from_min)} onchange={(e) => patch(i, { from_min: time(e, s.from_min) })} /></label>
      <label>to <input type="time" value={toTime(s.to_min)} onchange={(e) => patch(i, { to_min: time(e, s.to_min) })} /></label>
      <label>Down <input type="number" min="0" step="50" inputmode="numeric" value={s.down_kib || ""} placeholder="None" onchange={(e) => patch(i, { down_kib: kib(e) })} /> KiB/s</label>
      <label>Up <input type="number" min="0" step="50" inputmode="numeric" value={s.up_kib || ""} placeholder="None" onchange={(e) => patch(i, { up_kib: kib(e) })} /> KiB/s</label>
      <button class="btn ghost icon" aria-label="Remove slot {i + 1}" onclick={() => save(schedule.slots.filter((_, j) => j !== i))}><Icon name="x" size={14} /></button>
      <small class="sum">{describeSlot(s)}{s.from_min > s.to_min ? " — runs past midnight into the next day" : ""}</small>
    </div>
  {/each}

  <button class="btn add" onclick={() => save([...schedule.slots, { days: 127, from_min: 23 * 60, to_min: 7 * 60, down_kib: 0, up_kib: 0 }], true)}><Icon name="plus" />Add a time slot</button>
</div>

<style>
  .field {
    display: grid;
    gap: 8px;
  }
  .field > span {
    color: var(--text-secondary);
    font-size: 12px;
  }
  small {
    color: var(--text-muted);
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  input[type="checkbox"] {
    accent-color: var(--accent);
    margin: 0;
  }
  .slot {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 12px;
    align-items: center;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .slot label {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .days {
    display: flex;
    gap: 2px;
  }
  .day {
    width: 26px;
    height: 26px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: transparent;
    font-size: 11px;
  }
  .day.on {
    background: var(--selected);
    color: var(--accent-fg);
    border-color: var(--accent);
    font-weight: 500;
  }
  input[type="time"],
  input[type="number"] {
    height: 28px;
    padding: 0 6px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    user-select: text;
  }
  input[type="number"] {
    width: 78px;
    text-align: right;
  }
  .sum {
    flex-basis: 100%;
  }
  .add {
    width: fit-content;
  }
</style>
