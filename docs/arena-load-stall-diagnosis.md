# The client load stall: what the captures actually say

Status: **root cause narrowed to a stub character; the writer is not yet identified.**
Three hypotheses tested and killed along the way — they are recorded here so nobody
re-runs them.

## The symptom

The client completes its twelve boot requests and then asks for nothing further. A
boot that succeeds goes on to `catalogoverrides/globalshop`,
`towns/current/rewards/current`, `announcements` and `matches/create`. A stalled one
sends none of those. The client has everything it asked for and stops deciding.

## The discriminator: response SIZE, not status

Every response in a stalled boot is **HTTP 200**. Over 2,030 recorded
`GET /characters/{id}` responses for one character, the status is 200 in every
single one — and the size is **bimodal**:

| | stalled boots | working boots |
|---|---|---|
| `/characters/{id}` | ~980 B | ~249,300 B |
| `/quests` | 24,064 B | 51,864 B |
| `/challenges` | 16,011 B | 44,391 B |
| `inventories/current` | 14,409 B | 45,495 B |
| `/abysses/current` | **14 B** | 1,050 B |

The same character id, within the same hour, is served either the full 249 KB
character or a ~980-byte stub.

## What the stub is

Diffing the two bodies, the stub is missing **exactly** seven keys and nothing else:

```
abilities  avatarIconId  completedQuests  equippedAbilities
globalShopOffers  loadoutProfiles  pvpSeasonHistory
```

Those are **precisely** the seven fields that are `Value::Null` in
`CompleteCharacter::default()` (`blades_lib/src/user_data/mod.rs:68`), each carrying
`#[serde(default, skip_serializing_if = "Value::is_null")]`. So the stub is not a
partially-written character — it is shaped like a **freshly defaulted** one.

A client handed a character with no abilities, no equipped abilities and no loadout
profiles has nothing to build a loadout from, which is a plausible reason to stop at
the loading screen.

## Hypotheses tested and killed

Recorded so they are not repeated.

1. **"The `town` column is NULL for 34 of 74 characters, so the town load stalls."**
   Dead: `get_town` (`server/src/town.rs:72`) falls back to `default_town.json` on
   any miss and never errors. A NULL town cannot stall that endpoint. The NULL-town
   set turned out to be a *proxy* for "barely-populated character" — that bucket
   averages 5,434 B of `data` against 34,961 B for the rest, a 6× split across
   every column.

2. **"We omit keys retail always sent, so the client rejects the response."**
   Dead, and it matters — the opposite change would have made us *less* faithful.
   Across 150 retail character responses from `api_captures`, retail omits the same
   keys: `loadoutProfiles` present in only **12%**, `globalShopOffers` **56%**,
   `pvpSeasonHistory` **79%**, `abilities`/`equippedAbilities`/`avatarIconId`
   **97%**, `completedQuests` **99%** — and **never empty when present**. Omit-when-
   empty is retail's own rule. Our serialization is correct.

3. **"The economy write-back drops fields the struct cannot represent."**
   `CharacterDbEntryEconomy` (`server/src/models.rs:102`) is an `AsChangeset` that
   round-trips the whole `character` JSONB through `CompleteCharacter` on every
   abyss / challenge / shop write. That is a real read-modify-write, and it is the
   shape of bug this repo has hit before. But all seven fields **are** on the
   struct, typed `Value` with `serde(default)`, so the round-trip preserves them.
   Dead.

## Two measurement traps found in the capture data

Both invalidate naive counting, and both bit this investigation:

- **`session_id` is NULL for all 938,280 rows** of `newblades_captures`. Any query
  grouping boots by session silently returns nothing. Group by `(user_id, hour)`;
  `user_id` is set on 887,673 rows and `route`/`src_ip` on all of them.
- **Rows are duplicated ~13×** by the pcap re-ingest — the same request appears
  thirteen times under one identical timestamp. Every count in this document is
  therefore a **ratio**, never an absolute.

Also: `route` has only three distinct values (`redirect`, `analytics-drop`,
`announce-stub`), so it cannot resolve a boot sequence. Use `url`.

## Where this stands

- 34 of 74 characters in production are currently in the stub state.
- One of them is named `Imported` at level 1 — the signature of a transfer that did
  not complete.
- A character that stalled through June and July is **full now**, having been
  re-transferred.

That points at the same root cause as the known one-character-per-account
destruction: a fresh `CompleteCharacter::default()` replacing a real character.
`create_characters` (`server/src/character.rs:111`) builds exactly that default.

**Not yet established: what writes the stub over a full character.** Confirming it
needs a capture of the write, and capture has been off since **2026-08-01** — the
ten capture units are idled, so the currently-reported stall is not recorded
anywhere. Re-enabling them is the prerequisite for closing this.
