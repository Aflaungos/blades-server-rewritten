use std::sync::Arc;

use crate::{
    json_db::JsonDbWrapper,
    models::{CharacterDbEntryCharacterWalletInventory, QuestDbEntryDungeonStateAndGeneratedData},
};
use actix_web::{
    http::StatusCode,
    post,
    web::{self, Json},
};
use blades_lib::economy::RewardGrant;
use blades_lib::user_data::{
    B64EncodedData, CompleteCharacterWithIdWithoutData, CompleteInventoryUpdate, DungeonStatus,
    EnemyIndex, EnemyStatus, InventoryChangeTracker,
};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl, scoped_futures::ScopedFutureExt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{BladeApiError, ServerGlobal, session::SessionLookedUpMaybe};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct EnemyKilledUpdate {
    pub spawn_group_id: Uuid,
    pub spawner_index: usize,
    pub enemy_index: usize,
    #[allow(unused)]
    // We use the data stored in the generated data instead of trusting the client
    pub xp_reward: f64,
    pub time: u64,
}

/// A `combat_completed` action — the client posts it (alongside `enemy_killed`
/// actions) when a combat encounter/room resolves. The per-enemy XP + kills arrive as
/// the `EnemyKilled` actions in the SAME batch, so this is a state-only marker here
/// (the dungeon's `current_state` blob is persisted regardless). Fields vary by client
/// version; serde ignores any we don't name, so an evolving payload never 400s.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct CombatCompletedUpdate {
    #[serde(default)]
    #[allow(dead_code)]
    time: Option<u64>,
}

/// A `*_loot_collected` action: the player picked something up inside the dungeon.
///
/// The client reports WHAT it collected — the captured payload carries the contents
/// inline, e.g.
///
/// ```json
/// {"type":"item_loot_collected","spawnGroupId":"e7edb276-…","spawnGroupIndex":0,
///  "loot":{"stackableItems":{"e7193116-…":1}},"time":1777808410209}
/// ```
///
/// so `loot` is deserialized straight into [`RewardGrant`], whose camelCase wire form
/// is already exactly `{currencies, stackableItems, items}`. Retail trusted the client
/// here — there is no second request where the server states the contents — and the
/// spawn-group loot tables are not something we generate, so trusting it is also the
/// only option that credits anything at all.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LootCollectedUpdate {
    #[serde(default)]
    loot: RewardGrant,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DungeonUpdateAction {
    EnemyKilled(EnemyKilledUpdate),
    /// Accepted so a mixed `enemy_killed` + `combat_completed` batch deserializes —
    /// previously an unknown variant made serde reject the whole POST (→400), which is
    /// PaganBlueNose's "network error … with a quest".
    CombatCompleted(CombatCompletedUpdate),
    /// Loot off a corpse.
    EnemyLootCollected(LootCollectedUpdate),
    /// Loot off the dungeon floor — loose items and harvested plants. This is the one
    /// tracker #95 is about.
    ItemLootCollected(LootCollectedUpdate),
    /// Forward-compat: any OTHER action type the client emits is accepted and ignored
    /// rather than 400-ing the whole batch.
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DungeonUpdateRequest {
    current_state: B64EncodedData,
    actions: Vec<DungeonUpdateAction>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DungeonUpdateResponse {
    inventory: CompleteInventoryUpdate,
    character: CompleteCharacterWithIdWithoutData,
    dungeon_status: DungeonStatus,
}

#[post(
    "blades.bgs.services/api/game/v1/public/characters/{character_id}/quests/{quest_id}/dungeons/current/update"
)]
pub async fn dungeon_update(
    path: web::Path<(Uuid, Uuid)>,
    session: SessionLookedUpMaybe,
    app_state: web::Data<Arc<ServerGlobal>>,
    body: Json<DungeonUpdateRequest>,
) -> Result<Json<DungeonUpdateResponse>, BladeApiError> {
    let session = session.get_session_or_error()?;
    let (character_id, quest_id) = path.into_inner();
    let mut conn = app_state.db_pool.get().await.unwrap();

    conn.transaction(|mut conn| {
        async move {
            let (quest_data, mut character_data) = {
                use crate::schema::characters;
                use crate::schema::quests;

                quests::table
                    .filter(quests::id.eq(quest_id))
                    .filter(characters::id.eq(character_id))
                    .inner_join(characters::table)
                    .filter(characters::user_id.eq(session.session.user_id))
                    .select((
                        QuestDbEntryDungeonStateAndGeneratedData::as_select(),
                        CharacterDbEntryCharacterWalletInventory::as_select(),
                    ))
                    .for_no_key_update()
                    .load(&mut conn)
                    .await?
                    .into_iter()
                    .next()
                    // No matching quest/character for this user → 404 instead of a panic
                    // (dropped connection = the client's "network error").
                    .ok_or_else(|| BladeApiError::new(StatusCode::NOT_FOUND, 20000, 2))?
            };

            // The dungeon must have been entered/generated first. A missing
            // generated_data / dungeon_state is a client/state error → 400, not a panic.
            let generated_data = quest_data
                .generated_data
                .0
                .ok_or_else(|| BladeApiError::new(StatusCode::BAD_REQUEST, 20001, 2))?;
            let mut dungeon_state = quest_data
                .dungeon_state
                .ok_or_else(|| BladeApiError::new(StatusCode::BAD_REQUEST, 20001, 2))?
                .0;

            let mut inventory_modification_tracker = InventoryChangeTracker::default();

            dungeon_state.dungeon_status.current_state = body.current_state.clone();

            for action in &body.actions {
                match action {
                    DungeonUpdateAction::EnemyKilled(enemy_killed) => {
                        let enemy_index = EnemyIndex::new(
                            enemy_killed.spawn_group_id,
                            enemy_killed.spawner_index,
                            enemy_killed.enemy_index,
                        );
                        // A stale/unknown enemy index → skip THAT action, don't kill the
                        // whole dungeon update (was a panic).
                        let Some(enemy_generated_data) = generated_data.get_enemy(&enemy_index)
                        else {
                            log::warn!(
                                "dungeon_update: enemy {:?} not in generated data (stale) — skipping",
                                enemy_index
                            );
                            continue;
                        };
                        if let Some(current_enemy_data) = dungeon_state
                            .dungeon_status
                            .enemy_status
                            .get_mut(&enemy_index)
                        {
                            // Re-reporting an already-killed enemy (client retry/dup) is a
                            // no-op, not a panic — and must not double-count XP.
                            if current_enemy_data.killed {
                                continue;
                            }
                            current_enemy_data.killed = true;
                        } else {
                            dungeon_state.dungeon_status.enemy_status.insert(
                                enemy_index,
                                EnemyStatus {
                                    spawn_group_id: enemy_killed.spawn_group_id,
                                    xp_reward: enemy_generated_data.given_xp,
                                    killed: true,
                                    time: enemy_killed.time,
                                    loot: enemy_generated_data.merged_loot_table(),
                                },
                            );
                        }

                        character_data.character.0.experience += enemy_generated_data.given_xp;
                    }
                    // Room/combat finished — the kills (XP) arrived as EnemyKilled actions
                    // in this batch and the dungeon current_state blob is persisted below;
                    // no extra reward to apply here.
                    DungeonUpdateAction::CombatCompleted(_) => {}
                    // Floor loot, harvested plants, corpse loot. Before this these fell
                    // into `Unknown` and were logged and dropped, so picking anything up
                    // in a dungeon gave the player nothing (tracker #95).
                    DungeonUpdateAction::ItemLootCollected(collected)
                    | DungeonUpdateAction::EnemyLootCollected(collected) => {
                        blades_lib::economy::apply_reward(
                            &collected.loot,
                            &mut character_data.wallet.0,
                            &mut character_data.inventory.0,
                            &mut character_data.character.0,
                            &mut inventory_modification_tracker,
                        );
                    }
                    DungeonUpdateAction::Unknown => {
                        log::warn!(
                            "dungeon_update: ignoring unknown action type in quest {}",
                            quest_id
                        );
                    }
                }
            }

            // generate the response before we submit data to minimize the amount of cloning needed

            let result = DungeonUpdateResponse {
                dungeon_status: dungeon_state.dungeon_status.clone(),
                character: CompleteCharacterWithIdWithoutData {
                    id: character_id,
                    character: character_data.character.0.clone(),
                },
                inventory: character_data.inventory.0.generate_client_update(&inventory_modification_tracker)
            };

            let quest_data_rebuilt = QuestDbEntryDungeonStateAndGeneratedData {
                id: quest_id,
                dungeon_state: Some(JsonDbWrapper(dungeon_state)),
                generated_data: JsonDbWrapper(Some(generated_data)),
            };

            {
                use crate::schema::quests;
                diesel::update(quests::table)
                    // BOTH halves of the primary key. `quests.id` alone is NOT unique:
                    // an ordinary story quest is stored under the template id, so every
                    // character on that quest has a row with the same `id`, and an
                    // update filtered on `id` writes one player's dungeon state into
                    // all of them. The SELECT above is already scoped to this
                    // character; the write has to be too.
                    .filter(quests::id.eq(quest_id))
                    .filter(quests::character_id.eq(character_id))
                    .set(quest_data_rebuilt)
                    .execute(&mut conn)
                    .await?;
            }

            {
                use crate::schema::characters;

                diesel::update(characters::table)
                    .filter(characters::id.eq(character_data.id))
                    .set(character_data)
                    .execute(&mut conn)
                    .await?;
            }

            Ok::<_, BladeApiError>(Json(result))
        }
    }.scope_boxed()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_batch_with_combat_completed_deserializes() {
        // The real client posts a MIXED actions array (enemy_killed + combat_completed).
        // With the old single-variant enum, serde rejected `combat_completed` and the
        // WHOLE POST 400'd (PaganBlueNose's quest "network error"). It must now parse,
        // and an unknown future action type must be tolerated too.
        let raw = r#"{
            "currentState": {"b64": "AAAA"},
            "actions": [
                {"type":"enemy_killed","spawnGroupId":"11111111-0000-0000-0000-000000000001","spawnerIndex":0,"enemyIndex":0,"xpReward":11.0,"time":1234},
                {"type":"combat_completed","time":1300,"someFutureField":42},
                {"type":"room_cleared","whatever":true}
            ]
        }"#;
        let req: DungeonUpdateRequest =
            serde_json::from_str(raw).expect("mixed dungeon-update batch must deserialize");
        assert_eq!(req.actions.len(), 3);
        assert!(matches!(req.actions[0], DungeonUpdateAction::EnemyKilled(_)));
        assert!(matches!(req.actions[1], DungeonUpdateAction::CombatCompleted(_)));
        // Unknown action type tolerated (not a 400).
        assert!(matches!(req.actions[2], DungeonUpdateAction::Unknown));
    }

    /// Floor loot and harvested plants must parse as their own action and carry their
    /// contents — not fall into `Unknown`, which is what silently dropped them
    /// (tracker #95: "items placed on the dungeon floor or plants can't be picked up,
    /// they don't give anything to the player").
    ///
    /// The bodies here are copied from captured retail requests.
    #[test]
    fn floor_and_corpse_loot_parse_with_their_contents() {
        let raw = r#"{
            "currentState": {"b64": "AAAA"},
            "actions": [
                {"type":"item_loot_collected","spawnGroupId":"e7edb276-a04c-413f-80ab-69ffe304874f","spawnGroupIndex":0,
                 "loot":{"stackableItems":{"e7193116-d761-479b-8a20-5633737977f5":1}},"time":1777808410209},
                {"type":"enemy_loot_collected","spawnGroupId":"4295c814-e5e7-4a8a-939a-d3238471c906","spawnerIndex":0,"enemyIndex":0,
                 "loot":{"currencies":{"f8d27767-a85e-4fd6-a5bb-bf8a13d0daa2":4}},"time":1777808407519}
            ]
        }"#;
        let req: DungeonUpdateRequest =
            serde_json::from_str(raw).expect("captured loot batch must deserialize");

        let lumber: Uuid = "e7193116-d761-479b-8a20-5633737977f5".parse().unwrap();
        let gold: Uuid = "f8d27767-a85e-4fd6-a5bb-bf8a13d0daa2".parse().unwrap();

        match &req.actions[0] {
            DungeonUpdateAction::ItemLootCollected(c) => {
                assert_eq!(c.loot.stackable_items.get(&lumber), Some(&1));
            }
            other => panic!("floor loot must not be dropped, got {other:?}"),
        }
        match &req.actions[1] {
            DungeonUpdateAction::EnemyLootCollected(c) => {
                assert_eq!(c.loot.currencies.get(&gold), Some(&4));
            }
            other => panic!("corpse loot must not be dropped, got {other:?}"),
        }
    }

    /// A loot action with no `loot` block at all must still parse — the client omits it
    /// for an empty pickup, and a hard `loot` field would 400 the whole batch, which is
    /// the same class of bug as the old single-variant enum.
    #[test]
    fn a_loot_action_without_contents_still_parses() {
        let raw = r#"{
            "currentState": {"b64": "AAAA"},
            "actions": [{"type":"item_loot_collected","spawnGroupId":"e7edb276-a04c-413f-80ab-69ffe304874f","time":1}]
        }"#;
        let req: DungeonUpdateRequest = serde_json::from_str(raw).expect("must deserialize");
        match &req.actions[0] {
            DungeonUpdateAction::ItemLootCollected(c) => assert!(c.loot.is_empty()),
            other => panic!("expected ItemLootCollected(_), got {other:?}"),
        }
    }

    /// Parsing the action is only half of it — the loot has to land in the player's
    /// inventory and wallet. This drives the same `apply_reward` call the handler makes,
    /// so it fails if the credit is dropped rather than only if the parse is.
    #[test]
    fn collected_loot_is_credited_to_the_player() {
        use blades_lib::user_data::{
            Backpack, CompleteCharacter, CompleteInventory, CompleteWallet, Loadout, Treasury,
        };

        let raw = r#"{
            "currentState": {"b64": "AAAA"},
            "actions": [
                {"type":"item_loot_collected","spawnGroupId":"e7edb276-a04c-413f-80ab-69ffe304874f","spawnGroupIndex":0,
                 "loot":{"stackableItems":{"e7193116-d761-479b-8a20-5633737977f5":1}},"time":1},
                {"type":"enemy_loot_collected","spawnGroupId":"4295c814-e5e7-4a8a-939a-d3238471c906","spawnerIndex":0,"enemyIndex":0,
                 "loot":{"currencies":{"f8d27767-a85e-4fd6-a5bb-bf8a13d0daa2":4}},"time":2}
            ]
        }"#;
        let req: DungeonUpdateRequest = serde_json::from_str(raw).unwrap();

        let lumber: Uuid = "e7193116-d761-479b-8a20-5633737977f5".parse().unwrap();
        let gold: Uuid = "f8d27767-a85e-4fd6-a5bb-bf8a13d0daa2".parse().unwrap();

        let mut wallet = CompleteWallet::default();
        let mut inventory = CompleteInventory {
            backpack: Backpack::default(),
            loadout: Loadout::default(),
            treasury: Treasury::default(),
            overflow_treasury: Treasury::default(),
            backpack_version: 1,
            treasury_version: 0,
        };
        let mut character = CompleteCharacter::default();
        let mut tracker = InventoryChangeTracker::default();

        for action in &req.actions {
            if let DungeonUpdateAction::ItemLootCollected(c)
            | DungeonUpdateAction::EnemyLootCollected(c) = action
            {
                blades_lib::economy::apply_reward(
                    &c.loot,
                    &mut wallet,
                    &mut inventory,
                    &mut character,
                    &mut tracker,
                );
            }
        }

        assert_eq!(
            inventory.backpack.stackable_items.count(lumber),
            1,
            "floor loot must reach the backpack"
        );
        assert_eq!(wallet.balance(gold), 4, "corpse gold must reach the wallet");
        assert!(
            tracker.modified_backpack.stackable_items.contains(&lumber),
            "the pickup must be reported to the client, or the bag looks unchanged"
        );
    }
}
