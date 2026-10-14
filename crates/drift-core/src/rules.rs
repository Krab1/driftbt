//! Seeding goals: when a finished torrent may stop. Pure.

use serde::{Deserialize, Serialize};

/// Stop seeding at this share ratio or after this many minutes, whichever comes first. Either may be unset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub ratio: Option<f32>,
    pub minutes: Option<u32>,
}

impl Goal {
    pub fn is_empty(&self) -> bool {
        self.ratio.is_none() && self.minutes.is_none()
    }
}

/// A goal set on one torrent replaces the global default entirely.
pub fn effective(per_torrent: Option<Goal>, default: Goal) -> Goal {
    per_torrent.unwrap_or(default)
}

/// Whether seeding may stop now. Nothing stops before `min_seed_secs` has passed, whatever the ratio
/// (the courtesy minimum many trackers expect). The ratio is uploaded bytes over the torrent size.
pub fn reached(goal: Goal, uploaded: u64, size: u64, seeded_secs: u64, min_seed_secs: u64) -> bool {
    if seeded_secs < min_seed_secs {
        return false;
    }
    let by_ratio = goal.ratio.is_some_and(|r| r > 0.0 && size > 0 && uploaded as f64 / size as f64 >= f64::from(r));
    let by_time = goal.minutes.is_some_and(|m| m > 0 && seeded_secs >= u64::from(m) * 60);
    by_ratio || by_time
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn ratio_goal() {
        let g = Goal { ratio: Some(1.5), minutes: None };
        assert!(!reached(g, GB, GB, 100, 0));
        assert!(reached(g, GB + GB / 2, GB, 100, 0));
    }

    #[test]
    fn time_goal_in_minutes() {
        let g = Goal { ratio: None, minutes: Some(60) };
        assert!(!reached(g, 0, GB, 3599, 0));
        assert!(reached(g, 0, GB, 3600, 0));
    }

    #[test]
    fn either_condition_is_enough() {
        let g = Goal { ratio: Some(2.0), minutes: Some(10) };
        assert!(reached(g, 0, GB, 600, 0), "time reached, ratio not");
        assert!(reached(g, 2 * GB, GB, 1, 0), "ratio reached, time not");
    }

    #[test]
    fn the_minimum_seed_time_always_wins() {
        let g = Goal { ratio: Some(1.0), minutes: None };
        assert!(!reached(g, 5 * GB, GB, 30 * 60, 3600), "ratio 5.0 but only 30 min of a 60 min minimum");
        assert!(reached(g, 5 * GB, GB, 3600, 3600));
    }

    #[test]
    fn empty_zero_and_sizeless_goals_never_fire() {
        assert!(!reached(Goal::default(), 9 * GB, GB, 999_999, 0));
        assert!(!reached(Goal { ratio: Some(0.0), minutes: Some(0) }, 9 * GB, GB, 999_999, 0));
        assert!(!reached(Goal { ratio: Some(1.0), minutes: None }, GB, 0, 10, 0), "zero size must not divide");
    }

    #[test]
    fn a_torrent_goal_replaces_the_default() {
        let default = Goal { ratio: Some(2.0), minutes: None };
        let own = Goal { ratio: None, minutes: Some(5) };
        assert_eq!(effective(Some(own), default), own);
        assert_eq!(effective(None, default), default);
    }
}
