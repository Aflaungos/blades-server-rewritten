#!/usr/bin/env python3
"""Generate `server/src/arena/pvp_tuning.rs` from `pvp_matchmaking.json`.

Follows the same convention as `server/src/arena/combat/gamedata.rs`: pure const
tables, no build.rs, no runtime serde on the hot path, source hash embedded so a
silent drift fails a test instead of the ladder.

    python3 script/extract_pvp_matchmaking.py   # bundle  -> json
    python3 script/gen_pvp_tuning_rs.py         # json    -> rust
"""
import hashlib
import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "server", "src", "arena", "pvp_matchmaking.json")
OUT = os.path.join(ROOT, "server", "src", "arena", "pvp_tuning.rs")

BANNER = "// ---------------------------- generated below, do not hand-edit ----------------------------"


def as_int(x):
    assert float(x).is_integer(), "non-integral value in a table typed as int: %r" % (x,)
    return int(x)


def main():
    raw = open(SRC, "rb").read()
    sha = hashlib.sha256(raw).hexdigest()
    d = json.loads(raw)

    L = []
    w = L.append
    w('//! **Shipped PvP tuning** — retail\'s own Elo ladder and match-reward tables,')
    w('//! read verbatim out of the game client\'s `Matchmaking` ScriptableObject.')
    w('//!')
    w('//! `[Class 1 — shipped game data]`. Every number in this file was authored by')
    w('//! Bethesda and shipped inside the APK; none of it is fitted, back-solved or')
    w('//! inferred. Source: the `common` asset bundle -> MonoBehaviour `Matchmaking`')
    w('//! (`BGS.Game.Network.Matchmaking`, TypeDefIndex 12439), whose field layout is')
    w('//! in `blades-capture/reference/il2cpp/dump.cs`.')
    w('//!')
    w('//! # Why this file exists at all')
    w('//!')
    w('//! `blades-capture/reference/game-defs/loot.json` is an export of this SAME')
    w('//! asset — but its extractor kept only `arenas` and four `tuning` keys and')
    w('//! dropped `_eloFactors`, `_eloResultScore`, `_trophyCountAdjustment`,')
    w('//! `_trophyEquivalence`, `_pvpExperienceRules` and `_pvpSoftCurrencyRules`.')
    w('//! Because those were missing from the derived export, the arena scoring and')
    w('//! economy were previously modelled by fitting captures. They do not need to be:')
    w('//! the shipped tables reproduce the retail victory cards exactly (see')
    w('//! `arena_ladder::tests`), where the fit was only good to ~5%.')
    w('//!')
    w('//! # Regenerating')
    w('//!')
    w('//! ```text')
    w('//! python3 script/extract_pvp_matchmaking.py   # APK bundle -> pvp_matchmaking.json')
    w('//! python3 script/gen_pvp_tuning_rs.py         # json       -> this file')
    w('//! ```')
    w('')
    w('#![allow(dead_code)]')
    w('')
    w('/// sha256 of `server/src/arena/pvp_matchmaking.json`, the JSON this file was')
    w('/// generated from. Asserted by `tests::const_tables_match_the_committed_json`.')
    w('pub const SOURCE_JSON_SHA256: &str = "%s";' % sha)
    w('')
    w('/// sha256 of the APK asset bundle the JSON was extracted from.')
    w('pub const SOURCE_BUNDLE_SHA256: &str = "%s";' % d["_meta"]["bundle_sha256"])
    w('')
    w(BANNER)
    w('')

    # --- scalars
    w('/// `Matchmaking._trophyGainFloor` — a match never moves fewer trophies than')
    w('/// this in the direction its result demands.')
    w('pub const TROPHY_GAIN_FLOOR: i64 = %d;' % d["trophy_gain_floor"])
    w('')
    w('/// `Matchmaking._minimumTrophyCountToIgnoreEpl` — above this trophy count the')
    w('/// matchmaker stops considering Effective Player Level and goes on trophies')
    w('/// alone. Matchmaking input, not a scoring term.')
    w('pub const MINIMUM_TROPHY_COUNT_TO_IGNORE_EPL: i64 = %d;'
      % d["minimum_trophy_count_to_ignore_epl"])
    w('')

    # --- elo factors
    w('/// One rung of `Matchmaking._eloFactors`: the Elo K-factor in force from')
    w('/// `trophy_count` upwards.')
    w('#[derive(Debug, Clone, Copy, PartialEq, Eq)]')
    w('pub struct EloFactor {')
    w('    /// Lower bound (inclusive) of the trophy band.')
    w('    pub trophy_count: i64,')
    w('    /// The K-factor applied to players inside the band.')
    w('    pub k_factor: i64,')
    w('}')
    w('')
    w('/// `Matchmaking._eloFactors` — **this is the "early days vs later days" term**.')
    w('/// K starts at 100 for a brand-new ladder entrant and decays to 50 in the top')
    w('/// arena, so the same result moves twice as many trophies early on as it does')
    w('/// late. Bands are keyed on the player\'s own trophy count, ascending.')
    ef = d["elo_factors"]
    w('pub const ELO_FACTORS: [EloFactor; %d] = [' % len(ef))
    for e in ef:
        w('    EloFactor { trophy_count: %d, k_factor: %d },' % (e["trophy_count"], e["k_factor"]))
    w('];')
    w('')

    # --- elo result score
    ers = d["elo_result_score"]
    w('/// `Matchmaking._eloResultScore` — the Elo *actual score* `S` for each')
    w('/// best-of-three outcome. Retail did NOT score a match 1/0: a 2-1 win is worth')
    w('/// `0.92`, and losing 1-2 still banks `0.12`, so the round score feeds the')
    w('/// trophy swing as well as the gold.')
    w('#[derive(Debug, Clone, Copy, PartialEq)]')
    w('pub struct EloResultScore {')
    w('    /// Won 2-0.')
    w('    pub won_every_round: f64,')
    w('    /// Won 2-1.')
    w('    pub won_majority_of_rounds: f64,')
    w('    /// Lost 0-2.')
    w('    pub lost_every_round: f64,')
    w('    /// Lost 1-2.')
    w('    pub lost_majority_of_rounds: f64,')
    w('    /// Drawn — unreachable in a best-of-three, shipped anyway.')
    w('    pub tie: f64,')
    w('}')
    w('')
    w('/// The shipped `EloResultScore` row.')
    w('pub const ELO_RESULT_SCORE: EloResultScore = EloResultScore {')
    for k in ("won_every_round", "won_majority_of_rounds", "lost_every_round",
              "lost_majority_of_rounds", "tie"):
        w('    %s: %s,' % (k, repr(float(ers[k]))))
    w('};')
    w('')

    # --- trophy count adjustment
    tca = d["trophy_count_adjustment"]
    mp = tca["match_played_to_trophies_modifier"]
    w('/// `Matchmaking._trophyCountAdjustment._matchPlayedToTrophiesModifier` —')
    w('/// indexed by the character\'s `numberPvpMatchPlayed`, clamped to the last')
    w('/// entry. Shipped as a percentage.')
    w('///')
    w('/// `[Class 3 — role not established]`. The values are shipped; what retail')
    w('/// MULTIPLIED by them is not. The name and the neighbouring')
    w('/// [`EPL_TO_TROPHY_DEVIATION`] both point at the matchmaking search window')
    w('/// (a provisional-rating widening that shrinks as a player logs matches),')
    w('/// NOT at the trophy delta — so nothing in this crate multiplies a trophy')
    w('/// swing by it. See `docs/arena-season-model.md`.')
    w('pub const MATCH_PLAYED_TO_TROPHIES_MODIFIER: [i64; %d] = [%s];'
      % (len(mp), ", ".join(str(v) for v in mp)))
    w('')
    epl = tca["epl_to_trophy_count"]
    w('/// `Matchmaking._trophyCountAdjustment._eplToTrophyCountList` as')
    w('/// `(trophy_count, deviation)` — the trophy-space deviation allowed when')
    w('/// matching on Effective Player Level. Matchmaking input.')
    w('pub const EPL_TO_TROPHY_DEVIATION: [(i64, i64); %d] = [%s];'
      % (len(epl), ", ".join("(%d, %d)" % (e["trophy_count"], e["deviation"]) for e in epl)))
    w('')

    # --- arenas
    w('/// Per-arena matchmaking behaviour from `Matchmaking._arenas[]`. The trophy')
    w('/// thresholds and reward tables live in [`super::arena_ladder::ARENA_LADDER`];')
    w('/// this carries only the streak-exception and rating-mix knobs.')
    w('#[derive(Debug, Clone, Copy, PartialEq, Eq)]')
    w('pub struct ArenaMatchmakingParams {')
    w('    /// `arena_01` .. `arena_06`.')
    w('    pub arena_key: &\'static str,')
    w('    /// Trophies required to enter the arena.')
    w('    pub required_trophy_count: i64,')
    w('    /// `_arenaRarityLevel`.')
    w('    pub arena_rarity_level: i64,')
    w('    /// Percent chance the matchmaker uses the composite player-rating score')
    w('    /// instead of raw trophies. 90 in arena 1, 0 from arena 5 up.')
    w('    pub chance_of_using_player_rating_score: i64,')
    w('    /// Consecutive wins that trigger the "harder match" exception.')
    w('    pub num_wins_to_trigger_exception: i64,')
    w('    /// How many exception matches a win streak buys.')
    w('    pub num_exception_matches_after_win_streak: i64,')
    w('    /// Consecutive losses that trigger the "easier match" exception.')
    w('    pub num_losses_to_trigger_exception: i64,')
    w('    /// How many exception matches a loss streak buys.')
    w('    pub num_exception_matches_after_loss_streak: i64,')
    w('    /// Matchmaking-score offset applied during a loss-streak exception.')
    w('    pub matchmaking_score_offset_after_loss_streak: i64,')
    w('    /// Matchmaking-score offset applied during a win-streak exception.')
    w('    pub matchmaking_score_offset_after_win_streak: i64,')
    w('}')
    w('')
    ar = d["arenas"]
    w('/// The six shipped arena rows, in ladder order.')
    w('pub const ARENA_MATCHMAKING: [ArenaMatchmakingParams; %d] = [' % len(ar))
    for a in ar:
        w('    ArenaMatchmakingParams { arena_key: "%s", required_trophy_count: %d, '
          'arena_rarity_level: %d, chance_of_using_player_rating_score: %d, '
          'num_wins_to_trigger_exception: %d, num_exception_matches_after_win_streak: %d, '
          'num_losses_to_trigger_exception: %d, num_exception_matches_after_loss_streak: %d, '
          'matchmaking_score_offset_after_loss_streak: %d, '
          'matchmaking_score_offset_after_win_streak: %d },'
          % (a["arena_key"], as_int(a["required_trophy_count"]), a["arena_rarity_level"],
             as_int(a["chance_of_using_player_rating_score"]),
             a["num_wins_to_trigger_exception"], a["num_exception_matches_after_win_streak"],
             a["num_losses_to_trigger_exception"], a["num_exception_matches_after_loss_streak"],
             a["matchmaking_score_offset_after_loss_streak"],
             a["matchmaking_score_offset_after_win_streak"]))
    w('];')
    w('')

    # --- xp rules
    xr = d["pvp_experience_rules"]
    w('/// One row of `Matchmaking._pvpExperienceRules` — the character XP a match')
    w('/// pays at one character level.')
    w('#[derive(Debug, Clone, Copy, PartialEq, Eq)]')
    w('pub struct PvpExperienceRule {')
    w('    /// The character level this row applies to (1..=100).')
    w('    pub character_level: u16,')
    w('    /// Flat base, paid on every match.')
    w('    pub base_xp: i64,')
    w('    /// Added by rounds won: index 0/1/2 for 0, 1 or 2 rounds.')
    w('    pub xp_bonus_per_round_won: [i64; 3],')
    w('    /// Extra on a 2-0 win.')
    w('    pub win_xp_bonus_2_to_0: i64,')
    w('    /// Extra on a 2-1 win.')
    w('    pub win_xp_bonus_2_to_1: i64,')
    w('    /// Shipped upset bonus. NOT applied — the trigger condition is unknown and')
    w('    /// no captured card needs it; see `arena_ladder::match_reward`.')
    w('    pub trophy_diff_xp_bonus: i64,')
    w('    /// Added by arena, indexed `arena - 1` (arena 1 pays 0).')
    w('    pub arena_xp_bonus: [i64; 6],')
    w('}')
    w('')
    w('/// `Matchmaking._pvpExperienceRules`, dense and level-ordered (index = level-1).')
    w('pub const PVP_EXPERIENCE_RULES: [PvpExperienceRule; %d] = [' % len(xr))
    for r in xr:
        assert len(r["xp_bonus_per_round_won"]) == 3 and len(r["arena_xp_bonus"]) == 6
        w('    PvpExperienceRule { character_level: %d, base_xp: %d, xp_bonus_per_round_won: [%s], '
          'win_xp_bonus_2_to_0: %d, win_xp_bonus_2_to_1: %d, trophy_diff_xp_bonus: %d, '
          'arena_xp_bonus: [%s] },'
          % (r["character_level"], r["base_xp"],
             ", ".join(str(as_int(v)) for v in r["xp_bonus_per_round_won"]),
             r["win_xp_bonus_2_to_0"], r["win_xp_bonus_2_to_1"], r["trophy_diff_xp_bonus"],
             ", ".join(str(as_int(v)) for v in r["arena_xp_bonus"])))
    w('];')
    w('')

    # --- currency rules
    cr = d["pvp_soft_currency_rules"]
    w('/// One row of `Matchmaking._pvpSoftCurrencyRules` — the gold a match pays at')
    w('/// one character level. Note `currency_bonus_per_round_won[0]` is NEGATIVE:')
    w('/// a 0-2 loss is paid below base.')
    w('#[derive(Debug, Clone, Copy, PartialEq, Eq)]')
    w('pub struct PvpSoftCurrencyRule {')
    w('    /// The character level this row applies to (1..=100).')
    w('    pub character_level: u16,')
    w('    /// Flat base, paid on every match.')
    w('    pub base_currency: i64,')
    w('    /// Added by rounds won: index 0/1/2 for 0, 1 or 2 rounds. Index 0 is negative.')
    w('    pub currency_bonus_per_round_won: [i64; 3],')
    w('    /// Extra on a 2-0 win.')
    w('    pub win_currency_bonus_2_to_0: i64,')
    w('    /// Extra on a 2-1 win.')
    w('    pub win_currency_bonus_2_to_1: i64,')
    w('    /// Shipped upset bonus — zero in every shipped row.')
    w('    pub trophy_diff_currency_bonus: i64,')
    w('    /// Added by arena, indexed `arena - 1`.')
    w('    pub arena_currency_bonus: [i64; 6],')
    w('}')
    w('')
    w('/// `Matchmaking._pvpSoftCurrencyRules`, dense and level-ordered (index = level-1).')
    w('pub const PVP_SOFT_CURRENCY_RULES: [PvpSoftCurrencyRule; %d] = [' % len(cr))
    for r in cr:
        assert len(r["currency_bonus_per_round_won"]) == 3 and len(r["arena_currency_bonus"]) == 6
        w('    PvpSoftCurrencyRule { character_level: %d, base_currency: %d, '
          'currency_bonus_per_round_won: [%s], win_currency_bonus_2_to_0: %d, '
          'win_currency_bonus_2_to_1: %d, trophy_diff_currency_bonus: %d, '
          'arena_currency_bonus: [%s] },'
          % (r["character_level"], r["base_currency"],
             ", ".join(str(as_int(v)) for v in r["currency_bonus_per_round_won"]),
             r["win_currency_bonus_2_to_0"], r["win_currency_bonus_2_to_1"],
             r["trophy_diff_currency_bonus"],
             ", ".join(str(as_int(v)) for v in r["arena_currency_bonus"])))
    w('];')
    w('')

    open(OUT, "w").write("\n".join(L))
    print("wrote %s (%d lines, source sha %s)" % (OUT, len(L), sha[:12]))


if __name__ == "__main__":
    main()
