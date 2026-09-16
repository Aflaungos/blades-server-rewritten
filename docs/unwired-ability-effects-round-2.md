# The ability effects the server STILL ignores (round 2)

`plan-unwired-ability-effects.md` did this exercise once and closed out twelve
fields. This is the same scan re-run on 2026-09-16, after the Ward / Reckless
Fury / maneuver work, and it is **not** a restatement of that document: the
field set has grown, and several of the fields it lists were never in scope the
first time.

## How this list was produced (and how to trust it)

Every `AbilityField` variant (59 of them) was grepped against the whole combat
engine — both as `AbilityField::X` and through its named accessor, since about
half are reached one way and half the other. A field counts as READ if either
form appears.

The scan carries a **control**: seven fields known to be read (`ParalyzeDuration`,
`Duration`, `DodgeDuration`, `DelayDuration`, `BonusResistance`,
`ResistanceBonus`, `ChannelMaxLength`) are asserted to come back as "read". An
earlier version of this scan reported 26 unread and failed that control —
`ParalyzeDuration` is read via `r.paralyze_duration()` and the scan missed it.
Without the control the list would have sent someone to wire a field that
already works.

Each remaining field was then surveyed across the shipped ability table, because
a field that no *player* ability ships a non-zero value for is not worth wiring.

## Result: 25 unread, but only 11 matter

| field | player abilities that ship it (rank-1 value) |
|---|---|
| `magickaRegenerationBonus` | Magicka Surge (81.84/s) |
| `noMagickaRegenDuration` | Magicka Surge (10 s) |
| `cooldownReductionBonus` | Magicka Surge (4) — **no rendered unit**; seconds vs multiplier unverified |
| `staminaCostPerSecond` | Consuming Inferno (51.81) |
| `healthCostPerSecond` | Consuming Inferno (31.11) |
| `numberOfBolts` | Thunderstorm (3, over a 9 s `duration` — the 3 s interval is DERIVED, not authored) |
| `selfDamagePercent` | Wall of Fire / `Firewall` (0.2) |
| `poisonEffectIncrease` | Venom Strikes (0.08) |
| `weaponDelay` | Echo Weapon (0.5 s) |
| `maximumHealthRestored` | ~~Adrenaline Dodge~~ **WIRED** |
| `maximumMagickaRestored` | ~~Renewing Dodge~~ **WIRED** |
| `maximumCooldownReduction` | ~~Focusing Dodge~~ **WIRED** |

The other 13 (`conversionAmount`, `damageFactor`, `effectType`,
`extraDamageFactor`, `healthDamagePerSecond`, `linkDuration`,
`magickaDamagePerSecond`, `staminaDamagePerSecond`,
`statusApplicationDamageType`, `statusApplicationMultiplier`,
`poisonDurationIncrease`, `projectileSpeed`, plus the all-zero cases) are
**enemy-only or zero at every rank on every player ability**. `projectileSpeed`
was deliberately left out the first time round as client-side presentation, and
still should be. `poisonDurationIncrease` is zero on all 13 Venom Strikes ranks
despite a loc string existing for it — the data, not an omission.

## Structural gaps that are not a single field

These need a mechanism, not a lookup, which is why they are not in the table:

* **Wall of Fire** — needs a persistent zone that reacts to attacks passing
  through it (93.33 per attack, not per second) plus 20% self-damage. There is
  no "zone" concept in the engine.
* **Thunderstorm** — needs three scheduled bolts over nine seconds. The channel
  scheduler exists (`ActiveChannel`) and is the obvious hook, but a bolt is not
  a channel tick: the interval is derived, not authored.
* **Echo Weapon** — needs delayed echo attacks, which nothing models.
* **Blizzard Armor** — `vulnerableDamageTypes` [Fire] needs a per-type
  *vulnerability* on the fighter; only resistances exist. Already flagged in the
  round-1 doc and still true.
* **Multi-hit maneuvers** — Quick Strikes ("a quick combo of **two** strikes"),
  Piercing Strikes and Recovery Strikes. **The hit count is not in the data at
  all**: `QuickStrikesAbility`, `PowerAttackAbility` and `RecoveryStrikesAbility`
  declare zero extra serialized fields, and the count lives in the animation clip
  and `AbilityDoManeuver`'s execution steps, neither of which is in `dump.cs`
  (no method bodies). Only the description text asserts "two". Wiring a count
  means choosing one, so it wants a capture, not a guess.
* **Shield requirement on the four bashes** — there is no `requiresShield`
  field anywhere in the file; it lives in `ShieldBashAbility.CanBeCast`.
* **Dodge proximity curve** — the three `maximum*Restored` values are CAPS. The
  "closer to being hit, the more you recover" scaling is unauthored, so the
  wiring grants the cap on a dodge that connects. That is a modelling choice and
  is commented as one at the call site.

## Perks: narrower than their descriptions, structurally

All 20 perks serialize exactly one field, `bonusValue` (the only exceptions are
`MatchingSetPerk._slotsToCheck` and the enemy-only `AtronachPowerPerk`). **A
perk's scope is defined only by its description sentence — there is no data to
narrow it.** So these are judgement calls, not lookups:

* **Mettle** — "**Abilities** are {0}% more effective while Health is critical."
  Unqualified: maneuvers *and* spells. Threshold is the global
  `criticalHealthThreshold = 35`, not on the perk.
* **Maximum Power** — "**Spells** are {0}% more effective when Magicka is full."
  "More effective" is broad; it should scale every scalar a spell produces, not
  damage alone.
* **Augmented Flames / Frost / Poison / Shock** — **flat damage points, not
  percentages**, and the description carries **no source restriction** — not
  limited to spells, not to enchantments.
* **Enchantment Synergy** — "**Stacked** enchantments are {0}% more effective."
  There is no stack-count, minimum-stack or family field anywhere; "stacked" is
  defined in the enchantment system, not here.

## The light combo ramp

Not a field, but the largest open question in the damage model. See the doc
comment on `LIGHT_COMBO_CAP` in `tables.rs`: a corpus measurement and a directly
reproduced retail chain disagree about whether the ceiling is 2.23 or 4.12, and
one of the two is mislabelled. Resolving that is its own task and should not be
folded into a fix for something else.
