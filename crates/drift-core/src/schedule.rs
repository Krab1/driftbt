//! Weekly alt-speed schedule: different speed limits at certain times (night, work hours). Pure.

use serde::{Deserialize, Serialize};

pub const MON: u8 = 1;
pub const TUE: u8 = 2;
pub const WED: u8 = 4;
pub const THU: u8 = 8;
pub const FRI: u8 = 16;
pub const SAT: u8 = 32;
pub const SUN: u8 = 64;
pub const EVERY_DAY: u8 = 127;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    /// Bit 0 = Monday ... bit 6 = Sunday.
    pub days: u8,
    /// Minutes after local midnight, `0..1440`.
    pub from_min: u16,
    pub to_min: u16,
    /// KiB/s while the slot is active; 0 = unlimited in that direction.
    pub down_kib: u32,
    pub up_kib: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub enabled: bool,
    pub slots: Vec<Slot>,
}

impl Slot {
    /// `weekday`: 0 = Monday ... 6 = Sunday. A slot whose end is before its start runs past midnight into
    /// the next day (from 22:00 to 06:00 on Friday covers Saturday morning); equal times mean the whole day.
    fn covers(&self, weekday: u8, minute: u16) -> bool {
        let on = |d: u8| self.days & (1 << (d % 7)) != 0;
        let (from, to) = (self.from_min, self.to_min);
        if from == to {
            on(weekday)
        } else if from < to {
            on(weekday) && (from..to).contains(&minute)
        } else {
            // overnight: the evening part belongs to this day, the morning part to the day before
            (on(weekday) && minute >= from) || (on((weekday + 6) % 7) && minute < to)
        }
    }
}

/// The slot in force now, if any. When slots overlap the first one listed wins.
pub fn active(s: &Schedule, weekday: u8, minute: u16) -> Option<&Slot> {
    if !s.enabled {
        return None;
    }
    s.slots.iter().find(|slot| slot.covers(weekday, minute))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sched(slots: Vec<Slot>) -> Schedule {
        Schedule { enabled: true, slots }
    }

    fn slot(days: u8, from: u16, to: u16) -> Slot {
        Slot { days, from_min: from, to_min: to, down_kib: 100, up_kib: 50 }
    }

    const H: u16 = 60;

    #[test]
    fn a_daytime_slot_on_chosen_days_only() {
        let s = sched(vec![slot(MON | TUE | WED | THU | FRI, 9 * H, 17 * H)]);
        assert!(active(&s, 0, 9 * H).is_some(), "start is inclusive");
        assert!(active(&s, 4, 17 * H - 1).is_some());
        assert!(active(&s, 4, 17 * H).is_none(), "end is exclusive");
        assert!(active(&s, 0, 8 * H + 59).is_none());
        assert!(active(&s, 5, 12 * H).is_none(), "Saturday is not selected");
    }

    #[test]
    fn an_overnight_slot_runs_into_the_next_morning() {
        let s = sched(vec![slot(FRI, 22 * H, 6 * H)]);
        assert!(active(&s, 4, 23 * H).is_some(), "Friday night");
        assert!(active(&s, 5, 5 * H + 59).is_some(), "Saturday early morning belongs to Friday's slot");
        assert!(active(&s, 5, 6 * H).is_none());
        assert!(active(&s, 4, 5 * H).is_none(), "Friday early morning does not (Thursday isn't selected)");
        assert!(active(&s, 5, 23 * H).is_none(), "Saturday night does not");
    }

    #[test]
    fn sunday_night_wraps_to_monday() {
        let s = sched(vec![slot(SUN, 23 * H, 7 * H)]);
        assert!(active(&s, 6, 23 * H + 30).is_some());
        assert!(active(&s, 0, 3 * H).is_some(), "Monday 03:00 after Sunday's slot");
        assert!(active(&s, 0, 7 * H).is_none());
    }

    #[test]
    fn equal_times_mean_all_day_and_disabled_schedules_never_apply() {
        let s = sched(vec![slot(SAT | SUN, 0, 0)]);
        assert!(active(&s, 5, 0).is_some() && active(&s, 6, 23 * H + 59).is_some());
        assert!(active(&s, 2, 12 * H).is_none());
        let mut off = s.clone();
        off.enabled = false;
        assert!(active(&off, 5, 12 * H).is_none());
    }

    #[test]
    fn the_first_matching_slot_wins() {
        let mut a = slot(EVERY_DAY, 0, 0);
        a.down_kib = 1;
        let mut b = slot(EVERY_DAY, 0, 0);
        b.down_kib = 2;
        assert_eq!(active(&sched(vec![a, b]), 3, 600).unwrap().down_kib, 1);
    }
}
