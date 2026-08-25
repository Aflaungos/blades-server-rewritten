#!/usr/bin/env python3
"""Prove the quest/event tests are RED without their fixes.

A test that passes with AND without the change it claims to cover is worth nothing,
and this repo has been bitten by that more than once. So for each fix in the quest /
game-event work, this script applies a mutation that undoes exactly that fix, runs
the one test that is supposed to catch it, and requires the test to FAIL with an
ASSERTION (a compile error is rejected as weak evidence — it proves the code depends
on the change, not that the test observes it). Then it restores the file.

    python3 script/verify_quest_tests_are_red.py

Every mutation must print `RED ok`. Add a row here whenever you add a quest test.

Note: files are `utime`d after restore. Without that, `shutil.move` puts back an
mtime older than the mutated build and cargo happily reuses the broken artifacts,
which makes every later mutation look like a compile error.
"""
import subprocess, shutil, os, sys

R = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

MUTATIONS = [
    # (label, file, old, new, test filter, package)
    ("QuestStatus loses Completed",
     "blades_lib/src/user_data/quest.rs",
     "pub enum QuestStatus {\n    Active,\n    Completed,\n}",
     "pub enum QuestStatus {\n    Active,\n    #[serde(rename = \"NotWhatTheClientSends\")]\n    Completed,\n}",
     "the_client_report_that_400ed_every_time_now_parses", "server"),

    ("quest_rewards read through read_json again (a _meta key empties the table)",
     "server/src/static_loader.rs",
     "        read_uuid_map(&dir.join(\"quest_rewards.json\"));",
     "        read_json(&dir.join(\"quest_rewards.json\"));",
     "an_ordinary_quest_pays_from_the_template_keyed_table", "server"),

    ("reward lookup keyed on the row id first, template second",
     "server/src/quest.rs",
     "    if let Some(r) = static_data.quest_rewards.get(&quest.gld_quest_id) {\n        return r.clone();\n    }\n    if let Some(r) = static_data.quest_rewards.get(&quest_id) {\n        return r.clone();\n    }",
     "    if let Some(r) = static_data.quest_rewards.get(&quest_id) {\n        return r.clone();\n    }\n    #[allow(unreachable_code)]\n    {\n        return RewardGrant::default();\n    }",
     "an_ordinary_quest_pays_from_the_template_keyed_table", "server"),

    ("event quest pays a flat reward instead of the milestone ladder",
     "server/src/quest.rs",
     "        let completion = *server_state\n            .event_quest_completions\n            .entry(quest_id)\n            .or_insert(0) as usize;",
     "        let completion = 0usize;\n        let _ = &mut server_state.event_quest_completions;",
     "an_event_quest_pays_its_milestones_in_order", "server"),

    ("gldQuestId left equal to the instance id",
     "server/src/quest.rs",
     "        quest.gld_quest_id = def.quest_id;",
     "        quest.gld_quest_id = quest_id;",
     "an_event_quest_instance_id_is_not_its_template_id", "server"),

    ("game events ignore recurrenceInterval (the old day-slice)",
     "blades_lib/src/features/game_events.rs",
     "        let start = self.instance_start_at_or_before(now)?;\n        (now < start + self.window_secs()).then_some(start)",
     "        let _ = self.instance_start_at_or_before(now);\n        Some(now.div_euclid(86_400) * 86_400)",
     "an_event_is_open_only_inside_its_recurring_window", "blades_lib"),

    ("warning window listed events that are already open",
     "blades_lib/src/features/game_events.rs",
     "            let start = def.next_instance_start_after(now_secs)?;\n            (start - now_secs <= lead_secs).then(|| def.instance(start))",
     "            let start = def.instance_start_at_or_before(now_secs)?;\n            let _ = lead_secs;\n            Some(def.instance(start))",
     "warning_lists_what_opens_within_the_lead_and_nothing_else", "blades_lib"),

    ("GAME_EVENT rows routed into quests[] like any other row",
     "server/src/quest.rs",
     "        let is_event = matches!(info.r#type, blades_lib::user_data::QuestType::GameEvent);\n        if is_event && !open_event_instances.contains(&quest_id) {\n            continue;\n        }",
     "        let is_event = false;\n        let _ = &open_event_instances;",
     "event_rows_are_routed_to_the_event_array", "server"),

    ("objectives answer under `quest` for an event quest too",
     "server/src/quest.rs",
     "    if is_event {\n        (None, Some(quest))\n    } else {\n        (Some(quest), None)\n    }",
     "    let _ = is_event;\n    (Some(quest), None)",
     "an_event_quest_answers_under_its_own_key", "server"),
]

fails = []
for label, rel, old, new, test, pkg in MUTATIONS:
    path = os.path.join(R, rel)
    backup = path + ".redbak"
    shutil.copy(path, backup)
    src = open(path).read()
    if old not in src:
        print("SKIP (anchor not found): %s [%s]" % (label, rel))
        os.remove(backup)
        fails.append(label + " (anchor missing)")
        continue
    open(path, "w").write(src.replace(old, new, 1))
    os.utime(path, None)
    r = subprocess.run(
        ["cargo", "test", "--locked", "-p", pkg, test],
        cwd=R, capture_output=True, text=True)
    out = r.stdout + r.stderr
    shutil.move(backup, path)
    os.utime(path, None)
    compiled = "could not compile" not in out and "error[E" not in out
    ran = "test result:" in out
    if r.returncode == 0:
        print("!! STILL GREEN: %s -> %s" % (label, test))
        fails.append(label)
    elif not compiled:
        print("!! COMPILE ERROR (weak evidence): %s -> %s" % (label, test))
        import re as _re
        for m in list(_re.finditer(r"error\[E\d+\][^\n]*", out))[:3]:
            print("      ", m.group(0))
        if "could not compile" in out:
            for l in out.splitlines():
                if "could not compile" in l:
                    print("      ", l.strip())
                    break
        fails.append(label + " [compile error, not an assertion]")
    elif not ran or "FAILED" not in out:
        print("!! test did not run: %s -> %s" % (label, test))
        fails.append(label + " [did not run]")
    else:
        line = [l for l in out.splitlines() if "panicked at" in l or "assertion" in l]
        print("RED ok: %-60s -> %s" % (label, test))
        for l in out.splitlines():
            if l.strip().startswith("the ") or "must" in l or "got" in l:
                pass
        idx = out.find("panicked at")
        if idx >= 0:
            print("      %s" % out[idx:idx+200].splitlines()[1].strip()[:150])

print()
if fails:
    print("PROBLEMS:", fails)
    sys.exit(1)
print("every mutation turned its test red")
