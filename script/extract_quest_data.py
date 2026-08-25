#!/usr/bin/env python3
"""Extract the quest / game-event static data from the pre-shutdown capture corpus.

Regenerates three files under ``deploy/static``:

* ``game_events.json``    — the event library (eventId, questId, recurrence, window).
* ``event_quests.json``   — per event-quest TEMPLATE (gldQuestId): objective ids, wire
                            ``version``, the five milestone ``rewards`` and the
                            ``finalReward``.
* ``quest_rewards.json``  — completion reward per quest, keyed by the TEMPLATE quest id.

Why this script exists
----------------------
``quest_rewards.json`` used to be keyed by whatever ``questId`` appeared in the
``/complete`` URL. For an event quest that id is a *per-character instance* id, so 122
of its 148 keys matched no quest definition and could never be looked up again. This
script resolves every instance id back to its ``gldQuestId`` (the template) using the
``gameEventQuests[]`` arrays in the same corpus, which is what makes the table usable.

Everything here is CAPTURE-DERIVED. Nothing is authored. Where a value varies across
captures the most-frequently-observed variant is taken and the alternatives are recorded
in ``_meta.variants`` so the choice is auditable rather than hidden.

Only RETAIL traffic is read (``https://blades.bgs.services/``). Our own emulator's
traffic (``http://127.0.0.1:8087/``) is in the same table and is excluded on purpose:
feeding our own answers back in as "ground truth" would be circular.

Usage (on the capture host, read-only):

    sudo python3 extract_quest_data.py \
        --db /var/lib/newblades/db/blades.db \
        --archive /var/lib/newblades/db/blades-archive.db \
        --out /tmp/quest-static
"""

import argparse
import collections
import gzip
import json
import os
import sqlite3
import sys

RETAIL_PREFIX = "https://blades.bgs.services/%"


def decode(blob):
    if blob is None:
        return None
    if isinstance(blob, str):
        blob = blob.encode()
    if blob[:2] == b"\x1f\x8b":
        try:
            blob = gzip.decompress(blob)
        except Exception:
            return None
    try:
        return json.loads(blob)
    except Exception:
        return None


class Corpus:
    def __init__(self, db, archive):
        self.con = sqlite3.connect("file:%s?mode=ro" % db, uri=True)
        if archive and os.path.exists(archive):
            self.con.execute("ATTACH DATABASE ? AS arch", ("file:%s?mode=ro" % archive,))
            self.bodies = ("LEFT JOIN arch.capture_bodies b ON b.capture_id = c.id",
                           "COALESCE(c.request_body, b.request_body)",
                           "COALESCE(c.response_body, b.response_body)")
        else:
            self.bodies = ("", "c.request_body", "c.response_body")

    def rows(self, suffix):
        join, rq, rs = self.bodies
        sql = ("SELECT c.id, c.url, c.user_id, %s, %s FROM api_captures c %s "
               "WHERE c.url LIKE ? AND c.url LIKE ? AND c.method = 'POST' ORDER BY c.id"
               % (rq, rs, join))
        for cid, url, uid, req, res in self.con.execute(sql, (RETAIL_PREFIX, suffix)):
            yield cid, url, uid, decode(req), decode(res)


def most_common(counter):
    """(value, n, [alternatives]) for a Counter keyed by canonical JSON."""
    ordered = counter.most_common()
    best_key, best_n = ordered[0]
    alts = [{"n": n, "value": json.loads(k)} for k, n in ordered[1:]]
    return json.loads(best_key), best_n, alts


def canon(value):
    return json.dumps(value, sort_keys=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--archive", default="")
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    c = Corpus(args.db, args.archive)

    # ---------------------------------------------------------------- events
    # /gameevents is the event library on the wire. Each response repeats the
    # currently-open instances; the template behind them is (eventId, questId,
    # recurrence) and the instance window gives instanceDurationSecs.
    events = {}
    ev_seen = collections.Counter()
    for cid, url, uid, req, res in c.rows("%/gameevents"):
        if not isinstance(res, dict):
            continue
        for e in res.get("gameEvents", []) or []:
            iid = e.get("gameEventInstanceId") or ""
            event_id = iid.split("::")[0] if "::" in iid else None
            if not event_id or not e.get("questId"):
                continue
            ev_seen[event_id] += 1
            duration = (e.get("endTimeSecs") or 0) - (e.get("startTimeSecs") or 0)
            rec = e.get("recurrence") or {}
            prev = events.get(event_id)
            entry = {
                "eventId": event_id,
                "questId": e["questId"],
                "important": bool(e.get("important")),
                "instanceDurationSecs": duration if duration > 0 else rec.get("durationSecs", 0),
                "recurrence": {
                    "recurrenceType": rec.get("recurrenceType", "daily"),
                    "startTimeSecs": rec.get("startTimeSecs", 0),
                    "durationSecs": rec.get("durationSecs", 0),
                    "recurrenceInterval": rec.get("recurrenceInterval", 0),
                },
            }
            if prev is None:
                events[event_id] = entry
            elif canon(prev) != canon(entry):
                prev.setdefault("_variants", [])
                if canon(entry) not in [canon(v) for v in prev["_variants"]]:
                    prev["_variants"].append(entry)

    # ------------------------------------------------- event-quest templates
    # `gameEventQuests[]` in /quests (and `gameEventQuest` in /objectives) is the
    # only place the milestone rewards appear. questId there is a per-character
    # INSTANCE; gldQuestId is the template. This is the mapping the old extractor
    # was missing.
    inst2gld = {}
    tmpl = collections.defaultdict(lambda: {
        "objective_ids": set(),
        "versions": collections.Counter(),
        "rewards": collections.Counter(),
        "final": collections.Counter(),
        "event_ids": collections.Counter(),
        "instances": set(),
    })

    def absorb(q):
        qid, gld = q.get("questId"), q.get("gldQuestId")
        if not qid or not gld or qid == gld or q.get("type") != "GAME_EVENT":
            return
        inst2gld[qid] = gld
        t = tmpl[gld]
        t["instances"].add(qid)
        t["objective_ids"] |= set((q.get("objectiveStatuses") or {}).keys())
        if q.get("version") is not None:
            t["versions"][q["version"]] += 1
        if q.get("rewards"):
            t["rewards"][canon(q["rewards"])] += 1
        if q.get("finalReward"):
            t["final"][canon(q["finalReward"])] += 1
        gid = (q.get("gameEventQuestData") or {}).get("gameEventInstanceId")
        if gid and "::" in gid:
            t["event_ids"][gid.split("::")[0]] += 1

    for cid, url, uid, req, res in c.rows("%/quests"):
        if not isinstance(res, dict):
            continue
        for q in res.get("gameEventQuests", []) or []:
            absorb(q)
    for cid, url, uid, req, res in c.rows("%/objectives"):
        if isinstance(res, dict) and isinstance(res.get("gameEventQuest"), dict):
            absorb(res["gameEventQuest"])

    event_quests = {}
    for gld, t in sorted(tmpl.items()):
        if not t["rewards"]:
            continue
        rewards, rn, ralt = most_common(t["rewards"])
        final, fn, falt = (None, 0, [])
        if t["final"]:
            final, fn, falt = most_common(t["final"])
        event_quests[gld] = {
            "gldQuestId": gld,
            "version": t["versions"].most_common(1)[0][0] if t["versions"] else 0,
            "objectiveIds": sorted(t["objective_ids"]),
            "rewards": rewards,
            "finalReward": final,
            "eventIds": [e for e, _ in t["event_ids"].most_common()],
            "_meta": {
                "instancesObserved": len(t["instances"]),
                "rewardsObservations": rn,
                "rewardsVariants": ralt,
                "finalRewardObservations": fn,
                "finalRewardVariants": falt,
            },
        }

    # ------------------------------------------------------- quest rewards
    # /complete pays a flat reward for a NORMAL quest. For a GAME_EVENT quest it
    # pays rewards[completionCount] instead (see event_quests.json), so event
    # templates are deliberately EXCLUDED here — a single fixed number would be
    # wrong for four of the five completions.
    #
    # The tiered `rewards[]` the client is shown and the reward `/complete`
    # actually pays are the SAME numbers in a different envelope: retail lists the
    # gem currency under `stackableItems` in `rewards[]` and under `currencies` in
    # the `/complete` body. So the payable form is collected here too, straight
    # from the observed `/complete` bodies, indexed by completion order — rather
    # than re-deriving which uuid is a currency.
    per_quest = collections.defaultdict(collections.Counter)
    payable = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))
    completions_seen = collections.Counter()
    currency_ids = set()
    for cid, url, uid, req, res in c.rows("%/complete"):
        if not isinstance(res, dict) or "reward" not in res:
            continue
        parts = url.rstrip("/").split("/")
        if len(parts) < 3 or parts[-3] != "quests":
            continue
        qid = parts[-2]
        currency_ids |= set((res["reward"].get("currencies") or {}).keys())
        if qid in inst2gld:          # an event-quest instance: tiered, not flat
            tier = completions_seen[qid]
            completions_seen[qid] += 1
            payable[inst2gld[qid]][tier][canon(res["reward"])] += 1
            continue
        per_quest[qid][canon(res["reward"])] += 1

    for gld, tiers in payable.items():
        if gld not in event_quests:
            continue
        observed = {}
        for tier, counter in sorted(tiers.items()):
            value, n, alts = most_common(counter)
            observed[str(tier)] = {"reward": value, "observations": n, "variants": alts}
        event_quests[gld]["payableRewards"] = observed
    for gld in event_quests:
        event_quests[gld].setdefault("payableRewards", {})

    quest_rewards = {}
    variants = {}
    for qid, counter in sorted(per_quest.items()):
        value, n, alts = most_common(counter)
        quest_rewards[qid] = value
        if alts:
            variants[qid] = {"chosenObservations": n, "alternatives": alts}

    # ------------------------------------------------------------------ write
    out = args.out
    events_list = sorted(events.values(), key=lambda e: e["eventId"])
    with open(os.path.join(out, "game_events.json"), "w") as f:
        json.dump(events_list, f, indent=1)
    with open(os.path.join(out, "event_quests.json"), "w") as f:
        json.dump({
            "_meta": {
                "description": "Event ('Sigil') quest TEMPLATES, keyed by gldQuestId. "
                               "Capture-derived from the gameEventQuests[] arrays of "
                               "pre-shutdown /quests and /objectives responses.",
                "authoritative": "objectiveIds, version, rewards[], finalReward — all "
                                 "verbatim from retail responses.",
                "derivation": "Where a template was observed with more than one reward "
                              "payload (retail rotated the event currency per season) "
                              "the most-frequently-observed payload is used and the "
                              "alternatives are kept under _meta.rewardsVariants.",
                "rewardModel": "The Nth completion of an event-quest instance pays "
                               "rewards[N]; the last one pays rewards[last] + finalReward. "
                               "Verified against 93 retail instances / 300+ completions.",
                "payableRewards": "The same milestone payouts as observed in the "
                                  "/complete BODY, keyed by completion index. Retail "
                                  "lists the gem currency under stackableItems in "
                                  "rewards[] (what the client displays) and under "
                                  "currencies in /complete (what the server grants); "
                                  "these are the granting form, taken verbatim.",
                "currencyItemIds": sorted(currency_ids),
                "templates": len(event_quests),
            },
            "templates": event_quests,
        }, f, indent=1)
    with open(os.path.join(out, "quest_rewards.json"), "w") as f:
        json.dump(quest_rewards, f, indent=1)
    with open(os.path.join(out, "quest_rewards_variants.json"), "w") as f:
        json.dump(variants, f, indent=1)
    with open(os.path.join(out, "instance_to_template.json"), "w") as f:
        json.dump(inst2gld, f, indent=1)

    print("game_events.json      %d events" % len(events_list), file=sys.stderr)
    print("event_quests.json     %d templates" % len(event_quests), file=sys.stderr)
    print("quest_rewards.json    %d quests (%d with >1 observed reward)"
          % (len(quest_rewards), len(variants)), file=sys.stderr)
    print("instance_to_template  %d instances" % len(inst2gld), file=sys.stderr)


if __name__ == "__main__":
    main()
