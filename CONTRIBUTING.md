# Contributing

Thanks for wanting to help. This project reimplements the server for *The
Elder Scrolls: Blades*, whose official servers shut down on 2026-06-30, so that
the game remains playable and its protocol stays documented.

## Licence and the CLA

The project is AGPL-3.0. Contributions are accepted under that licence, and
before a first pull request can be merged you need to accept the Contributor
Licence Agreement in [`CLA.md`](CLA.md).

**You keep the copyright in your work.** The CLA is a licence grant, not an
assignment — clause 4 leaves you free to use, publish and relicense your own
contribution however you like, including in other projects.

**What it adds beyond the AGPL, stated plainly:** it lets the maintainer
relicense the project — including under a proprietary licence, or by handing
it to another party — **without asking you.** That is asymmetric. You are
granting a right you do not get back over anyone else's work, and you should
decide with that in front of you rather than discover it later.

Two reasons it is asked for:

1. **This reimplements a commercial game's server.** If a rights-holder ever
   objects, one identifiable person needs the standing to respond, settle, or
   hand the project over. Under a plain inbound-equals-outbound arrangement,
   doing any of that would require the agreement of every contributor who ever
   touched the file in question — and a single unreachable or pseudonymous
   account is enough to make it impossible.
2. **A preservation project should not be able to paint itself into a corner.**
   Linux cannot leave GPLv2 even if it wanted to, because it has no such
   agreement. Whether that turns out to matter here is unknowable now, which is
   exactly why it is cheaper to ask at the start than to chase signatures later.

If that is not a trade you want to make, say so — it is a reasonable position,
and it is better raised before you write code than after. `docs/LICENSING.md`
sets out the reasoning in full. Note that `CLA.md` section 5 runs the other
way: it is what **you** promise — that the work is yours to give, and that you
have not folded in game-client data or third-party code without saying so.

Accepting is one comment on your pull request:

    I have read the CLA and I hereby accept its terms.

Once, not per pull request. It is retroactive to anything you contributed
earlier, so contributing first and accepting later is fine. A workflow checks this and will
tell you on the pull request if it is outstanding; once you accept, it records
you in [`CLA-ACCEPTANCES.md`](CLA-ACCEPTANCES.md).

Please also sign your commits off:

    git commit -s -m "your message"

which adds `Signed-off-by: Your Name <your@email>`, certifying the
[DCO 1.1](https://developercertificate.org/) — that you wrote the patch or
have the right to submit it. Use a real name and a working email. If a pull
request is missing sign-offs:

    git rebase --signoff origin/main && git push --force-with-lease

## Before you write extracted game data into the repo

`deploy/static/*.json` and `server/src/arena/combat/gamedata.rs` contain data
extracted from the game client. That data is **not** ours and is not covered by
the AGPL — see `docs/LICENSING.md`.

If your change adds or regenerates such data, say so in the pull request and
say where it came from (which capture, which asset bundle, which extractor
script). Do not add third-party code or assets from any other source without
flagging it.

Findings that came from a capture should cite it. The existing code does this
throughout — a capture row id, a session number, a `dump.cs` line — and it is
the reason the protocol claims in this repo are auditable rather than folklore.
Please keep that up.

## Practical

- `cargo test` should pass. Combat and protocol changes should come with a test;
  where a real captured session covers the behaviour, replay it rather than
  hand-rolling a fixture.
- Where retail behaviour is unknown and you are inventing something plausible,
  label it as invented — the codebase distinguishes measured facts from
  reasonable guesses on purpose.
- Small, focused pull requests get reviewed faster than large ones.

## Questions

Open an issue. Uncertainty about the protocol is normal and worth writing down
even when unresolved.
