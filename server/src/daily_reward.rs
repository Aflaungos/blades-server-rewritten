//! Daily login reward — `POST /towns/current/rewards/current` (status) and
//! `.../rewards/current/collect`.
//!
//! A reward rotates each 24h period (pool is capture-derived); the player collects it
//! once per period (tracked in `server_state.daily_reward`). NOTE: `until` must be in
//! the future — a past value makes the client spin re-fetching and stall every other
//! request. `until_ms(period)` is the next period boundary, always ahead. Rotation/
//! period math is the pure [`blades_lib::features::daily_reward`] layer.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use actix_web::{
    http::StatusCode,
    post,
    web::{self, Json},
};
use blades_lib::economy::{RewardChest, RewardGrant, apply_reward, grant_chest};
use blades_lib::features::daily_reward::{self, DailyRewardPayload};
use blades_lib::user_data::{CompleteInventoryUpdate, InventoryChangeTracker};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl, scoped_futures::ScopedFutureExt};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    BladeApiError, ServerGlobal,
    models::{CharacterDbEntryEconomy, CharacterDbEntryServerState},
    session::SessionLookedUpMaybe,
    util::get_only_single_character_and_check_permission,
};

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DailyRewardStatus {
    reward_uid: Uuid,
    until: i64,
    daily_reward: DailyRewardPayload,
    collected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    daily_reward_status: DailyRewardStatus,
}

/// Build the status block for `period`, given whether it has been collected.
fn status_for(
    app_state: &ServerGlobal,
    period: i64,
    collected: bool,
) -> DailyRewardStatus {
    let until = daily_reward::until_ms(period);
    match daily_reward::reward_for_period(&app_state.static_data.daily_rewards, period) {
        Some(def) => DailyRewardStatus {
            reward_uid: def.reward_uid,
            until,
            daily_reward: def.daily_reward.clone(),
            collected,
        },
        // Empty pool: a placeholder with a future `until` so the client doesn't stall.
        None => DailyRewardStatus {
            reward_uid: Uuid::nil(),
            until,
            daily_reward: DailyRewardPayload::default(),
            collected,
        },
    }
}

#[post(
    "/api/game/v1/public/characters/{character_id}/towns/current/rewards/current"
)]
pub async fn get_daily_reward(
    session: SessionLookedUpMaybe,
    app_state: web::Data<Arc<ServerGlobal>>,
    path: web::Path<Uuid>,
) -> Result<Json<StatusResponse>, BladeApiError> {
    let session = session.get_session_or_error()?;
    let character_id = path.into_inner();
    let mut conn = app_state.db_pool.get().await.unwrap();

    let rows = {
        use crate::schema::characters::dsl::*;
        characters
            .filter(id.eq(character_id))
            .select(CharacterDbEntryServerState::as_select())
            .load(&mut conn)
            .await
            .unwrap()
    };
    let entry = get_only_single_character_and_check_permission(rows, &session.session)?;

    let period = daily_reward::current_period(now_secs());
    let collected = entry.server_state.0.daily_reward.collected_period == Some(period);
    Ok(Json(StatusResponse {
        daily_reward_status: status_for(&app_state, period, collected),
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CollectResponse {
    reward: RewardGrant,
    daily_reward_status: DailyRewardStatus,
    inventory: CompleteInventoryUpdate,
}

#[post(
    "/api/game/v1/public/characters/{character_id}/towns/current/rewards/current/collect"
)]
pub async fn collect_daily_reward(
    session: SessionLookedUpMaybe,
    app_state: web::Data<Arc<ServerGlobal>>,
    path: web::Path<Uuid>,
) -> Result<Json<CollectResponse>, BladeApiError> {
    let session = session.get_session_or_error()?;
    let user_id = session.session.user_id;
    let character_id = path.into_inner();
    let globals = app_state.get_ref().clone();
    let mut conn = app_state.db_pool.get().await.unwrap();

    conn.transaction(move |mut conn| {
        async move {
            let mut entry = {
                use crate::schema::characters;
                characters::table
                    .filter(characters::id.eq(character_id))
                    .filter(characters::user_id.eq(user_id))
                    .select(CharacterDbEntryEconomy::as_select())
                    .for_no_key_update()
                    .load(&mut conn)
                    .await?
                    .into_iter()
                    .next()
                    .ok_or_else(|| BladeApiError::new(StatusCode::NOT_FOUND, 20000, 2))?
            };

            let period = daily_reward::current_period(now_secs());
            let already = entry.server_state.0.daily_reward.collected_period == Some(period);

            let mut reward = RewardGrant::default();
            let mut tracker = InventoryChangeTracker::default();

            if !already {
                if let Some(def) = daily_reward::reward_for_period(
                    &globals.static_data.daily_rewards,
                    period,
                ) {
                    // The reward the client is TOLD about has to be the reward it
                    // actually got — chests included.
                    //
                    // This used to copy only `stackable_items` onto `reward` and
                    // grant the chests straight off `def`, so on a day whose reward
                    // is a chest and nothing else, `RewardGrant` stayed empty and
                    // (every field being skip-if-empty) serialized as `"reward": {}`.
                    // The chest did land in the treasury; the client was simply told
                    // it had collected nothing, and sat there waiting for a reward to
                    // present. That is report #161, whose pasted body shows exactly
                    // that pair: `"reward": {}` beside a treasury holding the tier-2
                    // level-86 chest it had just been given.
                    //
                    // `complete_quest` has always done it this way — chests live ON
                    // the grant and are granted FROM it — so this is the daily path
                    // catching up, not a new convention.
                    reward.stackable_items = def.daily_reward.stackable_items.clone();
                    // `ChestDef` (tier/level) → `RewardChest`, which also carries the
                    // capture's chest id. `id: None` is correct: the treasury assigns a
                    // fresh numeric id on grant, so naming one here would collide.
                    reward.chests = def
                        .daily_reward
                        .chests
                        .iter()
                        .map(|c| RewardChest { id: None, tier: c.tier, level: c.level })
                        .collect();
                    apply_reward(
                        &reward,
                        &mut entry.wallet.0,
                        &mut entry.inventory.0,
                        &mut entry.character.0,
                        &mut tracker,
                    );
                    if !reward.stackable_items.is_empty() {
                        entry.inventory.0.backpack_version += 1;
                    }
                    if !reward.chests.is_empty() {
                        for chest in &reward.chests {
                            grant_chest(&mut entry.inventory.0, chest.tier, chest.level, &mut tracker);
                        }
                        entry.inventory.0.treasury_version += 1;
                    }
                }
                entry.server_state.0.daily_reward.collected_period = Some(period);
            }

            let status = status_for(&globals, period, true);
            let inventory = entry.inventory.0.generate_client_update(&tracker);
            write_back(&mut conn, entry).await?;

            Ok::<_, BladeApiError>(Json(CollectResponse {
                reward,
                daily_reward_status: status,
                inventory,
            }))
        }
        .scope_boxed()
    })
    .await
}

async fn write_back(
    conn: &mut diesel_async::AsyncPgConnection,
    entry: CharacterDbEntryEconomy,
) -> Result<(), BladeApiError> {
    use crate::schema::characters;
    diesel::update(characters::table)
        .filter(characters::id.eq(entry.id))
        .set(entry)
        .execute(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod collect_response_tests {
    use super::*;
    use blades_lib::economy::RewardGrant;

    /// Build the `reward` exactly as `collect_daily_reward` does, for one day's
    /// definition. Kept beside the handler so the two cannot drift silently.
    fn reward_for(payload: &blades_lib::features::daily_reward::DailyRewardPayload) -> RewardGrant {
        let mut reward = RewardGrant::default();
        reward.stackable_items = payload.stackable_items.clone();
        reward.chests = payload
            .chests
            .iter()
            .map(|c| RewardChest { id: None, tier: c.tier, level: c.level })
            .collect();
        reward
    }

    /// THE regression, in the reporter's own numbers.
    ///
    /// Report #161 pasted a collect response with `"reward": {}` sitting beside a
    /// treasury holding the tier-2 level-86 chest it had just been granted. Every
    /// `RewardGrant` field is skip-if-empty, so a chest-only day serialized as an
    /// empty object: the chest arrived, and the client was told it had collected
    /// nothing and hung waiting for something to present.
    #[test]
    fn a_chest_only_day_still_reports_the_chest() {
        let payload: blades_lib::features::daily_reward::DailyRewardPayload =
            serde_json::from_value(serde_json::json!({
                "chests": [{ "tier": 2, "level": 86 }]
            }))
            .expect("a chest-only day must deserialize");
        let reward = reward_for(&payload);

        assert!(!reward.is_empty(), "the reward must not be empty");
        assert_eq!(reward.chests.len(), 1);
        assert_eq!(reward.chests[0].tier, 2);
        assert_eq!(reward.chests[0].level, 86);

        let json = serde_json::to_value(&reward).expect("serializes");
        assert_ne!(
            json,
            serde_json::json!({}),
            "`reward: {{}}` is the bug — the client reads this to present the reward"
        );
        assert!(json.get("chests").is_some(), "the chest must reach the wire");
    }

    /// The control: a stackables-only day must be unchanged. The fix adds chests to
    /// the grant and must not disturb the path that already worked.
    #[test]
    fn a_stackable_only_day_is_unchanged() {
        let payload: blades_lib::features::daily_reward::DailyRewardPayload =
            serde_json::from_value(serde_json::json!({
                "stackableItems": { "e7193116-d761-479b-8a20-5633737977f5": 25 }
            }))
            .expect("a stackable-only day must deserialize");
        let reward = reward_for(&payload);

        assert!(reward.chests.is_empty());
        assert_eq!(reward.stackable_items.len(), 1);
        let json = serde_json::to_value(&reward).expect("serializes");
        assert!(json.get("stackableItems").is_some());
        assert!(
            json.get("chests").is_none(),
            "an empty chest list must stay off the wire, as every other reward does"
        );
    }

    /// A day carrying both must report both — the two branches are independent and
    /// an `else` between them would have passed the first test.
    #[test]
    fn a_day_with_both_reports_both() {
        let payload: blades_lib::features::daily_reward::DailyRewardPayload =
            serde_json::from_value(serde_json::json!({
                "stackableItems": { "e7193116-d761-479b-8a20-5633737977f5": 25 },
                "chests": [{ "tier": 1, "level": 40 }]
            }))
            .expect("a mixed day must deserialize");
        let reward = reward_for(&payload);
        assert_eq!(reward.chests.len(), 1);
        assert_eq!(reward.stackable_items.len(), 1);
    }

    /// An empty day stays empty — `reward: {}` is correct when there is genuinely
    /// nothing, and the fix must not start inventing rewards.
    #[test]
    fn an_empty_day_stays_empty() {
        let payload: blades_lib::features::daily_reward::DailyRewardPayload =
            serde_json::from_value(serde_json::json!({})).expect("an empty day deserializes");
        assert!(reward_for(&payload).is_empty());
    }
}
