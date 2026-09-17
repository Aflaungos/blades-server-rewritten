//! Randomised global-shop bundles — the products retail re-rolled per purchase.
//!
//! THE BUG (#170, "store chests still give the same loot"). A purchase takes its
//! reward from `global_shop_grants`, a capture-derived recording of ONE retail
//! purchase per product, cloned on every buy. For a fixed product that is exactly
//! right. For a randomised bundle it means every player, every time, receives the
//! same gold and the same two items.
//!
//! Retail rolled. Of the 85 products bought more than once in the captures, six
//! varied — and four of those varied on EVERY purchase: 4,697 buys of the biggest
//! bundle produced 4,697 distinct rewards. Those same four are ABSENT from the
//! APK offer catalogue, which is why there was no authored content to grant and
//! the recording was all there was.
//!
//! THE PAYOUT SCALES WITH THE BUYER'S LEVEL, and that is the part worth getting
//! right. On the biggest bundle the median gold runs 9,735 at levels 1-5 and
//! 60,315 at 54-89, monotonically, r = 0.83; the two mid-size bundles score 0.93
//! and 0.90. Drawing pooled would hand a level-3 player a level-60 payout. This
//! is the same trap the enemy-loot corpus was built to avoid, and it was caught
//! here only because the bands came out non-monotonic on the first attempt — an
//! attribution bug in the miner, not a property of the data.
//!
//! The two remaining varying products are `needs_roll` arcane jewelry whose
//! TEMPLATE is fixed and whose enchant roll varies. They are handled by the
//! authored-contents path, not here.

use serde::Deserialize;
use uuid::Uuid;

use crate::economy::{RewardGrant, RewardItem};
use crate::user_data::Item;

static STORE_BUNDLE_LOOT_RAW: &str = include_str!("../store_bundle_loot.json");

#[derive(Deserialize)]
struct Corpus {
    products: Vec<Product>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Product {
    product_id: Uuid,
    observations: u64,
    by_level: Vec<Band>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Band {
    min_buyer_level: u64,
    max_buyer_level: u64,
    results: Vec<Drawn>,
}

#[derive(Deserialize)]
struct Drawn {
    reward: ObservedReward,
    n: u64,
}

/// One observed payout. `Item` deserializes straight from retail's own shape;
/// the corpus strips `items[].id` so a fresh one is minted per purchase.
#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct ObservedReward {
    #[serde(default)]
    currencies: std::collections::HashMap<Uuid, u64>,
    #[serde(default)]
    stackable_items: std::collections::HashMap<Uuid, u64>,
    #[serde(default)]
    items: Vec<Item>,
    #[serde(default)]
    town_xp: u64,
}

fn corpus() -> &'static Corpus {
    static TABLE: std::sync::OnceLock<Corpus> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(STORE_BUNDLE_LOOT_RAW)
            .unwrap_or_else(|_| Corpus { products: Vec::new() })
    })
}

/// A uuid4-shaped instance id derived from the draw. `blades_lib` does not carry
/// uuid's `v4` feature, and deriving it is better anyway: the same purchase
/// described twice keeps the same id, while a different purchase gets a
/// different one because the nonce is part of the seed.
fn instance_uuid(seed: u64, ordinal: usize) -> Uuid {
    let hi = mix(seed ^ (ordinal as u64).rotate_left(17));
    let lo = mix(hi ^ 0x5DEE_CE66_D1B2_4E35);
    let mut b = (((hi as u128) << 64) | lo as u128).to_be_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Uuid::from_bytes(b)
}

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Whether this product is one the server must roll rather than replay.
pub fn is_randomised_bundle(product_id: &Uuid) -> bool {
    corpus().products.iter().any(|p| &p.product_id == product_id)
}

/// How many retail purchases back this product, for tests and diagnostics.
pub fn bundle_observations(product_id: &Uuid) -> u64 {
    corpus()
        .products
        .iter()
        .find(|p| &p.product_id == product_id)
        .map(|p| p.observations)
        .unwrap_or(0)
}

/// Roll one purchase of a randomised bundle for a buyer at `buyer_level`.
///
/// `nonce` must differ per purchase — buying the same bundle twice has to be able
/// to give different things, which is the entire complaint. The caller passes
/// something that moves, such as the player's purchase count.
///
/// `None` when the product is not a randomised bundle, which leaves every other
/// product on the existing recorded-grant path.
pub fn roll_bundle(product_id: &Uuid, buyer_level: u64, nonce: u64) -> Option<RewardGrant> {
    let product = corpus()
        .products
        .iter()
        .find(|p| &p.product_id == product_id)?;

    // The band containing the buyer's level, else the nearest one — a level above
    // or below everything retail was observed at clamps rather than falling back
    // to a pooled draw, which is the thing that would misprice the payout.
    let band = product
        .by_level
        .iter()
        .find(|b| buyer_level >= b.min_buyer_level && buyer_level <= b.max_buyer_level)
        .or_else(|| {
            product.by_level.iter().min_by_key(|b| {
                if buyer_level < b.min_buyer_level {
                    b.min_buyer_level - buyer_level
                } else {
                    buyer_level.saturating_sub(b.max_buyer_level)
                }
            })
        })?;

    let total: u64 = band.results.iter().map(|r| r.n).sum();
    if total == 0 {
        return None;
    }
    let seed = mix(product_id.as_u128() as u64 ^ mix(nonce) ^ buyer_level.rotate_left(13));
    let mut pick = seed % total;
    let drawn = band
        .results
        .iter()
        .find(|r| {
            if pick < r.n {
                true
            } else {
                pick -= r.n;
                false
            }
        })
        .unwrap_or(band.results.last()?);

    let mut grant = RewardGrant {
        currencies: drawn.reward.currencies.clone(),
        stackable_items: drawn.reward.stackable_items.clone(),
        town_xp: drawn.reward.town_xp,
        ..RewardGrant::default()
    };
    for (ordinal, item) in drawn.reward.items.iter().enumerate() {
        grant.items.push(RewardItem {
            // A fresh instance per purchase. The frozen ids in the capture-derived
            // grants are what made buying twice overwrite the first item.
            id: instance_uuid(seed, ordinal),
            item: item.clone(),
        });
    }
    Some(grant)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIG: &str = "11102495-fde7-4e77-b6c4-d13b9303f1f5";
    const GOLD: &str = "f8d27767-a85e-4fd6-a5bb-bf8a13d0daa2";

    fn big() -> Uuid {
        BIG.parse().unwrap()
    }

    fn gold_of(g: &RewardGrant) -> u64 {
        *g.currencies.get(&GOLD.parse::<Uuid>().unwrap()).unwrap_or(&0)
    }

    /// The corpus must be compiled in and hold what was mined. Everything below
    /// is vacuous without it.
    #[test]
    fn the_bundle_corpus_loads() {
        // Parsed explicitly rather than through the OnceLock, so a deserialization
        // failure names itself instead of silently degrading to an empty corpus —
        // which is how a stripped property-definition id went unnoticed once.
        if let Err(e) = serde_json::from_str::<Corpus>(STORE_BUNDLE_LOOT_RAW) {
            panic!("the corpus failed to parse: {e}");
        }
        assert_eq!(corpus().products.len(), 4, "four randomised bundles were mined");
        assert_eq!(bundle_observations(&big()), 4697, "the big bundle corpus");
        assert!(is_randomised_bundle(&big()));
        assert!(
            !is_randomised_bundle(&"6ec8f67f-2cef-41aa-a7fc-f46237ae809c".parse().unwrap()),
            "a FIXED product must stay on the recorded-grant path"
        );
    }

    /// THE BUG: every purchase returned the same thing.
    #[test]
    fn buying_twice_can_give_different_things() {
        let mut seen = std::collections::HashSet::new();
        for nonce in 0..80u64 {
            let g = roll_bundle(&big(), 30, nonce).expect("the big bundle must roll");
            seen.insert(gold_of(&g));
        }
        assert!(
            seen.len() > 10,
            "80 purchases produced only {} distinct gold amounts — still replaying",
            seen.len()
        );
    }

    /// THE LEVEL CONTROL. Median gold runs 9,735 at levels 1-5 and 60,315 at
    /// 54-89. A pooled draw would pay a level-3 player a level-60 payout, which
    /// is indistinguishable from working code without this test.
    #[test]
    fn the_payout_scales_with_the_buyers_level() {
        let median_at = |level: u64| -> u64 {
            let mut v: Vec<u64> = (0..120u64)
                .map(|n| gold_of(&roll_bundle(&big(), level, n).unwrap()))
                .collect();
            v.sort_unstable();
            v[v.len() / 2]
        };
        let low = median_at(3);
        let high = median_at(70);
        assert!(low > 0, "a low-level buyer got no gold at all");
        assert!(
            low < 20_000,
            "a level-3 buyer received a median {low} gold; retail paid about 9,700 there, \
             so the level bands are being ignored"
        );
        assert!(high > 45_000, "a level-70 buyer received only {high}; retail paid about 60,000");
        assert!(high > low * 3, "the payout barely moved with level: {low} -> {high}");
    }

    /// Bands must be monotonic in level. A non-monotonic band was what exposed
    /// the attribution bug in the miner, so it is worth pinning.
    #[test]
    fn median_gold_never_falls_as_level_rises() {
        let median_at = |level: u64| -> u64 {
            let mut v: Vec<u64> = (0..80u64)
                .map(|n| gold_of(&roll_bundle(&big(), level, n).unwrap()))
                .collect();
            v.sort_unstable();
            v[v.len() / 2]
        };
        let levels = [2u64, 7, 12, 16, 20, 27, 32, 45, 60, 85];
        let mut previous = 0;
        for level in levels {
            let m = median_at(level);
            assert!(
                m + m / 3 >= previous,
                "median gold fell sharply from {previous} to {m} going into level {level}"
            );
            previous = m;
        }
    }

    /// A level far outside anything retail was observed at must clamp to the
    /// nearest band, not fall back to a pooled draw.
    #[test]
    fn an_out_of_range_level_clamps_to_the_nearest_band() {
        // Compared as DISTRIBUTIONS, not single draws: the buyer level is part of
        // the draw seed, so level 0 and level 1 pick different rewards out of the
        // same band. Only the band is being asserted here.
        let median_at = |level: u64| -> u64 {
            let mut v: Vec<u64> = (0..120u64)
                .map(|n| gold_of(&roll_bundle(&big(), level, n).unwrap()))
                .collect();
            v.sort_unstable();
            v[v.len() / 2]
        };
        assert!(roll_bundle(&big(), 0, 5).is_some(), "level 0 must still roll");
        assert!(roll_bundle(&big(), 500, 5).is_some(), "level 500 must still roll");

        let below = median_at(0);
        let lowest = median_at(1);
        let above = median_at(500);
        let highest = median_at(89);
        assert!(
            below.abs_diff(lowest) * 4 < lowest,
            "level 0 drew {below} against the lowest band's {lowest} — it did not clamp"
        );
        assert!(
            above.abs_diff(highest) * 4 < highest,
            "level 500 drew {above} against the highest band's {highest} — it did not clamp"
        );
        assert!(below < above, "clamping must still respect the ends of the ladder");
    }

    /// Gear must come out as real items with a fresh instance id — the frozen
    /// capture ids are what made buying twice overwrite the first item.
    #[test]
    fn gear_gets_a_fresh_instance_id_every_purchase() {
        let mut ids = std::collections::HashSet::new();
        let mut items = 0;
        for nonce in 0..40u64 {
            for item in &roll_bundle(&big(), 30, nonce).unwrap().items {
                items += 1;
                assert!(!item.item.item_template_id.is_nil());
                ids.insert(item.id);
            }
        }
        assert!(items > 20, "only {items} items over 40 purchases");
        assert_eq!(
            ids.len(),
            items,
            "instance ids repeated across purchases — buying twice would overwrite"
        );
        assert!(
            ids.iter().all(|id| id.get_version_num() == 4),
            "instance ids must look like the uuid4 retail minted"
        );
    }

    /// Only the mined bundles roll; everything else stays on the recorded path.
    #[test]
    fn an_unknown_product_does_not_roll() {
        assert!(roll_bundle(&Uuid::from_u128(0xDEAD), 30, 1).is_none());
    }
}
