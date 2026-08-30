//! Gem cost of skipping a running timer ("speed up" / `speedUp: true`).
//!
//! Blades lets the player pay GEMS to finish a town building's construction or a
//! crafting job instantly. Retail priced that from ONE global table — a Unity
//! `ScriptableObject` of type `SkipTimeCostTable`, pointed at by BOTH
//! `BuildingConstructionDataList._skipTimeData` and `RecipeData._skipTimeData` (the
//! same asset), so town and crafting share this module rather than each guessing.
//!
//! The asset ships two bands:
//!
//! ```jsonc
//! "_rateList": [
//!   { "_currency": "470c8f58-…", "_costPerHour": 12, "_maxHour":   12.0 },  // Gem
//!   { "_currency": "470c8f58-…", "_costPerHour":  6, "_maxHour": 1000.0 }
//! ]
//! ```
//!
//! # The algorithm is TAX-BRACKET, with a ceil PER BAND
//!
//! `TimeCostTable.GetCostForTime(double)` was read out of the il2cpp disassembly
//! (RVA 0x1C7B1DC) rather than inferred, because the two plausible readings —
//! tax-bracket (charge each band for the hours that fall inside it) vs tier-select
//! (find the band the total falls in and charge every hour at that rate) — differ by
//! up to 2x. It walks the bands, charges the hours inside each one at that band's
//! rate, and **rounds up separately in every band**:
//!
//! ```text
//! H = (f32) seconds / 3600f
//! cursor = 0; rem = H
//! for band in rateList:
//!     hours  = min(rem, band.maxHour - cursor)
//!     cost  += ceil(hours * band.costPerHour)      // per-band ceil, not one at the end
//!     cursor = band.maxHour
//!     rem   -= band.maxHour
//!     if rem <= 0: break
//! ```
//!
//! `BuildingSiteGroupSegment.GetConstructionSkipCosts()` tail-calls this with the
//! timer's raw remaining time: no clamp, no minimum, no per-building multiplier. Any
//! positive remaining time therefore costs at least 1 gem (band 1's ceil).
//!
//! # This is deliberately f32, and must stay f32
//!
//! Unity computes the whole walk in 32-bit floats, and the CLIENT shows the player
//! the number it computes. A tidier `f64` (or the closed form `ceil(12h₁) +
//! ceil(6h₂)` evaluated in double) disagrees at band boundaries — 76 800 s is **201**
//! gems in f32 and 200 in f64, because `76800f/3600f - 12f` lands at 9.333334 and
//! `9.333334 * 6` at 56.000004. Charging 200 while the button says 201 is a visible
//! display-vs-charge mismatch, so the arithmetic below is f32 ON PURPOSE. Do not
//! "simplify" it; [`tests::f32_arithmetic_is_load_bearing_at_a_band_boundary`] fails
//! if you do.
//!
//! # Provenance
//!
//! Two independent sources agree:
//!
//! * the APK asset above (`SkipTimeCostTable`), and
//! * 159 captured retail `speedUp: true` completions, where gems are debited at
//!   `/complete` and never at `/upgrade`. 145/146 usable pairs reproduce exactly,
//!   and the uncontaminated subset is 15/15, over 1 s → 460 795 s (1 → 840 gems).
//!   Rival rules score worse over the same 155 pairs (flat `ceil(r/300)` 135, flat
//!   `ceil(r/600)` 3, `round` instead of `ceil` 133), so both the two-band structure
//!   and the per-band ceil are pinned, not assumed. The join is crossed by the data:
//!   38 394 s → 128 gems (last band-1 point), 47 994 s → 152 (first band-2 point; a
//!   flat 12/hr would have said 160).
//!
//! # What this module does NOT price
//!
//! `gemsPayment: true` on `/upgrade` — "pay gems to cover the gold/materials I am
//! short of" — is a DIFFERENT and still-unsolved formula that depends on the
//! resource shortfall at click time. Nothing here covers it.

use serde_json::Value;
use uuid::Uuid;

use crate::economy::Price;

/// One rate band: every hour from the previous band's ceiling up to `max_hour` costs
/// `cost_per_hour` of `currency`.
#[derive(Debug, Clone, PartialEq)]
pub struct SkipTimeBand {
    pub currency: Uuid,
    pub cost_per_hour: f32,
    /// Cumulative ceiling in hours (NOT the band's width) — band 2's `12.0 → 1000.0`
    /// span is `max_hour` minus the previous band's `max_hour`.
    pub max_hour: f32,
}

/// The global skip-time price curve (the `SkipTimeCostTable` asset).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkipTimeCostTable {
    pub rate_list: Vec<SkipTimeBand>,
}

impl SkipTimeCostTable {
    /// Parse the table out of a loaded static JSON document, from
    /// `_meta.skipTimeCostTable.rateList` (it lives in `building_upgrades.json`
    /// because the curve is GLOBAL — one table for every building and every recipe —
    /// and that file is already the town static loaded at startup).
    ///
    /// Returns `None` when the key is absent OR when any band is malformed. Failing
    /// closed matters: `deploy/static/` is a bind mount that a merge does not ship,
    /// so between merging this and running `deploy/arena.sh static` the server runs
    /// with no table. `None` must mean "charge nothing" at the call site — giving a
    /// speed-up away for a few minutes is recoverable; inventing a price from half a
    /// table and taking a player's gems is not.
    pub fn from_static(root: &Value) -> Option<Self> {
        let rows = root
            .get("_meta")?
            .get("skipTimeCostTable")?
            .get("rateList")?
            .as_array()?;
        if rows.is_empty() {
            return None;
        }
        let mut rate_list = Vec::with_capacity(rows.len());
        for row in rows {
            // Every field is required. A band we cannot read is not a band we can
            // safely skip — dropping it would silently reprice the whole curve.
            let currency = row
                .get("currency")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())?;
            let cost_per_hour = row.get("costPerHour").and_then(Value::as_f64)? as f32;
            let max_hour = row.get("maxHour").and_then(Value::as_f64)? as f32;
            if !cost_per_hour.is_finite() || !max_hour.is_finite() || cost_per_hour < 0.0 {
                return None;
            }
            rate_list.push(SkipTimeBand {
                currency,
                cost_per_hour,
                max_hour,
            });
        }
        Some(SkipTimeCostTable { rate_list })
    }

    /// The price of skipping `seconds` of remaining time, as payable [`Price`] lines
    /// (in band order, one line per distinct currency).
    ///
    /// Non-positive (or NaN) input costs nothing: an elapsed timer is free, and a
    /// faithful walk over a negative remainder would produce a negative cost — i.e.
    /// pay the player to press the button. Retail's `GetRemainingTime()` never goes
    /// negative, so this clamp is ours, and it is the safe direction.
    ///
    /// The arithmetic is f32 to match the Unity client exactly — see the module doc.
    pub fn cost_for_time(&self, seconds: f32) -> Vec<Price> {
        let mut out: Vec<Price> = Vec::new();
        // `!(x > 0.0)` rather than `x <= 0.0` so NaN is also treated as free.
        if !(seconds > 0.0) {
            return out;
        }

        // f32 throughout, deliberately: the 3600.0 divisor, the per-band width, the
        // multiply and the ceil all happen in 32-bit floats in the client.
        let mut rem: f32 = seconds / 3600.0f32;
        let mut cursor: f32 = 0.0f32;

        for band in &self.rate_list {
            let hours = rem.min(band.max_hour - cursor);
            if hours > 0.0 {
                let quantity = (hours * band.cost_per_hour).ceil() as u64;
                if quantity > 0 {
                    match out.iter_mut().find(|p| p.currency_id == band.currency) {
                        Some(line) => line.quantity += quantity,
                        None => out.push(Price::new(band.currency, quantity)),
                    }
                }
            }
            cursor = band.max_hour;
            rem -= band.max_hour;
            if rem <= 0.0 {
                break;
            }
        }
        out
    }

    /// Convenience for the handlers: price a remaining time expressed in
    /// milliseconds (how both `constructionEnd` and a craft job's `completedAt` are
    /// stored). Already-elapsed timers (negative) cost nothing.
    pub fn cost_for_remaining_ms(&self, remaining_ms: i64) -> Vec<Price> {
        if remaining_ms <= 0 {
            return Vec::new();
        }
        self.cost_for_time(remaining_ms as f32 / 1000.0f32)
    }
}

/// Charge a speed-up against `wallet`, atomically.
///
/// `table` is `None` when the static data has not been pushed to the box yet — that
/// charges nothing rather than failing or inventing a price (see
/// [`SkipTimeCostTable::from_static`]).
///
/// Returns the lines actually charged, so a caller can log/assert them. On
/// insufficient funds NOTHING is debited (via [`CompleteWallet::try_pay`]) and the
/// caller must surface an error — a speed-up the player cannot afford has to FAIL,
/// not silently complete for free.
pub fn charge_skip_time(
    table: Option<&SkipTimeCostTable>,
    remaining_ms: i64,
    wallet: &mut crate::user_data::CompleteWallet,
) -> Result<Vec<Price>, crate::economy::EconomyError> {
    let prices = match table {
        Some(t) => t.cost_for_remaining_ms(remaining_ms),
        None => Vec::new(),
    };
    if prices.is_empty() {
        return Ok(prices);
    }
    wallet.try_pay(&prices)?;
    Ok(prices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::{EconomyError, GEMS};
    use crate::user_data::CompleteWallet;

    /// The shipped table, as it appears in `building_upgrades.json`.
    fn shipped_table_json() -> Value {
        serde_json::json!({
            "_meta": {
                "skipTimeCostTable": {
                    "rateList": [
                        { "currency": "470c8f58-a8dd-4c07-8c92-843b785e1139",
                          "costPerHour": 12, "maxHour": 12.0 },
                        { "currency": "470c8f58-a8dd-4c07-8c92-843b785e1139",
                          "costPerHour": 6, "maxHour": 1000.0 }
                    ]
                }
            }
        })
    }

    fn table() -> SkipTimeCostTable {
        SkipTimeCostTable::from_static(&shipped_table_json()).expect("shipped table parses")
    }

    fn gems(seconds: f32) -> u64 {
        let p = table().cost_for_time(seconds);
        assert!(
            p.iter().all(|l| l.currency_id == GEMS),
            "the skip-time table prices in gems only, got {p:?}"
        );
        p.iter().map(|l| l.quantity).sum()
    }

    // ── The curve, against numbers measured in retail captures ────────────────

    /// The two points that straddle the 12 h join, taken from the captures. These
    /// are the whole reason the band walk exists: a flat 12/hr would price 47 994 s
    /// at 160, and a flat 6/hr would price 38 394 s at 64.
    #[test]
    fn band_join_matches_the_captured_prices() {
        assert_eq!(gems(38_394.0), 128, "last captured band-1 point");
        assert_eq!(gems(47_994.0), 152, "first captured band-2 point");
    }

    /// Sub-hour: band 1 only, and the per-band ceil means any positive time costs at
    /// least one gem (1 s → 0.00333 gems → 1).
    #[test]
    fn sub_hour_times_round_up_to_at_least_one_gem() {
        assert_eq!(gems(1.0), 1, "1 s is the cheapest observed skip");
        assert_eq!(gems(1800.0), 6, "half an hour = ceil(0.5 * 12)");
        assert_eq!(gems(3600.0), 12, "exactly one hour");
    }

    /// The top of the captured range (460 795 s → 840 gems): 12 h at 12/hr = 144,
    /// plus the remaining ~115.99 h at 6/hr = 696.
    #[test]
    fn the_longest_captured_timer_prices_at_840_gems() {
        assert_eq!(gems(460_795.0), 840);
    }

    /// An elapsed timer is free — and a NEGATIVE remainder must not pay the player.
    #[test]
    fn elapsed_or_negative_time_is_free() {
        assert_eq!(gems(0.0), 0);
        assert!(table().cost_for_time(-1.0).is_empty(), "negative is free");
        assert!(
            table().cost_for_time(-100_000.0).is_empty(),
            "a long-elapsed timer must not CREDIT gems"
        );
        assert!(table().cost_for_remaining_ms(0).is_empty());
        assert!(table().cost_for_remaining_ms(-5_000).is_empty());
    }

    /// **Do not turn this into f64.** The client computes the walk in 32-bit floats
    /// and shows the player the result; 76 800 s is 201 there and 200 in f64,
    /// because `76800f/3600f` is a hair above 64/3. Charging 200 against a button
    /// that says 201 is exactly the display-vs-charge mismatch we are avoiding.
    #[test]
    fn f32_arithmetic_is_load_bearing_at_a_band_boundary() {
        assert_eq!(gems(76_800.0), 201, "f32 walk; an f64 walk yields 200");

        // Show the disagreement explicitly, so the intent survives a refactor.
        let f64_walk = {
            let h = 76_800.0f64 / 3600.0f64;
            (12.0f64 * 12.0).ceil() as u64 + ((h - 12.0) * 6.0).ceil() as u64
        };
        assert_eq!(f64_walk, 200, "the f64 reading really does differ");
    }

    /// Milliseconds in, the same curve out.
    #[test]
    fn ms_helper_agrees_with_the_seconds_walk() {
        assert_eq!(
            table().cost_for_remaining_ms(47_994_000),
            table().cost_for_time(47_994.0)
        );
    }

    // ── Parsing / fail-closed ─────────────────────────────────────────────────

    #[test]
    fn table_parses_to_the_two_apk_bands() {
        let t = table();
        assert_eq!(t.rate_list.len(), 2);
        assert_eq!(t.rate_list[0].currency, GEMS, "band 1 prices in Gem");
        assert_eq!(t.rate_list[0].cost_per_hour, 12.0);
        assert_eq!(t.rate_list[0].max_hour, 12.0);
        assert_eq!(t.rate_list[1].cost_per_hour, 6.0);
        assert_eq!(t.rate_list[1].max_hour, 1000.0);
    }

    /// A missing table is `None`, not a panic and not a default curve — the window
    /// between merging the code and rsyncing `deploy/static/` must be safe.
    #[test]
    fn a_missing_or_broken_table_is_none() {
        assert!(SkipTimeCostTable::from_static(&serde_json::json!({})).is_none());
        assert!(
            SkipTimeCostTable::from_static(&serde_json::json!({
                "_meta": { "skipTimeCostTable": { "rateList": [] } }
            }))
            .is_none(),
            "an empty rateList prices nothing — treat it as absent"
        );
        assert!(
            SkipTimeCostTable::from_static(&serde_json::json!({
                "_meta": { "skipTimeCostTable": { "rateList": [
                    { "currency": "470c8f58-a8dd-4c07-8c92-843b785e1139", "maxHour": 12.0 }
                ] } }
            }))
            .is_none(),
            "a band missing costPerHour must fail closed, not price at 0"
        );
    }

    // ── Charging ──────────────────────────────────────────────────────────────

    #[test]
    fn charging_debits_exactly_the_curve_price() {
        let t = table();
        let mut w = CompleteWallet::default();
        w.credit(GEMS, 1_000);
        let charged = charge_skip_time(Some(&t), 47_994_000, &mut w).unwrap();
        assert_eq!(charged, vec![Price::new(GEMS, 152)]);
        assert_eq!(w.balance(GEMS), 848);
    }

    /// Insufficient gems must FAIL, and must leave the wallet untouched — the
    /// alternative (skip the charge, complete anyway) hands out free speed-ups.
    #[test]
    fn insufficient_gems_errors_and_leaves_the_balance_unchanged() {
        let t = table();
        let mut w = CompleteWallet::default();
        w.credit(GEMS, 10);
        let err = charge_skip_time(Some(&t), 47_994_000, &mut w).unwrap_err();
        assert_eq!(
            err,
            EconomyError::InsufficientFunds {
                currency: GEMS,
                needed: 152,
                have: 10
            }
        );
        assert_eq!(w.balance(GEMS), 10, "a failed charge must not debit");
    }

    /// No table (static data not pushed yet) → free, not an error and not a guess.
    #[test]
    fn without_a_table_nothing_is_charged() {
        let mut w = CompleteWallet::default();
        w.credit(GEMS, 10);
        assert!(
            charge_skip_time(None, 47_994_000, &mut w)
                .unwrap()
                .is_empty()
        );
        assert_eq!(w.balance(GEMS), 10);
    }

    /// An already-elapsed timer is free even with a table and a full wallet.
    #[test]
    fn an_elapsed_timer_charges_nothing() {
        let t = table();
        let mut w = CompleteWallet::default();
        w.credit(GEMS, 500);
        assert!(charge_skip_time(Some(&t), 0, &mut w).unwrap().is_empty());
        assert!(
            charge_skip_time(Some(&t), -60_000, &mut w)
                .unwrap()
                .is_empty()
        );
        assert_eq!(w.balance(GEMS), 500);
    }
}
