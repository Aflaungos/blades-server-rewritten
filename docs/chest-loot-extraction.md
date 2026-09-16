# Chest loot tables, derived from retail capture

Tracker report #141 — "the sigil shop is empty and **chests in the store still load
the same loot**". The second half was true by construction: `chest_loots.json` was a
flat list of 40 reward bundles and `pick_loot` chose one by hashing the chest id, so
neither the chest's **tier** nor its **level** changed anything. A tier-1 starter
chest and a tier-5 Elder chest could draw the same bundle.

Retail rolled chest contents server-side at open time and never shipped the loot
tables in the APK, so we cannot reproduce the roll. What we can do — and what this
document describes — is replace the invented pool with bundles Bethesda's server
*actually returned*, separated by tier and level.

## Method

`script/extract_chest_loot.py` (committed; both phases are re-runnable).

### Phase 1 — `opens`: recover each opening's tier and level

Chest openings are `POST /characters/{uuid}/chests/{slot}/collect`. The response
carries the full `reward` block, but **the request names only the treasury slot id**
(`"1"`, `"2"`, … per character) — never the tier. Tiers are recovered by replaying
each character's capture stream in capture-id order and tracking its treasury:

| source | shape | effect on the tracked map |
| --- | --- | --- |
| `GET /character/{uuid}/inventories/current` | `inventory.treasury.chests[]` — a **full** listing of `{id, tier, level}` | replaces the map |
| `POST …/quests/{uuid}/dungeons/current/update` | partial `chests[]`, the chest just found | upsert |
| `POST …/quests/{uuid}/complete` | partial `chests[]` | upsert |
| `POST …/abysses/current/update` | partial `chests[]` | upsert |
| any response carrying `treasury.removedChests[]` | slot ids | delete |

At a collect the slot is looked up in the map as of that moment (before the collect's
own `removedChests` is applied).

**This step needed a control.** A first pass that scanned only the "obvious" endpoints
left 452 of 766 openings unresolved. The missing source was
`quests/{uuid}/dungeons/current/update` — chests are added *mid-dungeon*, not at
completion — which contributed 663 of the 753 delta observations. Scanning it brought
unresolved down to 6. A second diagnostic (is the slot unknown because it was never
seen, or because of timing?) reported **0** openings whose slot was never seen for
that character, which is what says the source set is now complete rather than merely
larger.

Both databases are opened `mode=ro` with `PRAGMA query_only`; nothing is written to
production.

### Phase 2 — `tables`: aggregate

Each surviving opening becomes one `{chestLevel, reward}` entry in its tier's pool.
The reward is **verbatim** — no synthesis, no smoothing, no fitted distribution. The
instanced item `id` is kept because `RewardItem.id` is required; the collect handler
re-mints it per grant.

## Sample sizes and what was dropped

766 collect captures were found. 741 are usable:

| dropped | n | why |
| --- | --- | --- |
| post-retail (≥ 2026-06-30) | 14 | our own server answering, not Bethesda |
| HTTP 400 | 2 | no reward was rolled |
| tier never observed | 4 | slot seen for the character, but not in a window covering the open |
| tutorial chest, `tier: -1` | 5 | see below |

## Per-tier results

| tier | openings | distinct bundles | characters | users | chest levels | instanced items per open | gold |
| ---: | ---: | ---: | ---: | ---: | --- | --- | --- |
| 1 | 154 | 154 | 14 | 11 | 1–93 (49 distinct, max gap 16) | 0 in 110, 1 in 44 | 176–1071 |
| 2 | 305 | 305 | 24 | 18 | 2–100 (65 distinct, max gap 21) | always 1 | 385–3229 |
| 3 | 270 | 270 | 23 | 15 | 3–100 (64 distinct, max gap 9) | always 2 | 1716–9707 |
| 4 | **11** | 11 | 6 | 5 | 54–94 (6 distinct) | always 2 | 20958–23920 |
| 5 | **1** | 1 | 1 | 1 | 93 | 2 | 55598 |

Every bundle within a tier is distinct (154/154, 305/305, 270/270) — retail genuinely
randomised each roll.

### Findings that hold across the whole set

**The item count per open is deterministic in the tier.** Tier 2 returned exactly one
instanced item in all 305 openings, tier 3 exactly two in all 270, tier 4 exactly two
in all 11. Tier 1 is the exception and returned an item only 44 times in 154 (28.6%).

**Enchantment tier is the sharpest tier signal.**

| chest tier | enchantment tiers seen (count) | enchantments per dropped item |
| ---: | --- | --- |
| 1 | 1×23, 2×8, 3×2 | 0 in 19, 1 in 19, 2 in 4, 3 in 2 |
| 2 | 1×136, 2×120, 3×92, 4×17, 5×1 | 0 in 98, 1 in 103, 2 in 49, 3 in 55 |
| 3 | 1×322, 2×232, 3×182, 4×122, 5×10 | 0 in 109, 1 in 168, 2 in 89, 3 in 174 |
| 4 | 5×24, 6×16, 7×6 | 0 in 3, 1 in 3, 2 in 5, 3 in 11 |
| 5 | 6×2 | one item with 0, one with 2 |

Tier 4 never dropped an enchantment below tier 5; tiers 1–3 never dropped one above
tier 5. The bands barely overlap.

**Gems only come out of low tiers.** The gem currency
(`470c8f58-a8dd-4c07-8c92-843b785e1139`) appeared in **154/154** tier-1 openings
(1–2 gems) and **159/305** tier-2 openings (2–3 gems), and in **0** of the 282
tier-3/4/5 openings. Control: the same query finds gems in tiers 1 and 2, so their
absence at tier 3+ is a real absence and not a broken probe.

**Gold scales with chest level, then saturates.** A linear fit is poor (R² = 0.31 /
0.48 / 0.54 for tiers 1/2/3) because the curve flattens, not because level does not
matter:

| tier | chest lvl 1–10 | 11–20 | 21–30 | 31–40 | 81–90 |
| ---: | --- | --- | --- | --- | --- |
| 1 | 176–357 (n22) | 285–990 (n48) | 851–993 (n36) | 849–998 (n24) | 1036 (n1) |
| 2 | 385–932 (n23) | 889–2984 (n37) | 954–3010 (n74) | 1781–3012 (n49) | 2700–3229 (n57) |
| 3 | 1716–4450 (n11) | 2896–8878 (n20) | 3874–8978 (n42) | 6258–9016 (n83) | 8069–9675 (n52) |

Each tier appears to have a gold ceiling (~1070 / ~3230 / ~9710 / ~23900 / 55598)
reached by roughly chest level 20–30, with a wide random band below it. **So yes,
chest level affects contents** — which is why the published pools are level-tagged
and `pick_loot` selects on level.

**One stackable is universal above tier 1.** `42d91529-c88b-4c5b-815b-b55508b4e7ef`
appeared in 305/305 tier-2, 270/270 tier-3, 11/11 tier-4 and 1/1 tier-5 openings, with
quantity scaling by tier (6–42, 12–201, 80–266, 158). At tier 1 it appeared only
8/154 times. Beyond it the stackable pool is broad: 56 distinct at tier 1, 106 at
tier 2, 104 at tier 3.

**The item template pool is broad and not obviously level-gated.** Tier 2 dropped 123
distinct templates over 305 drops (most frequent: 9 occurrences, 3.0%); tier 3, 152
over 540 (most frequent: 11, 2.0%). Individual templates recur across nearly the full
chest-level range (e.g. `1b1d7a8e-…` at chest levels 13 through 100), so we could not
establish a per-level item whitelist from this sample.

### The tutorial chest: `tier: -1`

Five openings carry tier **-1** at level 1, always in slot "1", on five different
characters and five different dates spanning May–June 2026. This is not corrupt data:
retail's own `inventory.treasury.chests[]` reported the tier as -1. It is a scripted
first chest, and its contents are near-identical across all five (gold 58–67, always
exactly one instanced item, always the same four stackables:
`05a7d501`×1, `42d91529`×25–27, `b81952e0`×8–20, `f00f350d`×15–20).

It is **not published**, for a mechanical reason: our `Chest.tier` is a `u64` and
cannot represent -1. Granting the tutorial chest faithfully needs a signed tier (or a
separate starter-grant path) and is left as a follow-up. The five bundles are in the
capture and the extraction script reports them under
`tutorial_chest_tier_minus_1` in its stats output.

## What is published, and what the server does with it

`deploy/static/chest_loots.json`:

```jsonc
{
  "tiers":            { "1": [ {"chestLevel": 1, "reward": { … }}, … ], "2": [ … ], "3": [ … ] },
  "provisionalTiers": { "4": [ … 11 entries … ], "5": [ … 1 entry … ] }
}
```

`pick_loot(tables, tier, level, chest_id)`:
1. takes the tier's own pool (`tiers`, else `provisionalTiers`);
2. narrows to the samples whose `chestLevel` is nearest the chest's level;
3. picks among those by hashing `chest_id` — deterministic, so a retry yields the
   same loot.

**Confidence per tier.**

- **Tiers 1–3 — good.** 154/305/270 openings from 11–18 distinct users and 14–24
  characters, covering 49–65 distinct chest levels each with a worst-case gap of 21
  levels. Enough for the pool to feel like a table rather than a loop.
- **Tier 4 — thin, published as provisional.** 11 openings from 5 users, chest levels
  54–94 only. A tier-4 chest will repeat one of eleven real bundles, and a tier-4
  chest below level ~54 gets a bundle rolled for a much higher-level chest. This is
  still closer to retail than handing it the tier-3 table, but it is a small pool and
  should be replaced if more tier-4 captures surface.
- **Tier 5 — one sample, published as provisional.** Every tier-5 chest yields the
  same bundle (gold 55598, two items, 14 stackables), rolled at chest level 93. It is
  a real bundle and nothing about it is invented, but it is not a distribution. If a
  single repeating tier-5 reward is worse than none, delete the `"5"` key from
  `provisionalTiers` — the loader tolerates it (the startup assert in
  `static_loader` would then need relaxing to `1..=4`).

`static_loader`'s `committed_static_data_loads_non_empty` asserts tiers 1–3 each hold
≥100 bundles and that every tier 1–5 has a pool **of its own**, so `pick_loot` can
never silently fall back to a neighbouring tier's table. Both asserts were
mutation-tested: removing tier 2 fails with `chest_loots.json tier 2 pool`, and
removing provisional tier 5 fails with `has no pool of its own for tier 5`.
`committed_chest_loot_varies_by_tier_and_level` asserts gold rises strictly across
tiers 1→5 at a fixed chest id and level, and that each of tiers 1–3 pays differently
at chest level 1 and 100 — it fails with
`tier 1 paid 923 but tier 2 paid 923` if the tiers are made to share a pool, i.e. it
reproduces report #141's symptom.

## What this could NOT determine

- **The actual loot tables.** These are pools of observed outcomes, not the drop
  tables retail rolled against. Item drop rates below roughly 1-in-150 are invisible
  at this sample size, and nothing here can produce a bundle no player ever saw.
- **The gold formula.** The level→gold relationship is visibly a rising curve with a
  per-tier ceiling, but 154–305 points with a ±2× random band at fixed level are not
  enough to separate `min(cap, a + b·level)` from several other shapes. No formula is
  published; the level-tagged samples carry the effect empirically instead.
- **Whether *character* level or *chest* level drives the scaling.** Only the chest's
  level is recorded in the treasury, and the two are correlated in practice. The
  residual spread at a fixed chest level (e.g. tier 1, level 18: 491–990) could be
  random roll, character level, or something else.
- **Per-level item whitelists.** See above — templates recur across the whole level
  range, so no level gating could be established.
- **Whether `fastTrack` changes the roll.** 93 of 766 openings used
  `fastTrack=True` (gem-skip) and 673 did not, but they are not paired on the same
  chest, so the comparison has no control. The field is recorded in the intermediate
  and left unused.
- **Tiers above 5.** None were observed. If one exists, `pick_loot` falls back to the
  nearest table below it and the startup assert does not cover it.
- **4 openings' tiers**, where the slot was seen for the character but not in a window
  covering the open.

## Reproducing

```sh
# phase 1, on the capture box (read-only; ~90 s, 65k bodies out of the archive)
sudo nice -n 15 python3 extract_chest_loot.py opens --out /tmp/chest_opens.jsonl

# phase 2, anywhere
python3 script/extract_chest_loot.py tables \
    --opens chest_opens.jsonl \
    --out deploy/static/chest_loots.json \
    --stats /tmp/chest_loot_stats.json
```
