use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
#[serde(rename_all = "UPPERCASE")]
pub enum QuestType {
    Normal,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
#[serde(rename_all = "PascalCase")]
pub enum QuestStatus {
    Active,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ObjectiveStatus {
    pub status: QuestStatus,
    pub progress: f64,
    pub completed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Quest {
    pub version: u64,
    pub r#type: QuestType,
    pub objective_statuses: HashMap<Uuid, ObjectiveStatus>,
    pub difficulty_level: i64,
    /// Retail's quest seed is SIGNED and routinely negative — a transfer carrying
    /// one failed to deserialize with `invalid value: integer -1785270870,
    /// expected u64`, taking the whole character import down with it (report #59).
    /// Every other seed in the codebase is already `i64`; this was the outlier,
    /// and `jobs_gen` had to cast its own signed seed back through `as u64` to
    /// fit it.
    pub seed: i64,
    pub gld_quest_id: Uuid,
    pub completed: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QuestWithId {
    pub quest_id: Uuid,
    #[serde(flatten)]
    pub quest: Quest,
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Report #59: a character transfer died with
    /// `Json deserialize error: invalid value: integer -1785270870, expected u64`.
    ///
    /// Retail's quest seed is signed and often negative. `Quest.seed` was the only
    /// `u64` seed in the codebase, so ONE such quest failed the whole
    /// `import-character` body and the player could not transfer at all.
    ///
    /// The value below is the exact one from his error.
    #[test]
    fn a_negative_retail_seed_deserializes() {
        let q: QuestWithId = serde_json::from_value(serde_json::json!({
            "questId": "159bc1e7-454c-4e2a-90cf-e200c74b961a",
            "version": 2,
            "type": "NORMAL",
            "objectiveStatuses": {},
            "difficultyLevel": -1,
            "seed": -1785270870i64,
            "gldQuestId": "159bc1e7-454c-4e2a-90cf-e200c74b961a",
            "completed": false,
        }))
        .expect("a negative seed is normal retail data and must not fail the import");
        // Compared through JSON rather than against a typed literal: a negative
        // literal would not COMPILE against a u64 field, and a compile error is
        // weaker evidence than watching the deserialize itself fail.
        let back = serde_json::to_value(&q).unwrap();
        assert_eq!(back["seed"], serde_json::json!(-1785270870i64));
    }

    /// It must round-trip unchanged: casting a negative seed through `u64` would
    /// hand the client a huge positive number instead of the value retail used.
    #[test]
    fn a_negative_seed_round_trips() {
        let src = serde_json::json!({
            "questId": "159bc1e7-454c-4e2a-90cf-e200c74b961a",
            "version": 2,
            "type": "NORMAL",
            "objectiveStatuses": {},
            "difficultyLevel": -1,
            "seed": -1785270870i64,
            "gldQuestId": "159bc1e7-454c-4e2a-90cf-e200c74b961a",
            "completed": false,
        });
        let q: QuestWithId = serde_json::from_value(src).unwrap();
        let back = serde_json::to_value(&q).unwrap();
        assert_eq!(back["seed"], serde_json::json!(-1785270870i64));
    }

    /// Positive seeds, which most captured quests carry, still work.
    #[test]
    fn a_positive_seed_still_works() {
        let q: QuestWithId = serde_json::from_value(serde_json::json!({
            "questId": "159bc1e7-454c-4e2a-90cf-e200c74b961a",
            "version": 2, "type": "NORMAL", "objectiveStatuses": {},
            "difficultyLevel": -1, "seed": 485975867,
            "gldQuestId": "159bc1e7-454c-4e2a-90cf-e200c74b961a", "completed": false,
        }))
        .unwrap();
        let back = serde_json::to_value(&q).unwrap();
        assert_eq!(back["seed"], serde_json::json!(485975867i64));
    }
}
