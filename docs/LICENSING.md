# Licensing

## Short version

| What | Licence |
|---|---|
| Source code written for this project | AGPL-3.0-only |
| Code inherited from the upstream fork | MIT (Marius DAVID) — retained, see `NOTICE` |
| Extracted game data (`deploy/static/`, `gamedata.rs`) | **Not ours, not licensed by us** |

## Why AGPL

This is a server. Under GPL-3.0, someone could run a modified copy as a public
service and never publish their changes, because they never "distribute" a
binary. AGPL-3.0 section 13 closes that: if you let users interact with a
modified version over a network, you must offer them the corresponding source.
For a game-server preservation project that is the whole point — improvements
should come back.

AGPL does not prevent forking. Anyone may fork. It requires that they publish
their changes under the same terms.

## Relationship to upstream

This project began as a fork of
`https://github.com/marius851000/blades-server-rewritten`, which is MIT.
MIT permits relicensing a derivative work under stricter terms provided the
original copyright notice and permission notice are retained — they are, in
`NOTICE`.

Two consequences worth being explicit about:

1. **Upstream is unaffected.** Marius's MIT grant is irrevocable. His
   repository remains MIT and anyone may still use it on those terms. Nothing
   here retroactively restricts his code.
2. **The gate is one-way.** MIT code can be merged into this AGPL project.
   Code written here cannot be contributed back to an MIT upstream without the
   author's separate permission. If upstream activity resumes and merging back
   matters, raise it before writing the code, not after.

## Third-party game data — read this before redistributing

This repository carries data this project did not author, so that the original
client can talk to this server. `NOTICE` is the authoritative statement; this
is the readable version of it. Concretely:

- `deploy/static/*.json` (33 files) and `server/data/static/*.json` (5 files) —
  about 154,000 lines: item and quest tables, shop catalogues, reward tables
  and recorded server responses.
- `server/src/arena/combat/gamedata.rs` — about 40,000 generated lines of
  weapon, armour, shield, ability and enchantment tables.

It is not one uniform body of content. Some is read from the client, some is
measurement recorded from the retail service while it ran, and some is a model
this project authored where retail's values existed only server-side and were
never observable. That last category is our own work and is covered by the
AGPL like any other file here. `NOTICE` sets out the three in full.

These contain Bethesda's own identifiers, localisation keys and tuning values
(`Items.Name.ChaurusArmor`, ability rank tables, ~8,600 asset UUIDs). **No
contributor to this project authored them, so no contributor can license
them.** The AGPL grant in `LICENSE` covers the source code of this project and
does not extend to these files. All rights in them remain with ZeniMax Media
Inc. / Bethesda Softworks LLC.

They are included because the game's servers were shut down on 2026-06-30 and
the client is unusable without them. That is an interoperability and
preservation purpose. It is not a copyright licence, and this document is not
a legal opinion — if you redistribute this repository, that is your call to
make.

If you would rather not carry the client-derived part, the extraction pipeline
is documented and much of it can be regenerated locally from a copy of the
client you own: see `blades-capture/reference/game-defs/README.md` and
`blades-capture/tools/ios/ASSET-EXPORT.md`.

Be aware this is **not** true of the whole set. The measured and authored
content cannot be regenerated from a client at all — the service it was
observed from no longer exists, and the authored models were never in the
client. Stripping the game-derived files leaves a server missing values, not
one that rebuilds them.

## Contributing

**Contribution terms are not settled yet, and this relicence does not settle
them.** Nothing in this document asks you to sign anything.

Until they are, treat a pull request as offered under the same AGPL-3.0 that
covers the rest of the project. If the terms end up needing something more
than that, contributors will be asked before anything they have already
written is treated differently.

This is a deliberate split: the licence the code is distributed under is one
decision, and the terms contributions are accepted under is another. Doing the
first should not quietly commit anyone to the second.

## Dependency licences

All current Rust dependencies are permissive (MIT and/or Apache-2.0), which is
compatible with AGPL-3.0 in this direction. Before adding a dependency, check
it is not GPL-2.0-only, which would be incompatible.
