#!/usr/bin/env python3
"""Prove every guild permission/join rule is RED without its fix.

A test suite that passes both with and without the code under test is not a test
suite, and this repo has been bitten by exactly that. So: for each mutation below
we break exactly ONE rule in `server/src/guild_policy.rs`, run the guild tests,
and record which fail. A mutation that breaks a real rule but leaves every test
green means that rule is untested, and the script exits non-zero.

Run it after touching guild_policy.rs:

    python3 script/mutation_test_guild_policy.py

It restores the original file on the way out, including on failure.
"""
import io, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "server/src/guild_policy.rs")

MUTATIONS = [
    ("MASTER gains kick+ban authority",
     "        GuildRank::Master | GuildRank::Elder | GuildRank::Member => RankPowers {\n            can_edit_guild: false,\n            can_approve_applications: false,\n            can_ban: false,\n            can_kick: false,\n        },",
     "        GuildRank::Master => RankPowers {\n            can_edit_guild: false,\n            can_approve_applications: false,\n            can_ban: true,\n            can_kick: true,\n        },\n        GuildRank::Elder | GuildRank::Member => RankPowers {\n            can_edit_guild: false,\n            can_approve_applications: false,\n            can_ban: false,\n            can_kick: false,\n        },"),

    ("MEMBER gains every power",
     "        GuildRank::Master | GuildRank::Elder | GuildRank::Member => RankPowers {\n            can_edit_guild: false,\n            can_approve_applications: false,\n            can_ban: false,\n            can_kick: false,\n        },",
     "        GuildRank::Master | GuildRank::Elder | GuildRank::Member => RankPowers {\n            can_edit_guild: true,\n            can_approve_applications: true,\n            can_ban: true,\n            can_kick: true,\n        },"),

    ("outranks() becomes non-strict (equal ranks may remove each other)",
     "        (self as u8) < (other as u8)",
     "        (self as u8) <= (other as u8)"),

    ("kick drops the outranking requirement",
     "    powers(actor).can_kick && actor.outranks(target)",
     "    powers(actor).can_kick"),

    ("unknown rank strings silently become MEMBER",
     '            _ => None,\n        }\n    }\n\n    /// Strictly more authoritative',
     '            _ => Some(GuildRank::Member),\n        }\n    }\n\n    /// Strictly more authoritative'),

    ("a second guild membership is allowed",
     "    if ctx.already_in_guild {\n        return Err(JoinRefusal::AlreadyHasGuild);\n    }",
     "    if false {\n        return Err(JoinRefusal::AlreadyHasGuild);\n    }"),

    ("a second application is allowed",
     "    if ctx.already_applied {\n        return Err(JoinRefusal::AlreadyAppliedToGuild);\n    }",
     "    if false {\n        return Err(JoinRefusal::AlreadyAppliedToGuild);\n    }"),

    ("CLOSED guilds become joinable",
     "        GuildType::Closed => return Err(JoinRefusal::GuildIsClosed),",
     "        GuildType::Closed => JoinAdmission::Join,"),

    ("CLOSED is checked after the level gate (refusal precedence breaks)",
     "    let admission = match guild_type {\n        GuildType::Closed => return Err(JoinRefusal::GuildIsClosed),\n        GuildType::Open => JoinAdmission::Join,\n        GuildType::ApplyOnly => JoinAdmission::Apply,\n    };\n\n    if ctx.character_level < MIN_LEVEL_TO_JOIN {\n        return Err(JoinRefusal::BelowMinimumLevel);\n    }",
     "    if ctx.character_level < MIN_LEVEL_TO_JOIN {\n        return Err(JoinRefusal::BelowMinimumLevel);\n    }\n\n    let admission = match guild_type {\n        GuildType::Closed => return Err(JoinRefusal::GuildIsClosed),\n        GuildType::Open => JoinAdmission::Join,\n        GuildType::ApplyOnly => JoinAdmission::Apply,\n    };"),

    ("APPLY_ONLY admits directly instead of queueing",
     "        GuildType::ApplyOnly => JoinAdmission::Apply,",
     "        GuildType::ApplyOnly => JoinAdmission::Join,"),

    ("the minimum-level gate is removed",
     "    if ctx.character_level < MIN_LEVEL_TO_JOIN {\n        return Err(JoinRefusal::BelowMinimumLevel);\n    }",
     "    if false {\n        return Err(JoinRefusal::BelowMinimumLevel);\n    }"),

    ("the removal cooldown stops blocking",
     "        if removal.blocks(ctx.now) {\n            return Err(JoinRefusal::UserRecentlyRemovedFromGuild);\n        }",
     "        if false && removal.blocks(ctx.now) {\n            return Err(JoinRefusal::UserRecentlyRemovedFromGuild);\n        }"),

    ("a ban expires like a kick",
     "        self.banned || now.saturating_sub(self.removed_at) < REJOIN_COOLDOWN_SECS",
     "        now.saturating_sub(self.removed_at) < REJOIN_COOLDOWN_SECS"),

    ("the member cap stops applying to joins",
     "            if ctx.member_count >= MAX_MEMBERS {\n                return Err(JoinRefusal::GuildIsAtMaxMembers);\n            }",
     "            if false {\n                return Err(JoinRefusal::GuildIsAtMaxMembers);\n            }"),

    ("the application cap stops applying",
     "            if ctx.application_count >= MAX_APPLICATIONS {\n                return Err(JoinRefusal::GuildIsAtMaxApplications);\n            }",
     "            if false {\n                return Err(JoinRefusal::GuildIsAtMaxApplications);\n            }"),

    ("approval checks capacity before permission",
     "    if !can_approve_applications(actor) {\n        return Err(ApprovalRefusal::NotPermitted);\n    }\n    if member_count >= MAX_MEMBERS {\n        return Err(ApprovalRefusal::GuildIsAtMaxMembers);\n    }",
     "    if member_count >= MAX_MEMBERS {\n        return Err(ApprovalRefusal::GuildIsAtMaxMembers);\n    }\n    if !can_approve_applications(actor) {\n        return Err(ApprovalRefusal::NotPermitted);\n    }"),

    ("message length is counted in bytes, not characters",
     "pub fn message_length_ok(text: &str) -> bool {\n    let len = text.chars().count();",
     "pub fn message_length_ok(text: &str) -> bool {\n    let len = text.len();"),

    ("empty chat messages are accepted",
     "    (MESSAGE_MIN_LEN..=MESSAGE_MAX_LEN).contains(&len)",
     "    len <= MESSAGE_MAX_LEN"),

    ("guild name length is unchecked",
     "    (NAME_MIN_LEN..=NAME_MAX_LEN).contains(&name_len)",
     "    true"),

    ("succession picks the most junior member",
     "        .min_by_key(|(_, rank, join_date)| (*rank as u8, *join_date))",
     "        .max_by_key(|(_, rank, join_date)| (*rank as u8, *join_date))"),

    ("MAX_MEMBERS is quietly changed to 25",
     "pub const MAX_MEMBERS: i64 = 20;",
     "pub const MAX_MEMBERS: i64 = 25;"),

    ("MAX_APPLICATIONS is quietly changed to 15",
     "pub const MAX_APPLICATIONS: i64 = 10;",
     "pub const MAX_APPLICATIONS: i64 = 15;"),

    ("the rejoin cooldown is quietly changed to 24h",
     "pub const REJOIN_COOLDOWN_SECS: i64 = 604_800;",
     "pub const REJOIN_COOLDOWN_SECS: i64 = 86_400;"),

    ("MIN_LEVEL_TO_JOIN is quietly changed to 1",
     "pub const MIN_LEVEL_TO_JOIN: u16 = 5;",
     "pub const MIN_LEVEL_TO_JOIN: u16 = 1;"),

    ("the message page limit is quietly changed to 100",
     "pub const MESSAGE_PAGE_LIMIT: i64 = 30;",
     "pub const MESSAGE_PAGE_LIMIT: i64 = 100;"),
]


def run_tests():
    p = subprocess.run(
        ["cargo", "test", "--locked", "-p", "server", "guild"],
        cwd=ROOT, capture_output=True, text=True,
    )
    out = p.stdout + p.stderr
    if "error[E" in out or "error: could not compile" in out:
        return None, out
    failed = re.findall(r"^test (\S+) \.\.\. FAILED$", out, re.M)
    return failed, out


original = io.open(SRC, encoding="utf-8").read()
ok = True
try:
    base_failed, base_out = run_tests()
    if base_failed is None:
        print("BASELINE DOES NOT COMPILE"); print(base_out[-3000:]); sys.exit(1)
    if base_failed:
        print("BASELINE IS NOT GREEN:", base_failed); sys.exit(1)
    total = re.search(r"(\d+) passed", base_out)
    print(f"baseline: green ({total.group(1) if total else '?'} passed)\n")

    for desc, find, repl in MUTATIONS:
        if original.count(find) != 1:
            print(f"[SKIP-BROKEN] {desc}: anchor matched {original.count(find)} times")
            ok = False
            continue
        io.open(SRC, "w", encoding="utf-8").write(original.replace(find, repl))
        failed, out = run_tests()
        if failed is None:
            print(f"[COMPILE-FAIL] {desc}")
            ok = False
        elif not failed:
            print(f"[!! GREEN !!]  {desc}  <-- RULE IS UNTESTED")
            ok = False
        else:
            print(f"[RED x{len(failed):<2}]     {desc}")
            for f in sorted(failed):
                print(f"                 - {f.split('::')[-1]}")
finally:
    io.open(SRC, "w", encoding="utf-8").write(original)

print("\nrestored original;", "ALL MUTATIONS CAUGHT" if ok else "SOME MUTATIONS SURVIVED")
sys.exit(0 if ok else 1)
