//! Game events (daily / Sigil quests) — `POST /gameevents`.
//!
//! Bethesda advertises a rotating set of timed event quests; completing one pays Sigil
//! (the event currency) via the normal quest flow. The full event library is
//! capture-derived (the union of every event seen); the endpoint surfaces a few as
//! *active now* by stamping a current time window onto them, so 2-3 daily/Sigil quests
//! appear available over the next day or two.
//!
//! Captured event:
//! ```jsonc
//! { "gameEventInstanceId": "b483c668-…::1777780800", "type": "quest",
//!   "startTimeSecs": 1777780800, "endTimeSecs": 1777953600,
//!   "recurrence": { "recurrenceType": "daily", "startTimeSecs": 1663214400,
//!                   "durationSecs": 172800, "recurrenceInterval": 39 },
//!   "questId": "7f0d1508-…", "important": true }
//! ```
//!
//! The recurrence is real, not a slice. Every one of the 39 captured events repeats
//! on a `recurrenceInterval` of 39 days with a `durationSecs` window of 172 800 (2
//! days), so at any instant `39 events x 2/39 days` puts an expected **2** events in
//! their window — and retail's captures show exactly 1 or 2 active (2 in 614
//! responses, 1 in 43). The same arithmetic puts exactly 1 event within a day of
//! opening, and retail's `gameEventQuestsInWarning` array had exactly 1 entry in all
//! 686 responses that carried one. An earlier version of this module ignored
//! `recurrenceInterval` and surfaced a rotating slice of 3 instead.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Default active window if an event template carries no instance duration (2 days,
/// matching the observed `durationSecs`).
const DEFAULT_WINDOW_SECS: i64 = 172_800;
const SECS_PER_DAY: i64 = 86_400;

/// How far ahead of its opening an event is announced in `gameEventQuestsInWarning`.
///
/// MEASURED: over the 686 retail `/quests` responses carrying a warning entry, the
/// lead time `startTimeSecs - now` ran from 0.1 h to exactly 24.0 h and never above.
pub const WARNING_LEAD_SECS: i64 = SECS_PER_DAY;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Recurrence {
    pub recurrence_type: String,
    pub start_time_secs: i64,
    pub duration_secs: i64,
    pub recurrence_interval: i64,
}

/// A capture-derived event template (one quest event, its recurrence + how long an
/// instance stays open).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EventDef {
    pub event_id: Uuid,
    pub quest_id: Uuid,
    pub recurrence: Recurrence,
    #[serde(default)]
    pub important: bool,
    /// How long one active instance lasts (captured `endTimeSecs - startTimeSecs`).
    #[serde(default)]
    pub instance_duration_secs: i64,
}

/// One active event on the wire.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GameEvent {
    pub game_event_instance_id: String,
    pub r#type: String,
    pub start_time_secs: i64,
    pub end_time_secs: i64,
    pub recurrence: Recurrence,
    pub quest_id: Uuid,
    pub important: bool,
}

impl EventDef {
    /// How long one instance stays open.
    pub fn window_secs(&self) -> i64 {
        if self.instance_duration_secs > 0 {
            self.instance_duration_secs
        } else if self.recurrence.duration_secs > 0 {
            self.recurrence.duration_secs
        } else {
            DEFAULT_WINDOW_SECS
        }
    }

    /// The gap between two consecutive instances, in seconds.
    fn period_secs(&self) -> i64 {
        let days = self.recurrence.recurrence_interval;
        if days > 0 { days * SECS_PER_DAY } else { 0 }
    }

    /// Start of the instance that most recently began at or before `now`.
    ///
    /// `None` when the series has not started yet, or when the event does not repeat
    /// (interval 0) and its single window already lies in the future.
    pub fn instance_start_at_or_before(&self, now: i64) -> Option<i64> {
        let anchor = self.recurrence.start_time_secs;
        if now < anchor {
            return None;
        }
        let period = self.period_secs();
        if period <= 0 {
            return Some(anchor);
        }
        Some(anchor + ((now - anchor) / period) * period)
    }

    /// Start of the first instance that begins strictly after `now`.
    pub fn next_instance_start_after(&self, now: i64) -> Option<i64> {
        let anchor = self.recurrence.start_time_secs;
        if now < anchor {
            return Some(anchor);
        }
        let period = self.period_secs();
        if period <= 0 {
            return None; // one-shot event, already begun
        }
        Some(anchor + ((now - anchor) / period + 1) * period)
    }

    /// The instance covering `now`, if the event is open.
    pub fn active_instance_start(&self, now: i64) -> Option<i64> {
        let start = self.instance_start_at_or_before(now)?;
        (now < start + self.window_secs()).then_some(start)
    }

    /// Build the wire object for the instance beginning at `start`.
    pub fn instance(&self, start: i64) -> GameEvent {
        GameEvent {
            game_event_instance_id: format!("{}::{}", self.event_id, start),
            r#type: "quest".to_string(),
            start_time_secs: start,
            end_time_secs: start + self.window_secs(),
            recurrence: self.recurrence.clone(),
            quest_id: self.quest_id,
            important: self.important,
        }
    }
}

/// The events whose instance window covers `now`.
pub fn active_events(library: &[EventDef], now_secs: i64) -> Vec<GameEvent> {
    let mut out: Vec<GameEvent> = library
        .iter()
        .filter_map(|def| Some(def.instance(def.active_instance_start(now_secs)?)))
        .collect();
    out.sort_by_key(|e| (e.start_time_secs, e.game_event_instance_id.clone()));
    out
}

/// The events whose next instance opens within `lead_secs` of `now` — retail's
/// `gameEventQuestsInWarning`, i.e. "starting soon", not "ending soon".
pub fn upcoming_events(library: &[EventDef], now_secs: i64, lead_secs: i64) -> Vec<GameEvent> {
    let mut out: Vec<GameEvent> = library
        .iter()
        .filter_map(|def| {
            let start = def.next_instance_start_after(now_secs)?;
            (start - now_secs <= lead_secs).then(|| def.instance(start))
        })
        .collect();
    out.sort_by_key(|e| (e.start_time_secs, e.game_event_instance_id.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(n: u128) -> EventDef {
        EventDef {
            event_id: Uuid::from_u128(n),
            quest_id: Uuid::from_u128(n + 1000),
            recurrence: Recurrence {
                recurrence_type: "daily".to_string(),
                start_time_secs: 1663214400,
                duration_secs: 172800,
                recurrence_interval: 39,
            },
            important: true,
            instance_duration_secs: 172800,
        }
    }

    const ANCHOR: i64 = 1_663_214_400;
    const PERIOD: i64 = 39 * 86_400;
    const WINDOW: i64 = 172_800;

    #[test]
    fn an_event_is_open_only_inside_its_recurring_window() {
        let d = def(1);
        // Inside the very first window.
        assert_eq!(d.active_instance_start(ANCHOR), Some(ANCHOR));
        assert_eq!(d.active_instance_start(ANCHOR + WINDOW - 1), Some(ANCHOR));
        // One second after it closes, and for the 37 days that follow, it is shut.
        assert_eq!(d.active_instance_start(ANCHOR + WINDOW), None);
        assert_eq!(d.active_instance_start(ANCHOR + PERIOD - 1), None);
        // ...and open again exactly one period later. This is the assertion the old
        // day-slice implementation could not pass: it ignored recurrenceInterval and
        // surfaced three events every single day regardless of their windows.
        assert_eq!(
            d.active_instance_start(ANCHOR + PERIOD),
            Some(ANCHOR + PERIOD),
            "the series repeats on recurrenceInterval days"
        );
        assert!(active_events(&[d.clone()], ANCHOR + WINDOW).is_empty());
        assert_eq!(active_events(&[d], ANCHOR + PERIOD).len(), 1);
    }

    #[test]
    fn an_event_that_has_not_started_yet_is_not_active() {
        let d = def(1);
        assert_eq!(d.active_instance_start(ANCHOR - 1), None);
        assert_eq!(d.next_instance_start_after(ANCHOR - 1), Some(ANCHOR));
    }

    #[test]
    fn the_wire_instance_is_stamped_with_its_own_window() {
        let d = def(1);
        let e = d.instance(ANCHOR + PERIOD);
        assert_eq!(e.game_event_instance_id, format!("{}::{}", d.event_id, ANCHOR + PERIOD));
        assert_eq!(e.start_time_secs, ANCHOR + PERIOD);
        assert_eq!(e.end_time_secs, ANCHOR + PERIOD + WINDOW);
        assert_eq!(e.r#type, "quest");
        assert_eq!(e.quest_id, d.quest_id);
    }

    #[test]
    fn warning_lists_what_opens_within_the_lead_and_nothing_else() {
        let d = def(1);
        let just_inside = ANCHOR - WARNING_LEAD_SECS;
        let just_outside = ANCHOR - WARNING_LEAD_SECS - 1;
        assert_eq!(
            upcoming_events(&[d.clone()], just_inside, WARNING_LEAD_SECS).len(),
            1,
            "an event opening in exactly 24h is announced"
        );
        assert!(
            upcoming_events(&[d.clone()], just_outside, WARNING_LEAD_SECS).is_empty(),
            "one second earlier it is not"
        );
        // An event that is currently OPEN is not also announced — the entry points at
        // its *next* window, which is a period away.
        let up = upcoming_events(&[d], ANCHOR, WARNING_LEAD_SECS);
        assert!(up.is_empty(), "an open event is not in the warning list");
    }

    #[test]
    fn empty_library_yields_nothing() {
        assert!(active_events(&[], 1_777_800_000).is_empty());
        assert!(upcoming_events(&[], 1_777_800_000, WARNING_LEAD_SECS).is_empty());
    }

    /// The committed 39-event library, checked against what retail's captures show.
    ///
    /// With 39 events on a 39-day period and a 2-day window, `39 x 2/39 = 2` events
    /// should be open at any instant and `39 x 1/39 = 1` should be within a day of
    /// opening. Retail's `/quests` captures show 2 active in 614 responses and 1 in
    /// 43, and exactly 1 warning entry in all 686 responses that had one. Sampling a
    /// year of days must land in the same place; the old day-slice implementation
    /// returned a flat 3 active and 0 upcoming, every day.
    #[test]
    fn the_committed_library_opens_about_two_events_at_a_time() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../deploy/static/game_events.json");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let lib: Vec<EventDef> = serde_json::from_str(&raw).expect("valid game_events.json");
        assert_eq!(lib.len(), 39, "the committed library is the full captured set");

        let start = 1_777_800_000i64;
        let mut totals = (0usize, 0usize);
        let mut max_active = 0usize;
        for day in 0..365 {
            let now = start + day * 86_400;
            let a = active_events(&lib, now).len();
            let u = upcoming_events(&lib, now, WARNING_LEAD_SECS).len();
            totals.0 += a;
            totals.1 += u;
            max_active = max_active.max(a);
        }
        let mean_active = totals.0 as f64 / 365.0;
        let mean_upcoming = totals.1 as f64 / 365.0;
        assert!(
            (1.5..=2.5).contains(&mean_active),
            "expected ~2 open events on an average day, got {mean_active}"
        );
        assert!(
            (0.5..=1.5).contains(&mean_upcoming),
            "expected ~1 event within a day of opening, got {mean_upcoming}"
        );
        assert!(max_active <= 4, "never a firehose: max {max_active} open at once");
    }
}
