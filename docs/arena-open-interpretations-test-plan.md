# Arena: what is still guesswork, and one evening to settle it

Everything reimplemented so far that a capture could prove, is proved. What remains is
a list of **interpretations** — places where the data admits two readings, or where the
value came from the il2cpp dump and no captured frame confirms the client acts on it.

This document is (1) that list, and (2) a test plan that settles most of it in **one
evening of real games**, because a human's eyes are the instrument we are missing. The
server can tell us what it sent; only a player can tell us what the client did with it.

---

## Part 1 — the open interpretations

Ranked by how much they affect a player, and by how cheap they are to settle.

| # | Question | Two readings | Evidence today | Cost if wrong |
|---|---|---|---|---|
| **A** | Does the enemy's swing animate? | the `Charging` wind-up was the missing piece / something else is still missing | 593/593 retail swings begin with gmid 45; never tested on a device | the headline bug is still open |
| **B** | `Blind` = 8? | 8 is right / the fog needs something else | dump.cs:609805; **zero** capture frames | Blind does nothing, silently |
| **C** | `ElementalStormArmor` = 16? | 16 is right / wrong id | dump.cs:609812; **zero** capture frames | shield invisible (still mitigates) |
| **D** | gmid 41 propId 10 | `OptimalBlockAllowed` / something else | dump names it; captures show true 231 / false 17 with no pattern | perfect-parry feedback wrong |
| **E** | Do maneuvers deal weapon damage? | yes, Middle-side weapon swing / an ability value we haven't found | s506 differential says weapon; no device check | maneuvers feel wrong |
| **F** | dps-only spells (Frostbite) | one lump of `dps × 5 s` / ticked over 5 s | no capture of a Frostbite cast | 178 damage arrives wrong |
| **G** | Does block piercing beat an OPTIMAL block? | only weakens LATE / pierces optimal too | optimal-zero is capture-pinned; piercing is not | Skullcrusher underperforms |
| **H** | Weakness statuses 100-103 | propId 5 carries the type / propId 12 does | 58/58 frames fit **both** readings | can't wire weaknesses at all |
| **I** | `_vulnerableDamageTypes` | shield opens an elemental weakness / cosmetic | field exists, unread | armor too strong |
| **J** | gmid 52 propId 10 `Direction` | (0,0) is fine / the clip needs a real vector | (0,0) in 78 % of retail frames | swings may not animate |

**A, B and C are the evening's priority.** A is the bug you reported and it is untested;
B and C are the only two values in the whole system that came from the dump with no
capture behind them.

---

## Part 2 — the constraint that shapes everything

**Most of these need a specific ability equipped**, and that is the scheduling problem,
not the server work:

| Need | Ability | Tests |
|---|---|---|
| a blind spell | **Blind** | B |
| an armor spell | **FirestormArmor** / Blizzard / Tempest | C, I |
| a block-piercing maneuver | **Skullcrusher** | G |
| a dps-only spell | **Frostbite** | F |
| any maneuver | QuickStrikes / PiercingStrikes | E |
| nothing special | — | A, D, J |

**Before the evening starts**, tell me which of those your character can actually
equip. If Blind isn't available on any character you can play, B is untestable that
night and I'll re-plan around it rather than burn a round discovering it.

---

## Part 3 — the cycle

```
  I prepare the server  →  you play one specific thing  →  you say "done"
        ↑                                                       ↓
  I prepare the next  ←  I read the logs + your report  ←  I analyse
```

**Your side is deliberately small.** Each round is one or two sentences of instruction
and one observation. You never read a log.

**My side per round:** read the arena log for what the server sent, pair it with what
you saw, decide the verdict, and either ship the next hypothesis or move on. A server
change is a merge to `main` → ghcr build → the deploy timer picks it up in **~5 min**.

**Deploy discipline:** merging kicks anyone mid-match, so I check for live peers first.
Between rounds you'll be in the menu, so the window is free. Expect **~6 min** of dead
time per server change; rounds needing no change are back-to-back.

---

## Part 4 — the rounds

### Round 1 — A, D, E, J (no ability requirement, no server change)

**You:** play one full match against the AI. During it: (1) swing normally several
times, (2) hold block until the opponent hits you, (3) use any maneuver once.
Then tell me:
- did the **opponent's** weapon visibly wind up and swing, or did damage just appear?
- did your **own** charge circle appear while holding the attack button?
- did the maneuver deal damage?

**I check:** `combat: slot N maneuver … → weapon damage X`, and the gmid 45 → 52 → 43 →
44 ordering in the state broadcasts.

| You saw | Verdict |
|---|---|
| enemy winds up and swings | **A resolved** — the wind-up was the fix |
| damage appears, no animation | **A open** — next probe is the il2cpp `PlayerChargingState` animator hookup, not more captures |
| no charge circle either | the whole family is being dropped — suspect the frame shape, not the ids |
| maneuver dealt 0 | **E wrong** — revert to an ability-value reading |

That last row matters: if your own charge circle is missing too, the problem is not
Blind or armor ids, it is the family, and B/C become untestable noise. **That is why
this round is first.**

### Round 2 — B (needs Blind; no server change)

**You:** equip Blind. Land it on the opponent. Tell me whether **your screen** fogs
green, and whether an opponent set on fire stays visible through it.

**I check:** `combat: slot N BLINDED Xs (hit … >= threshold)`.

- log line + fog → **B confirmed**, `Blind = 8` is right.
- log line + no fog → the id is wrong or the client needs more. I try `StatusEffectType`
  neighbours from the same dump block, one per round.
- no log line → the damage never cleared `_damageToCauseBlind`. I lower the gate for one
  round to separate "threshold too high" from "wrong id".

### Round 3 — C and I (needs an Armor spell; no server change)

**You:** cast FirestormArmor, then let the AI hit you several times. Tell me: any shield
visual? Does incoming damage visibly drop?

**I check:** `combat: slot N storm-armor shield +116.0`, and the damage lines before/after.

- visual + damage drops ≈ half → **C confirmed** and the 0.50 absorb is right.
- damage drops, no visual → the mitigation is right, `16` is wrong. Mechanic works; I
  hunt the id separately without blocking you.
- damage does not drop → the pool isn't being consumed; that is a server bug and I fix
  it before continuing.

Then, same round: cast it and take **fire** damage specifically. If fire hurts *more*
than before, **I is real** and the shield opens an elemental weakness.

### Round 4 — G (needs Skullcrusher; ONE server change)

**You:** have the AI block, then hit it with Skullcrusher. Twice: once while it blocks
early (optimal), once late.

**I check** the block flags and totals in the damage lines.

If optimal-block damage is unchanged, that is consistent with the current reading (LATE
only). To test the other reading I ship a one-line change letting piercing reduce an
optimal block, and you repeat. Whichever produces a plausible number wins — this one is
a judgment call, not a proof, and I will say so in the result.

### Round 5 — F (needs Frostbite; ONE server change)

**You:** cast Frostbite. Tell me whether the damage arrives **all at once** or **ticks**.

Your eyes settle this directly: the HP bar either drops once or in steps. If it should
tick, I ship the DoT version and you re-cast.

### Not this evening — H

The weakness ambiguity (propId 5 vs propId 12) **cannot be settled by playing**, because
we don't send weaknesses at all — there is nothing to observe. It needs a retail capture
of a weakness being applied, and our corpus has 58 frames that fit both readings
equally. Leave it. If you ever find a session where a weakness was cast, that is the
unlock.

---

## Part 5 — what I need from you, total

1. **Before we start:** which of Blind / an Armor spell / Skullcrusher / Frostbite you
   can equip.
2. **Per round:** do the one thing, then one sentence of what you saw.
3. Nothing else. No logs, no SQL, no timing.

## Part 6 — realistic shape of the evening

Round 1 is the gate and needs no server change — if the animation family is broken,
we spend the evening on that instead and B/C wait. If it passes, rounds 2 and 3 are
also change-free, so the first three rounds are back-to-back: **maybe 20 minutes** of
play. Rounds 4 and 5 each cost one deploy (~6 min) and only if the first result is
ambiguous.

Best case the evening answers **A, B, C, D, E, I and J** and narrows F and G. H stays
open by nature.

## Part 7 — the rule for the night

If a round's result is ambiguous, I say so and we move on rather than re-running it
three times. An honest "we could not tell" is a result, and the failure mode this
project keeps hitting is a plausible reading nobody verified — the packed-stats field
order, the maneuver damage path, and the swing with no wind-up were all exactly that.
The point of the evening is to convert guesses into knowledge, not to collect
confirmations.
