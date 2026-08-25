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

## The writer — FOUND (2026-08-25)

`web/app/play/AutoTransfer.tsx`. When a **complete** character failed to import,
`run()` fell through to:

```ts
const r = await post(templateAvailable ? { template: true } : { fresh: true });
```

**Automatically, on page load, over a transient failure, with no confirmation.**
With no starter template configured `fresh` resolves to the arena server's bare
`CompleteCharacter::default()`, and since `characters` carries `UNIQUE(user_id)` and
`import_character` "overwrites all four payload columns of the existing row", that
level-1 shell lands on top of the real character.

That accounts for every observation above:

- the stub is `CompleteCharacter::default()`-shaped — because it *is* one;
- it alternates with the full character within an hour — the player reloads
  `/play`; sometimes the import succeeds (249 KB), sometimes it fails (980 B);
- it hits imported characters specifically — they are the ones with a capture to
  re-import on every fresh browser session (the only guard was `sessionStorage`);
- 34 of 74 characters sit in the shell state, one of them named `Imported` at
  level 1.

The name-mismatch branch immediately above already refused this substitution, with
the comment *"which would OVERWRITE their arena character with a level-1 one
(tracker #28)"*. The plain-failure path never had the same guard applied.

Fixed in `dethele-com/newblades-project` PR #183: report the failure, change
nothing, and offer the fallback as a button the player presses themselves.

**Still open:** the 34 characters already reduced to shells are not repaired by that
fix — it only stops new ones. Repairing them means re-transferring from each
player's own capture.
