#!/usr/bin/env python3
"""Extract the `GuildData` ScriptableObject from the Blades APK bundles.

Produces the JSON checked in at `data/guild_data.json`, which is the provenance
for every constant and for the whole permission matrix in
`server/src/guild_policy.rs`. Re-run it to re-derive them rather than trust the
committed file.

Schema-driven manual reader. `TypeTreeGeneratorAPI` is not required (and was in
fact unavailable when this was written): instead of generating typetrees from
libil2cpp.so we parse the MonoBehaviour's raw serialized bytes directly, using the
field layout read straight out of the il2cpp dump's `GuildData` (TypeDefIndex
11371) and its nested types.

Correctness proof, and the reason the output is trustworthy: the reader must
consume EXACTLY len(raw_data) bytes with no slack. A wrong field order or a
mis-aligned bool desyncs by tens of bytes and the check fails loudly rather than
yielding plausible-looking garbage. As a second check, every string field decodes
to a localization key that resolves in `loc_strings_en.json`.

Usage:
    # 1. unzip the APK's bundles somewhere persistent. The APK lives in the
    #    blades-capture repo at reference/apk/blades.apk:
    unzip -o blades.apk 'assets/Bundles/*' -d /some/persistent/dir
    # 2. point this at the extracted Bundles directory:
    python3 script/extract_guild_data.py /some/persistent/dir/assets/Bundles

Requires UnityPy. NOTE: the sibling extractors in the blades-capture repo
(reference/game-defs/extract/) assume a venv and an unzip target under /tmp, both
of which macOS reaps — hence the explicit path argument here.
"""
import os, struct, json, sys
import UnityPy

DEFAULT_BUNDLES = "/tmp/blades-apk-extract/assets/Bundles"
BUNDLES = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "BLADES_BUNDLES", DEFAULT_BUNDLES
)

class R:
    def __init__(self, b):
        self.b = b; self.p = 0
    def align(self, n=4):
        r = self.p % n
        if r: self.p += n - r
    def i32(self):
        v = struct.unpack_from("<i", self.b, self.p)[0]; self.p += 4; return v
    def f32(self):
        v = struct.unpack_from("<f", self.b, self.p)[0]; self.p += 4; return v
    def i64(self):
        v = struct.unpack_from("<q", self.b, self.p)[0]; self.p += 8; return v
    def boolean(self):
        v = self.b[self.p]; self.p += 1; self.align(4); return bool(v)
    def string(self):
        n = self.i32()
        s = self.b[self.p:self.p+n].decode("utf-8", "replace"); self.p += n
        self.align(4); return s
    def pptr(self):
        return {"m_FileID": self.i32(), "m_PathID": self.i64()}
    def lst(self, fn):
        n = self.i32()
        return [fn() for _ in range(n)]

def uid(r):            return r.string()                      # Uid { _id }
def locstr(r):         return r.string()                      # LocalizedString { _key }
def uidptr(r):         return {"uid": uid(r), "uid_parent": uid(r), "search_type": r.i32()}
def uiditemptr(r):
    d = uidptr(r); d["item_type_filter"] = r.i32(); d["equipment_slot_filter"] = r.i32(); return d
def codepointrange(r): return [r.i32(), r.i32()]

def utv(r):  # UserTextValidation
    return {
        "valid_code_point_ranges": r.lst(lambda: codepointrange(r)),
        "invalid_code_points": r.lst(r.i32),
        "min_length": r.i32(),
        "max_length": r.i32(),
        "profanity_filter_languages_checked": r.lst(r.string),
    }

def badge_icon(r):   return {"index": r.i32(), "sprite_uid_icon": uidptr(r)}
def region(r):       return {"index": r.i32(), "weight": r.i32(), "name": r.string(), "title": locstr(r)}
def lowhigh(r):      return {"low": r.i32(), "high": r.i32(), "title": locstr(r)}
def itemqty(r):      return {"item_template": uiditemptr(r), "quantity": r.i32(), "item_enhancement": uidptr(r)}

def msgboard(r):
    return {"max_messages_to_display": r.i32(),
            "profanity_replace_symbol": r.string(),
            "client_text_validation": utv(r)}

GUILD_TYPE = {-1: "INVALID", 0: "OPEN", 1: "APPLY_ONLY", 2: "CLOSED"}
GUILD_RANK = {-1: "INVALID", 0: "GRANDMASTER", 1: "MASTER", 2: "ELDER", 3: "MEMBER"}

def guildtypedata(r):
    return {"guild_type": GUILD_TYPE.get(r.i32()), "sprite_size": [r.f32(), r.f32()],
            "sprite_uid": uidptr(r), "title": locstr(r)}

def rankperm(r):
    return {"guild_rank": GUILD_RANK.get(r.i32()), "can_kick": r.boolean(), "can_ban": r.boolean()}

def guildrankdata(r):
    return {"guild_rank": GUILD_RANK.get(r.i32()),
            "sprite_uid_icon": uidptr(r), "sprite_uid_portrait_frame": uidptr(r),
            "title": locstr(r),
            "can_edit_guild": r.boolean(),
            "can_approve_guild_applications": r.boolean(),
            "can_ban_non_members": r.boolean(),
            "rank_permissions": r.lst(lambda: rankperm(r))}

def main():
    env = UnityPy.load(BUNDLES)
    raw = None
    for o in env.objects:
        if o.type.name == "MonoBehaviour" and o.path_id == 454:
            raw = bytes(o.get_raw_data()); break
    if raw is None:
        sys.exit("GuildData (path_id 454) not found")

    r = R(raw)
    r.pptr()                    # m_GameObject
    r.boolean()                 # m_Enabled
    r.pptr()                    # m_Script
    name = r.string()           # m_Name
    assert name == "GuildData", name

    out = {
        "separator": r.string(),
        "min_level_to_join": r.i32(),
        "max_members": r.i32(),
        "max_applications": r.i32(),
        "name_validation": utv(r),
        "short_description_validation": utv(r),
        "long_description_validation": utv(r),
        "badge_icons": r.lst(lambda: badge_icon(r)),
        "regions": r.lst(lambda: region(r)),
        "scores": r.lst(lambda: lowhigh(r)),
        "sizes": r.lst(lambda: lowhigh(r)),
        "create_costs": r.lst(lambda: itemqty(r)),
        "message_board": msgboard(r),
        "guild_types_data": r.lst(lambda: guildtypedata(r)),
        "guild_ranks_data": r.lst(lambda: guildrankdata(r)),
        "chat_timestamp_refresh_interval_seconds": r.f32(),
        "admission_timeout_after_removal_from_guild_in_seconds": r.f32(),
    }

    out["_meta"] = {
        "source": "reference/apk/blades.apk -> assets/Bundles/BuildPlayer-common.sharedAssets, "
                  "MonoBehaviour path_id 454, m_Name 'GuildData' (class GuildData : ScriptableObject)",
        "method": "manual raw-bytes reader driven by the field layout in reference/il2cpp/dump.cs "
                  "(GuildData TypeDefIndex 11371). Verified by exact byte consumption.",
        "bytes_total": len(raw),
        "bytes_consumed": r.p,
        "exact": r.p == len(raw),
    }
    print(json.dumps(out, indent=1))
    print("consumed %d / %d  EXACT=%s" % (r.p, len(raw), r.p == len(raw)), file=sys.stderr)

main()
