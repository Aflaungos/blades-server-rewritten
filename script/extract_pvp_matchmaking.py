#!/usr/bin/env python3
"""Extract the shipped `Matchmaking` ScriptableObject from the game client.

This is the asset that carries retail's PvP tuning: the Elo K-factor ladder, the
round-score -> Elo "actual score" mapping, the trophy-count adjustment, and the
per-character-level XP / soft-currency reward rules.

`blades-capture/reference/game-defs/loot.json` was extracted from this same asset
but kept only `arenas` + four `tuning` keys; it DROPPED `_eloFactors`,
`_eloResultScore`, `_trophyCountAdjustment`, `_trophyEquivalence`,
`_pvpExperienceRules` and `_pvpSoftCurrencyRules`. Everything this script adds is
verbatim shipped data, not a fit.

The `common` asset bundle ships embedded type trees, so plain UnityPy reads it
with no TypeTreeGenerator.

    python3 script/extract_pvp_matchmaking.py \
        --bundle /tmp/blades-apk-extract/assets/Bundles/common \
        --out server/src/arena/pvp_matchmaking.json

Then regenerate the Rust tables:

    python3 script/gen_pvp_tuning_rs.py
"""
import argparse
import hashlib
import json
import os
import sys

DEFAULT_BUNDLE = "/tmp/blades-apk-extract/assets/Bundles/common"
MARKERS = ("_eloFactors", "_eloResultScore", "_pvpExperienceRules")


def find_matchmaking(bundle_path):
    import UnityPy

    env = UnityPy.load(bundle_path)
    for obj in env.objects:
        if obj.type.name != "MonoBehaviour":
            continue
        try:
            tree = obj.read_typetree()
        except Exception:
            continue
        if all(k in tree for k in MARKERS):
            return tree
    return None


def rounded(x):
    """Unity serialises these as float32; the authored values are integral or
    2-decimal. Round to 6 places so 0.9200000166893005 -> 0.92."""
    if isinstance(x, float):
        return round(x, 6)
    return x


def dump_one_row_per_line(obj):
    """Top-level keys indented, but every list element on a single line — a
    100-row table stays diff-readable without becoming 4 500 lines."""
    parts = []
    for key, val in obj.items():
        if isinstance(val, list):
            rows = ",\n  ".join(json.dumps(v, sort_keys=False) for v in val)
            parts.append(' %s: [\n  %s\n ]' % (json.dumps(key), rows))
        else:
            parts.append(' %s: %s' % (json.dumps(key), json.dumps(val, indent=2).replace("\n", "\n ")))
    return "{\n" + ",\n".join(parts) + "\n}\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bundle", default=DEFAULT_BUNDLE)
    ap.add_argument("--out", default=os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "server", "src", "arena", "pvp_matchmaking.json"))
    args = ap.parse_args()

    if not os.path.exists(args.bundle):
        sys.exit("bundle not found: %s\n"
                 "unzip the APK first (blades-capture/tools/apk-extract)" % args.bundle)

    tree = find_matchmaking(args.bundle)
    if tree is None:
        sys.exit("no MonoBehaviour with %s in %s" % (", ".join(MARKERS), args.bundle))

    with open(args.bundle, "rb") as fh:
        bundle_sha = hashlib.sha256(fh.read()).hexdigest()

    out = {
        "_meta": {
            "source": "APK asset bundle `common` -> MonoBehaviour `Matchmaking` "
                      "(BGS.Game.Network.Matchmaking, TypeDefIndex 12439)",
            "bundle_sha256": bundle_sha,
            "extractor": "script/extract_pvp_matchmaking.py",
            "note": "Verbatim shipped values. Floats are float32 rounded to 6dp.",
        },
        "trophy_gain_floor": tree["_trophyGainFloor"],
        "minimum_trophy_count_to_ignore_epl": tree["_minimumTrophyCountToIgnoreEpl"],
        "elo_factors": [
            {"trophy_count": e["_trophyCount"], "k_factor": e["_kfactor"]}
            for e in tree["_eloFactors"]
        ],
        "elo_result_score": {
            "won_every_round": rounded(tree["_eloResultScore"]["_wonEveryRounds"]),
            "won_majority_of_rounds": rounded(tree["_eloResultScore"]["_wonMajorityOfRounds"]),
            "lost_every_round": rounded(tree["_eloResultScore"]["_lostEveryRounds"]),
            "lost_majority_of_rounds": rounded(tree["_eloResultScore"]["_lostMajorityOfRounds"]),
            "tie": rounded(tree["_eloResultScore"]["_tie"]),
        },
        "trophy_count_adjustment": {
            "epl_to_trophy_count": [
                {"trophy_count": e["_trophyCount"], "deviation": e["_deviation"]}
                for e in tree["_trophyCountAdjustment"]["_eplToTrophyCountList"]
            ],
            "match_played_to_trophies_modifier":
                list(tree["_trophyCountAdjustment"]["_matchPlayedToTrophiesModifier"]),
        },
        "trophy_equivalence": [
            {"expected_trophy_count": e["_expectedTrophyCount"],
             "max_disparity_percentage": rounded(e["_maxDisparityPercentage"])}
            for e in tree["_trophyEquivalence"]
        ],
        "arenas": [
            {
                "arena_key": a["_arenaKey"],
                "required_trophy_count": rounded(a["_requiredTrophyCount"]),
                "arena_rarity_level": a["_arenaRarityLevel"],
                "chance_of_using_player_rating_score": rounded(a["_chanceOfUsingPlayerRatingScore"]),
                "num_wins_to_trigger_exception": a["_numWinsToTriggerException"],
                "num_exception_matches_after_win_streak": a["_numExceptionMatchesAfterWinStreak"],
                "num_losses_to_trigger_exception": a["_numLossesToTriggerException"],
                "num_exception_matches_after_loss_streak": a["_numExceptionMatchesAfterLossStreak"],
                "matchmaking_score_offset_after_loss_streak": a["_matchmakingScoreOffsetAfterLossStreak"],
                "matchmaking_score_offset_after_win_streak": a["_matchmakingScoreOffsetAfterWinStreak"],
            }
            for a in tree["_arenas"]
        ],
        "pvp_experience_rules": [
            {
                "character_level": r["_characterLevel"],
                "base_xp": r["_baseXp"],
                "xp_bonus_per_round_won": list(r["_xpBonusPerRoundWon"]),
                "win_xp_bonus_2_to_0": r["_winXpBonus2To0"],
                "win_xp_bonus_2_to_1": r["_winXpBonus2To1"],
                "trophy_diff_xp_bonus": r["_trophyDiffXpBonus"],
                "arena_xp_bonus": [rounded(v) for v in r["_arenaXpBonus"]],
            }
            for r in tree["_pvpExperienceRules"]
        ],
        "pvp_soft_currency_rules": [
            {
                "character_level": r["_characterLevel"],
                "base_currency": r["_baseCurrency"],
                "currency_bonus_per_round_won": list(r["_currencyBonusPerRoundWon"]),
                "win_currency_bonus_2_to_0": r["_winCurrencyBonus2To0"],
                "win_currency_bonus_2_to_1": r["_winCurrencyBonus2To1"],
                "trophy_diff_currency_bonus": r["_trophyDiffCurrencyBonus"],
                "arena_currency_bonus": [rounded(v) for v in r["_arenaCurrencyBonus"]],
            }
            for r in tree["_pvpSoftCurrencyRules"]
        ],
    }

    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w") as fh:
        fh.write(dump_one_row_per_line(out))
    print("wrote %s (bundle sha256 %s)" % (args.out, bundle_sha))
    print("  elo_factors           : %d" % len(out["elo_factors"]))
    print("  pvp_experience_rules  : %d" % len(out["pvp_experience_rules"]))
    print("  pvp_soft_currency_rules: %d" % len(out["pvp_soft_currency_rules"]))


if __name__ == "__main__":
    main()
