//! Daily login reward — `POST /towns/current/rewards/current` (status) and
//! `POST /towns/current/rewards/current/collect`.
//!
//! A reward rotates each 24h period; the player may collect it once per period. The
//! rotation pool is capture-derived (7 distinct rewards — some grant stackables, some
//! grant a treasury chest); the per-character last-collected period lives in
//! `server_state.daily_reward`.
//!
//! Captured status:
//! ```jsonc
//! { "dailyRewardStatus": { "rewardUid": "eefb9db4-…", "until": 1777784455168,
//!     "dailyReward": { "stackableItems": { "790a188b-…": 2 } }, "collected": false } }
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Length of a daily-reward period (24h).
pub const DAILY_PERIOD_SECS: i64 = 86_400;

/// Where retail put the daily boundary: **05:00 UTC**, not midnight.
///
/// Measured, not assumed. Over 617 retail captures of
/// `…/towns/current/rewards/current` spanning 2026-05-02 → 2026-06-30, every
/// hour offset from −24 to +24 was scanned and −5h is the *unique* offset at
/// which no `rewardUid` spans two weekdays. That is midnight US Eastern on a
/// fixed UTC−5, not DST-adjusted. See
/// `blades-capture/docs/blades-game-data-reference.md` §4.
///
/// It matters twice over: it decides which reward is offered today, and it
/// decides the `until` the client counts down to. A midnight-UTC boundary put
/// both five hours out.
pub const DAILY_RESET_OFFSET_SECS: i64 = 5 * 3600;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChestDef {
    pub tier: u64,
    pub level: u64,
}

/// A daily reward's payload: either stackables or a chest (matches the captured
/// `dailyReward` object, which carries one or the other).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DailyRewardPayload {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub stackable_items: HashMap<Uuid, u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chests: Vec<ChestDef>,
}

/// One entry of the daily-reward rotation (capture-derived).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DailyRewardDef {
    pub reward_uid: Uuid,
    pub daily_reward: DailyRewardPayload,
    /// Which weekday this reward belongs to: 0 = Sunday … 6 = Saturday.
    ///
    /// Optional so an older `daily_rewards.json` still loads. Absent, the pool
    /// falls back to position-in-list rotation, which is what this code did
    /// before and which lands the right rewards on the wrong days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekday: Option<u8>,
}

/// Per-character daily-reward state (persisted in `server_state`).
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DailyRewardState {
    /// The last 24h period the reward was collected in (`None` = never).
    pub collected_period: Option<i64>,
}

/// The 24h period index for a unix timestamp, counted from 05:00 UTC.
pub fn current_period(now_secs: i64) -> i64 {
    (now_secs - DAILY_RESET_OFFSET_SECS).div_euclid(DAILY_PERIOD_SECS)
}

/// When the current period ends (next reset), in unix ms (the wire uses ms).
pub fn until_ms(period: i64) -> i64 {
    ((period + 1) * DAILY_PERIOD_SECS + DAILY_RESET_OFFSET_SECS) * 1000
}

/// Weekday of a period, 0 = Sunday … 6 = Saturday.
///
/// Unix day 0 (1970-01-01) was a Thursday, so period 0 — the day beginning
/// 1970-01-01 05:00 UTC — is a Thursday, index 4.
pub fn weekday_of_period(period: i64) -> u8 {
    (period + 4).rem_euclid(7) as u8
}

/// The reward offered in a given period.
///
/// Weekday-keyed when the pool says which weekday each entry belongs to, which
/// is what retail did: exactly seven `rewardUid`s, one per weekday, stable over
/// two months of captures. Tuesday is always Clay; Friday is always a Revive
/// Scroll. Rotating by position instead gets the right seven rewards in the
/// right cycle length and puts every one of them on the wrong day.
///
/// Falls back to position rotation when no entry carries a weekday, so a pool
/// written before this field existed still works.
pub fn reward_for_period(defs: &[DailyRewardDef], period: i64) -> Option<&DailyRewardDef> {
    if defs.is_empty() {
        return None;
    }
    let wanted = weekday_of_period(period);
    if let Some(def) = defs.iter().find(|d| d.weekday == Some(wanted)) {
        return Some(def);
    }
    // No entry claims today. If ANY entry is weekday-keyed the pool is meant to
    // be weekday-keyed and this is a gap in the data, not a reason to serve an
    // unrelated day's reward — but serving nothing would stall the client on a
    // screen it cannot dismiss, so fall through to the rotation either way.
    Some(&defs[period.rem_euclid(defs.len() as i64) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> Vec<DailyRewardDef> {
        vec![
            DailyRewardDef {
                reward_uid: Uuid::from_u128(1),
                daily_reward: DailyRewardPayload {
                    stackable_items: HashMap::from([(Uuid::from_u128(10), 2)]),
                    chests: vec![],
                },
                weekday: None,
            },
            DailyRewardDef {
                reward_uid: Uuid::from_u128(2),
                daily_reward: DailyRewardPayload {
                    stackable_items: HashMap::default(),
                    chests: vec![ChestDef { tier: 3, level: 1 }],
                },
                weekday: None,
            },
        ]
    }

    /// A weekday-keyed pool, one entry per day, uid = weekday index + 100.
    fn weekday_defs() -> Vec<DailyRewardDef> {
        (0u8..7)
            .map(|w| DailyRewardDef {
                reward_uid: Uuid::from_u128(100 + w as u128),
                daily_reward: DailyRewardPayload::default(),
                weekday: Some(w),
            })
            .collect()
    }

    /// A known Tuesday: 2026-05-05 12:00 UTC.
    const TUESDAY_NOON: i64 = 1_777_982_400;

    #[test]
    fn period_advances_daily_and_rotates() {
        let d0 = current_period(0);
        let d1 = current_period(DAILY_PERIOD_SECS);
        assert_eq!(d1, d0 + 1);
        // With no weekday keys the pool still rotates by position. Asserted as
        // alternation rather than as a fixed starting index: which entry lands
        // on which absolute day is an artifact of the epoch and the reset
        // offset, and pinning it would just re-break on the next offset change.
        let a = reward_for_period(&defs(), d0).unwrap().reward_uid;
        let b = reward_for_period(&defs(), d1).unwrap().reward_uid;
        assert_ne!(a, b);
        assert_eq!(reward_for_period(&defs(), d0 + 2).unwrap().reward_uid, a);
    }

    /// The boundary is 05:00 UTC, so 04:59 belongs to the previous reward day.
    /// This is the assertion that fails if someone "simplifies" the offset away.
    #[test]
    fn the_day_turns_over_at_05_00_utc_not_midnight() {
        let midnight = TUESDAY_NOON - 12 * 3600; // 2026-05-05 00:00 UTC
        let previous_day = current_period(midnight - 12 * 3600); // 2026-05-04 12:00
        assert_eq!(
            current_period(midnight + 4 * 3600 + 3599),
            previous_day,
            "04:59:59 still belongs to the reward day that opened yesterday at 05:00",
        );
        assert_eq!(
            current_period(midnight + 5 * 3600),
            previous_day + 1,
            "05:00:00 starts the new one",
        );
    }

    /// Tuesday is Clay, every time. Retail's table is weekday-keyed and this is
    /// the property a position-rotating pool cannot hold.
    #[test]
    fn a_weekday_keyed_pool_serves_the_same_reward_every_tuesday() {
        let pool = weekday_defs();
        let tuesday = Uuid::from_u128(100 + 2);
        for week in 0..8 {
            let p = current_period(TUESDAY_NOON + week * 7 * DAILY_PERIOD_SECS);
            assert_eq!(weekday_of_period(p), 2, "week {week} is a Tuesday");
            assert_eq!(reward_for_period(&pool, p).unwrap().reward_uid, tuesday);
        }
    }

    #[test]
    fn every_weekday_gets_its_own_reward_across_one_week() {
        let pool = weekday_defs();
        let seen: Vec<Uuid> = (0..7)
            .map(|d| {
                let p = current_period(TUESDAY_NOON + d * DAILY_PERIOD_SECS);
                reward_for_period(&pool, p).unwrap().reward_uid
            })
            .collect();
        let unique: std::collections::HashSet<_> = seen.iter().collect();
        assert_eq!(unique.len(), 7, "seven days, seven distinct rewards: {seen:?}");
    }

    /// A pool with a hole must still answer, or the client sits on a screen it
    /// cannot dismiss.
    #[test]
    fn a_missing_weekday_falls_back_rather_than_serving_nothing() {
        let pool: Vec<_> = weekday_defs().into_iter().filter(|d| d.weekday != Some(2)).collect();
        assert!(reward_for_period(&pool, current_period(TUESDAY_NOON)).is_some());
    }

    #[test]
    fn until_is_the_next_05_00_in_ms() {
        let p = current_period(TUESDAY_NOON);
        let until_secs = until_ms(p) / 1000;
        assert!(until_secs > TUESDAY_NOON);
        assert!(until_secs - TUESDAY_NOON <= DAILY_PERIOD_SECS);
        assert_eq!(
            until_secs.rem_euclid(DAILY_PERIOD_SECS),
            DAILY_RESET_OFFSET_SECS,
            "the countdown must end at 05:00 UTC",
        );
    }

    #[test]
    fn payload_serializes_one_branch_only() {
        let stack = serde_json::to_value(&defs()[0].daily_reward).unwrap();
        assert!(stack.get("stackableItems").is_some());
        assert!(stack.get("chests").is_none(), "empty chests omitted");
        let chest = serde_json::to_value(&defs()[1].daily_reward).unwrap();
        assert!(chest.get("chests").is_some());
        assert!(chest.get("stackableItems").is_none());
    }

    #[test]
    fn empty_pool_has_no_reward() {
        assert!(reward_for_period(&[], 5).is_none());
    }
}
