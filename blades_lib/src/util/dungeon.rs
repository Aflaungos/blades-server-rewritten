//! Building `DungeonGeneratedData` for a dungeon.
//!
//! ## Why this is shared
//!
//! The client is told which dungeon to load and, separately, what is inside it.
//! Those two must describe the SAME dungeon: the generated data is keyed by the
//! dungeon's own spawn-group / chest / item ids, and the client looks up each id
//! as it populates the level.
//!
//! Hand it ids from a different dungeon and nothing resolves — no enemies
//! spawn, so no `enemy_killed` action is ever sent, so the run cannot progress.
//! That is exactly what the Abyss did: it served a hard-coded stub whose two
//! spawn groups exist only in the floor-1 dungeon, so floor 1 played and every
//! other floor hung. Six of the seven live runs sat at floor 0 with nothing
//! completed.
//!
//! The quest path had always generated this correctly from the real dungeon.
//! Rather than a second implementation for the Abyss, that logic lives here and
//! both call it.

use std::collections::HashMap;

use serde::Deserialize;
use uuid::Uuid;

use crate::{
    game_data::GameData,
    user_data::{
        ChestGeneratedData, DungeonEnemyResult, DungeonGeneratedData, DungeonItemResult,
        LootTableResult,
    },
};

/// Build the generated data for `dungeon_uuid`, with every enemy at
/// `enemy_level` and worth `given_xp`.
///
/// Returns `None` when the dungeon is not in `parsed.json` — the caller decides
/// whether that is fatal. A malformed item spawn is skipped rather than
/// panicking: a partial `parsed.json` must not take down the request, the item
/// simply does not appear.
// -- interactable loot ------------------------------------------------------
//
// `GameDataInteractable::loot_table` is a `HashMap<Uuid, EmptyStruct>`: parsed.json
// carries the loot-table IDS with no contents. So every breakable and floor pickup
// was generated with `LootTableResult::default()` -- empty -- and the client was
// told the barrel contains nothing (tracker #100, #102).
//
// Retail's own `itemGeneratedData[].lootTableLoot` is keyed by exactly those ids
// and DOES carry contents. The table below is 108,122 observations over 25 loot
// tables, mined from captured responses -- including how often each table rolled
// EMPTY, because retail's breakables frequently give nothing and reproducing the
// hit rate matters as much as reproducing the contents.
static INTERACTABLE_LOOT_RAW: &str = include_str!("../interactable_loot.json");

// The APK carries the exact tier and quantity on every dungeon chest spawn,
// but the old parsed.json extractor discarded both fields and retained only
// the spawn id. This sidecar was extracted from the same 137 dungeon assets:
// all 292 ids in parsed.json are present, including seven groups with more
// than one chest. Keep it compiled into the binary like interactable loot so
// a missing production bind-mount cannot silently restore the tier-1 stub.
static CHEST_TIERS_RAW: &str = include_str!("../chest_tiers.json");

#[derive(Deserialize)]
struct ChestTierCorpus {
    chests: HashMap<Uuid, ChestSpawnDefinition>,
}

#[derive(Deserialize)]
struct ChestSpawnDefinition {
    tier: Option<u64>,
    quantity: u64,
}

fn chest_tiers() -> &'static ChestTierCorpus {
    static TABLE: std::sync::OnceLock<ChestTierCorpus> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(CHEST_TIERS_RAW).unwrap_or_else(|_| ChestTierCorpus {
            chests: HashMap::new(),
        })
    })
}

fn interactable_loot() -> &'static serde_json::Value {
    static TABLE: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(INTERACTABLE_LOOT_RAW).unwrap_or_else(|e| {
            // blades_lib has no logger; a malformed table degrades to empty, which is
            // exactly the behaviour that preceded this change.
            let _ = e;
            serde_json::json!({ "tables": {} })
        })
    })
}

/// Deterministic per (dungeon, spawn, table) so a re-fetch of the same dungeon
/// yields the same contents -- the client is told once what a barrel holds and
/// must still find it there when it breaks it.
fn loot_seed(dungeon_uuid: &Uuid, spawn_id: &Uuid, table_id: &Uuid) -> u64 {
    let mut x = dungeon_uuid.as_u128() as u64
        ^ (spawn_id.as_u128() as u64).rotate_left(21)
        ^ (table_id.as_u128() as u64).rotate_left(42);
    // splitmix64 finaliser
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// One roll of a loot table, weighted by how often retail produced each outcome.
///
/// A whole RESULT is drawn, not a single item. Retail's results routinely carry
/// several stacks at once -- across the corpus, 68,000 results held one item,
/// 9,659 held two, 5,096 held three and 1,379 held four -- so drawing item by
/// item could never reproduce a breakable that "spews five things" (#104). It
/// also keeps which items appeared TOGETHER, instead of inventing a
/// distribution over combinations.
///
/// The empty result is one of the drawn outcomes (68,081 of them), so a barrel
/// that gives nothing stays as common as retail made it.
fn roll_loot_table(dungeon_uuid: &Uuid, spawn_id: &Uuid, table_id: &Uuid) -> LootTableResult {
    let mut out = LootTableResult::default();
    let Some(results) = interactable_loot()
        .get("tables")
        .and_then(|t| t.get(table_id.to_string()))
        .and_then(|e| e.get("results"))
        .and_then(|r| r.as_array())
    else {
        // A table we never observed stays empty, exactly as before.
        return out;
    };

    let total: u64 = results
        .iter()
        .filter_map(|r| r.get("n").and_then(|v| v.as_u64()))
        .sum();
    if total == 0 {
        return out;
    }

    let mut pick = loot_seed(dungeon_uuid, spawn_id, table_id) % total;
    for r in results {
        let n = r.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
        if pick >= n {
            pick -= n;
            continue;
        }
        let loot = r.get("loot");
        if let Some(items) = loot.and_then(|l| l.get("stackableItems")).and_then(|v| v.as_object()) {
            for (id, qty) in items {
                if let (Ok(uuid), Some(q)) = (Uuid::parse_str(id), qty.as_u64()) {
                    out.stackable_items.insert(uuid, q);
                }
            }
        }
        if let Some(curs) = loot.and_then(|l| l.get("currencies")).and_then(|v| v.as_object()) {
            for (id, amt) in curs {
                if let (Ok(uuid), Some(a)) = (Uuid::parse_str(id), amt.as_u64()) {
                    out.currencies.insert(uuid, a);
                }
            }
        }
        return out;
    }
    out
}

/// Which dungeon owns this enemy spawn group, if exactly one does.
///
/// THE VARIANT MISMATCH (#174). Retail builds several versions of a dungeon —
/// `EQ22_SQ102_DungeonSettings_A`, `_B`, `_C` — and 23 families in `parsed.json`
/// have them. A quest names exactly one, always the `_A`, and every one of those
/// 23 families gives its variants **completely different** enemy spawn groups: not
/// one group is shared between any two variants.
///
/// The client does not always walk the one we named. When it walks `_B`, every
/// kill it reports names a spawner we have no data for, so `dungeon_update` logs
/// "not in generated data (stale)" and throws the kill away — no experience, no
/// loot, for that whole stage. 15 of the 22 such warnings in eleven days are
/// exactly this.
///
/// Because the variants share no groups, the reported spawner identifies the
/// variant unambiguously, which is what makes the repair in `dungeon_update` safe:
/// the client tells us which version it is in, and we can generate the right data
/// instead of discarding its progress.
///
/// `None` when no dungeon owns the group, or when more than one does — in which
/// case the answer is not unambiguous and the caller must not guess.
pub fn dungeon_owning_spawn_group(game_data: &GameData, group_id: &Uuid) -> Option<Uuid> {
    let mut found = None;
    for (dungeon_id, dungeon) in &game_data.dungeons {
        if dungeon.spawn_info.enemy_spawn_groups.contains_key(group_id) {
            if found.is_some() {
                return None; // ambiguous
            }
            found = Some(*dungeon_id);
        }
    }
    found
}

pub fn generate_for_dungeon(
    game_data: &GameData,
    dungeon_uuid: &Uuid,
    enemy_level: i64,
    given_xp: u64,
) -> Option<DungeonGeneratedData> {
    let dungeon = game_data.dungeons.get(dungeon_uuid)?;

    Some(DungeonGeneratedData {
        enemy_generated_data: dungeon
            .spawn_info
            .enemy_spawn_groups
            .iter()
            .map(|(spawn_group_id, spawn_group)| {
                let mut enemies_info = Vec::new();
                for _ in 0..spawn_group.quantity.max(1) {
                    enemies_info.push(vec![DungeonEnemyResult {
                        enemy_level,
                        given_xp,
                        spawn_group_loot: HashMap::default(),
                        loot_table_loot: HashMap::default(),
                    }]);
                }
                (*spawn_group_id, enemies_info)
            })
            .collect(),
        chest_generated_data: dungeon
            .spawn_info
            .chest
            .iter()
            .map(|(chest_spawn_id, _)| {
                let definition = chest_tiers().chests.get(chest_spawn_id);
                // One APK row has the unset -1 rarity. Tier 1 is the existing safe
                // fallback for it and for a future dungeon absent from this corpus.
                let tier = definition.and_then(|d| d.tier).unwrap_or(1);
                let quantity = definition.map(|d| d.quantity).unwrap_or(1).max(1);
                (
                    *chest_spawn_id,
                    (0..quantity).map(|_| ChestGeneratedData { tier }).collect(),
                )
            })
            .collect(),
        item_generated_data: dungeon
            .spawn_info
            .item
            .iter()
            .filter_map(|(item_spawn_id, spawn_info)| {
                let picked = spawn_info.apparition_settings.first()?;
                let interactable = game_data.interactables.get(&picked.interactable_uuid)?;
                Some((
                    *item_spawn_id,
                    vec![DungeonItemResult {
                        loot_table_loot: interactable
                            .loot_table
                            .iter()
                            .map(|(k, _)| {
                                (*k, roll_loot_table(dungeon_uuid, item_spawn_id, k))
                            })
                            .collect(),
                    }],
                ))
            })
            .collect(),
        algorithm_version: 1,
        version: 0,
    })
}

#[cfg(test)]
mod chest_generation_tests {
    use super::*;

    #[test]
    fn apk_chest_corpus_is_complete_and_not_the_old_stub() {
        let corpus = chest_tiers();
        assert_eq!(corpus.chests.len(), 292, "the APK has 292 chest spawn groups");
        assert_eq!(
            corpus.chests.values().filter(|c| c.tier == Some(1)).count(),
            136
        );
        assert_eq!(
            corpus.chests.values().filter(|c| c.tier == Some(2)).count(),
            107
        );
        assert_eq!(
            corpus.chests.values().filter(|c| c.tier == Some(3)).count(),
            48
        );
        assert_eq!(corpus.chests.values().filter(|c| c.tier.is_none()).count(), 1);
        assert_eq!(
            corpus.chests.values().filter(|c| c.quantity > 1).count(),
            7,
            "multi-chest groups must not collapse back to one"
        );
    }

    #[test]
    fn every_parsed_chest_uses_its_apk_tier_and_quantity() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../deploy/static/parsed.json");
        let raw = std::fs::read_to_string(path).expect("read parsed.json");
        let game_data: GameData = serde_json::from_str(&raw).expect("parse game data");
        let mut checked = 0;

        for (dungeon_id, dungeon) in &game_data.dungeons {
            let Some(generated) = generate_for_dungeon(&game_data, dungeon_id, 1, 1) else {
                panic!("known dungeon did not generate");
            };
            for chest_id in dungeon.spawn_info.chest.keys() {
                let expected = chest_tiers()
                    .chests
                    .get(chest_id)
                    .unwrap_or_else(|| panic!("parsed chest {chest_id} missing from APK sidecar"));
                let actual = generated
                    .chest_generated_data
                    .get(chest_id)
                    .expect("generated chest group");
                assert_eq!(actual.len(), expected.quantity.max(1) as usize, "{chest_id}");
                let expected_tier = expected.tier.unwrap_or(1);
                assert!(actual.iter().all(|c| c.tier == expected_tier), "{chest_id}");
                checked += 1;
            }
        }

        assert_eq!(checked, 292, "the test must cover the complete APK corpus");
    }
}

#[cfg(test)]
mod interactable_loot_tests {
    use super::*;

    fn table() -> &'static serde_json::Value {
        interactable_loot()
    }

    /// The corpus must actually be there. Everything below is vacuous without it.
    #[test]
    fn the_loot_table_corpus_loads() {
        let tables = table()["tables"].as_object().expect("tables object");
        assert!(tables.len() >= 20, "expected the mined tables, got {}", tables.len());
        let observations: u64 = tables
            .values()
            .flat_map(|t| t["results"].as_array().cloned().unwrap_or_default())
            .filter_map(|r| r["n"].as_u64())
            .sum();
        assert!(observations > 100_000, "only {observations} observations");
    }

    /// A breakable must be able to produce something. This is the bug: every loot
    /// table generated empty, so barrels and plants held nothing at all.
    #[test]
    fn some_rolls_produce_loot() {
        let tables = table()["tables"].as_object().unwrap();
        let dungeon = Uuid::from_u128(0xD0);
        let mut produced = 0;
        let mut checked = 0;

        for tid in tables.keys() {
            let table_id: Uuid = tid.parse().unwrap();
            // several spawns, because one spawn may legitimately roll empty
            for s in 0..40u128 {
                let got = roll_loot_table(&dungeon, &Uuid::from_u128(s), &table_id);
                checked += 1;
                if !got.stackable_items.is_empty() {
                    produced += 1;
                }
            }
        }
        assert!(checked > 0);
        assert!(
            produced > 0,
            "not one of {checked} rolls produced loot — breakables are still empty"
        );
    }

    /// Empty is a real outcome, not a failure. Retail's tables rolled empty tens
    /// of thousands of times; a build where every barrel pays is as wrong as one
    /// where none do.
    #[test]
    fn empty_remains_possible() {
        let tables = table()["tables"].as_object().unwrap();
        let dungeon = Uuid::from_u128(0xD1);
        let mut empty = 0;
        let mut total = 0;
        for tid in tables.keys() {
            let table_id: Uuid = tid.parse().unwrap();
            for s in 0..40u128 {
                let got = roll_loot_table(&dungeon, &Uuid::from_u128(s), &table_id);
                total += 1;
                if got.stackable_items.is_empty() && got.currencies.is_empty() {
                    empty += 1;
                }
            }
        }
        assert!(empty > 0, "every one of {total} rolls paid out; retail's did not");
        assert!(empty < total, "no roll paid out at all");
    }

    /// The same barrel must hold the same thing every time it is described.
    ///
    /// The client is told the dungeon's contents when it loads and must find them
    /// unchanged when it breaks the barrel; a re-roll would desync the two.
    #[test]
    fn a_roll_is_stable_for_the_same_barrel() {
        let tables = table()["tables"].as_object().unwrap();
        let tid: Uuid = tables.keys().next().unwrap().parse().unwrap();
        let d = Uuid::from_u128(7);
        let s = Uuid::from_u128(9);
        let a = roll_loot_table(&d, &s, &tid);
        let b = roll_loot_table(&d, &s, &tid);
        assert_eq!(a.stackable_items, b.stackable_items, "same barrel, different loot");

        // control: a DIFFERENT spawn should not be forced to match, or the
        // stability check above would hold trivially for everything.
        let mut differs = false;
        for other in 0..60u128 {
            let c = roll_loot_table(&d, &Uuid::from_u128(other), &tid);
            if c.stackable_items != a.stackable_items {
                differs = true;
                break;
            }
        }
        assert!(differs, "every spawn rolls identically — the seed is not varying");
    }

    /// Every item id we hand out must be one retail actually placed in that table.
    #[test]
    fn we_only_ever_emit_items_retail_used() {
        let tables = table()["tables"].as_object().unwrap();
        let dungeon = Uuid::from_u128(0xD2);
        let mut checked = 0;
        for (tid, entry) in tables {
            let table_id: Uuid = tid.parse().unwrap();
            let allowed: std::collections::HashSet<String> = entry["results"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .flat_map(|r| {
                    r["loot"]["stackableItems"]
                        .as_object()
                        .map(|o| o.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default()
                })
                .collect();
            for s in 0..25u128 {
                let got = roll_loot_table(&dungeon, &Uuid::from_u128(s), &table_id);
                for item in got.stackable_items.keys() {
                    assert!(
                        allowed.contains(&item.to_string()),
                        "table {tid} produced {item}, which retail never put in it"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "no items emitted — the test proved nothing");
    }

    /// A breakable must be able to spew SEVERAL stacks at once.
    ///
    /// My first pass mined (itemId, quantity) pairs independently and drew one,
    /// so a barrel could never yield more than a single stack — reported as
    /// "breakables are only spewing 1 item but they usually spew 5" (#104).
    /// Retail's results carry 2, 3 and 4 items together (9,659 / 5,096 / 1,379
    /// observations), so a multi-item roll has to be reachable.
    #[test]
    fn a_roll_can_yield_several_items_at_once() {
        let tables = table()["tables"].as_object().unwrap();
        let dungeon = Uuid::from_u128(0xD3);
        let mut multi = 0;
        let mut single = 0;
        let mut biggest = 0;

        for tid in tables.keys() {
            let table_id: Uuid = tid.parse().unwrap();
            for s in 0..200u128 {
                let got = roll_loot_table(&dungeon, &Uuid::from_u128(s), &table_id);
                let n = got.stackable_items.len();
                biggest = biggest.max(n);
                if n > 1 {
                    multi += 1;
                } else if n == 1 {
                    single += 1;
                }
            }
        }
        assert!(
            multi > 0,
            "not one roll produced more than a single stack (biggest was {biggest}) — \
             breakables can still only spew one thing"
        );
        // Control: single-item results must still dominate, or we have swung too
        // far and made every barrel a jackpot.
        assert!(single > multi, "multi-item rolls ({multi}) outnumber single ({single})");
    }
}

#[cfg(test)]
mod variant_owner_tests {
    use super::*;

    /// A DUNGEON VARIANT IS IDENTIFIABLE FROM ONE SPAWN GROUP.
    ///
    /// That is what makes the `dungeon_update` repair safe. Retail builds several
    /// versions of a dungeon (`_A`, `_B`, `_C`); 23 families in `parsed.json` have
    /// them, and in all 23 the variants share NOT ONE enemy spawn group — so a
    /// reported spawner names its variant unambiguously.
    ///
    /// Asserted against the shipped data rather than a fixture, because the whole
    /// claim is about that data.
    #[test]
    fn variants_never_share_an_enemy_spawn_group() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../deploy/static/parsed.json");
        let raw = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p:?}: {e}"));
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let dungeons = parsed["dungeons"].as_object().expect("dungeons");

        // Group by handle stem: EQ22_SQ102_DungeonSettings_A -> EQ22_SQ102_DungeonSettings
        let mut families: std::collections::HashMap<String, Vec<&str>> = Default::default();
        for (id, d) in dungeons {
            let h = d["handle"].as_str().unwrap_or("");
            if let Some(stem) = h.strip_suffix("_A")
                .or_else(|| h.strip_suffix("_B"))
                .or_else(|| h.strip_suffix("_C"))
                .or_else(|| h.strip_suffix("_D"))
            {
                families.entry(stem.to_string()).or_default().push(id);
            }
        }
        let multi: Vec<_> = families.values().filter(|v| v.len() > 1).collect();
        assert!(!multi.is_empty(), "no variant families found — the premise is gone");

        let groups_of = |id: &str| -> std::collections::HashSet<String> {
            dungeons[id]["spawn_info"]["enemy_spawn_groups"]
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default()
        };
        for fam in &multi {
            for (i, a) in fam.iter().enumerate() {
                for b in fam.iter().skip(i + 1) {
                    let overlap: Vec<_> = groups_of(a).intersection(&groups_of(b)).cloned().collect();
                    assert!(
                        overlap.is_empty(),
                        "variants {a} and {b} share spawn group(s) {overlap:?} — a reported \
                         spawner would no longer identify one variant"
                    );
                }
            }
        }
    }

    /// THE CONTROL: the lookup must find a real group, and must refuse an unknown
    /// one. A function that returned `Some` for everything would satisfy the repair
    /// path while attributing the player to an arbitrary dungeon.
    #[test]
    fn the_owner_lookup_finds_real_groups_and_refuses_unknown_ones() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../deploy/static/parsed.json");
        let raw = std::fs::read_to_string(&p).unwrap();
        let gd: GameData = serde_json::from_str(&raw).expect("parsed.json loads as GameData");

        // A group we know exists: take one from any dungeon.
        let (want_dungeon, want_group) = gd
            .dungeons
            .iter()
            .find_map(|(id, d)| d.spawn_info.enemy_spawn_groups.keys().next().map(|g| (*id, *g)))
            .expect("some dungeon has an enemy spawn group");
        assert_eq!(
            dungeon_owning_spawn_group(&gd, &want_group),
            Some(want_dungeon),
            "a real spawn group must resolve to its own dungeon"
        );

        // And one that exists nowhere.
        assert_eq!(
            dungeon_owning_spawn_group(&gd, &Uuid::from_u128(0xDEADBEEF)),
            None,
            "an unknown group must not be attributed to any dungeon"
        );
    }
}
