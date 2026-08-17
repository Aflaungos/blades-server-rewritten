#!/usr/bin/env python3
"""Extract the AUTHORITATIVE item durability + repair-cost tables from the APK.

Why this exists
---------------
`deploy/static/item_durability.json` used to be built from captured traffic by
`blades-capture/scripts/extract-item-durability.py`, which took the *maximum
durability ever observed* for a `(itemTemplateId, temperingLevel)` pair as that
pair's full durability. That is unsound twice over:

  * a pair that was only ever captured while the item was DAMAGED yields a max
    below the real one, and
  * a pair nobody ever equipped yields no entry at all.

The result on prod was a table covering 218 of 1113 templates at an average of
1.44 of the 11 temper levels each, so `repair.rs` silently skipped most items
("repair all does not repair all", tracker #30).

The real numbers ship inside the APK. From the il2cpp dump:

    ItemTemplate (dump.cs:559428)
        public const float DEFAULT_DURABILITY = 100;
        [SerializeField] protected float _maxDurability;          // temper level 0
        [SerializeField] private ItemTemperProperties[] _temperProperties;
    ItemTemperProperties (dump.cs:559379)
        _damage, _twoHandedDamage, _protection, _value, _maxDurability

so max durability is an ABSOLUTE per-(template, temper level) value:

    temper 0      -> ItemTemplate._maxDurability
    temper 1..10  -> ItemTemplate._temperProperties[level - 1]._maxDurability

and the repair price comes from

    RepairRecipe (dump.cs:553248)
        [SerializeField] protected List<RecipeInput> _inputs;      // gold
        [SerializeField] private LevelInputData[] _temperLevels;
        [SerializeField] private LevelInputData[] _enchantmentLevels;
        private void AddInputs(map, inputs, float itemCondition)

`AddInputs` takes the item's condition, i.e. the recipe's gold is the price of
repairing from BROKEN to full and the charge scales with the missing fraction.
Every input in all 626 shipped recipes is Gold (`f8d27767-…`); the temper and
enchantment surcharge arrays are present but zero throughout.

Outputs (both written to --out, default `deploy/static/`)
--------------------------------------------------------
  item_durability.json  {"<templateId>": {"0": f, "1": f, … "10": f}, "_meta": {…}}
  repair_costs.json     {"<templateId>": <gold at zero condition>, "_meta": {…}}

Reproduce
---------
  unzip reference/apk/blades.apk \
      'assets/Bundles/*' 'lib/arm64-v8a/libil2cpp.so' \
      'assets/bin/Data/Managed/Metadata/global-metadata.dat' -d /tmp/blades-apk-extract
  pip install 'UnityPy==1.25.0' 'TypeTreeGeneratorAPI==0.0.10'
  APK_EXTRACT=/tmp/blades-apk-extract python3 script/extract_item_repair_data.py \
      --out deploy/static

APK: reference/apk/blades.apk in the blades-capture repo, Unity 2019.4.37f1,
il2cpp metadata v24, SHA-256
fd6e55f561542cac41a016975318bc69759e512d5155b3c8945367883c833417.
Same extraction method as `script/extract_parsed_json.py`.
"""

import argparse
import json
import os
import sys

APK_EXTRACT = os.environ.get("APK_EXTRACT", "/tmp/blades-apk-extract")
BUNDLES = APK_EXTRACT + "/assets/Bundles"
SO = APK_EXTRACT + "/lib/arm64-v8a/libil2cpp.so"
META = APK_EXTRACT + "/assets/bin/Data/Managed/Metadata/global-metadata.dat"

APK_SHA256 = "fd6e55f561542cac41a016975318bc69759e512d5155b3c8945367883c833417"
GOLD = "f8d27767-a85e-4fd6-a5bb-bf8a13d0daa2"

# ItemTemplate.DEFAULT_DURABILITY (dump.cs:559431).
DEFAULT_DURABILITY = 100.0
# Temper levels the client models: 0 (untempered) .. 10.
MAX_TEMPER_LEVEL = 10

TEMPLATE_LISTS = [
    "ArmorTemplateList", "WeaponTemplateList", "ShieldTemplateList",
    "ItemTemplateList", "ConsumableTemplateList", "BookTemplateList",
    "EmoteTemplateList", "QuestItemTemplateList",
]


def load_bundles():
    import UnityPy
    from UnityPy.helpers.TypeTreeGenerator import TypeTreeGenerator

    for p in (BUNDLES, SO, META):
        if not os.path.exists(p):
            sys.exit("missing %s — see the module docstring for the unzip command" % p)
    env = UnityPy.load(BUNDLES)
    objs = list(env.objects)
    if not objs:
        sys.exit("no Unity objects loaded from %s" % BUNDLES)
    gen = TypeTreeGenerator(objs[0].assets_file.unity_version)
    gen.load_il2cpp(open(SO, "rb").read(), open(META, "rb").read())
    env.typetree_generator = gen
    print("[apk] %d objects, il2cpp ready" % len(objs), file=sys.stderr)
    return objs


def classname(o):
    try:
        return o.read().m_Script.read().m_ClassName
    except Exception:
        return ""


def by_class(objs, name):
    for o in objs:
        if o.type.name != "MonoBehaviour" or classname(o) != name:
            continue
        try:
            yield o.read_typetree()
        except Exception:
            continue


def uid(d):
    """Unwrap the `{_uid: {_id}}` / `{_id}` Uid shapes; None for nil/empty."""
    if not isinstance(d, dict):
        return None
    inner = d.get("_uid", d)
    i = inner.get("_id") if isinstance(inner, dict) else None
    return None if i in (None, "", "0") else i


def gold_of(inputs):
    """The gold quantity in a `List<RecipeInput>`, or 0."""
    for i in inputs or []:
        if uid(i.get("_itemTemplate")) == GOLD:
            return int(i.get("_quantity") or 0)
    return 0


def build_durability(objs):
    """templateId -> {"0".."10": max durability}. Only breakable items
    (`_maxDurability > 0`) get an entry — a durability of 0 means "not
    breakable" (`ItemTemplate.IsBreakable`) and repair must not touch it."""
    table = {}
    stats = {"templates": 0, "breakable": 0, "temperable": 0}
    for cls in TEMPLATE_LISTS:
        for tt in by_class(objs, cls):
            for e in tt.get("_templateList") or []:
                u = uid(e)
                if not u:
                    continue
                stats["templates"] += 1
                base = float(e.get("_maxDurability") or 0.0)
                if base <= 0.0:
                    continue
                stats["breakable"] += 1
                levels = {"0": base}
                temper = e.get("_temperProperties") or []
                if temper:
                    stats["temperable"] += 1
                for lvl in range(1, MAX_TEMPER_LEVEL + 1):
                    if lvl - 1 < len(temper):
                        v = temper[lvl - 1].get("_maxDurability")
                        # A zero/absent entry means this level is not authored;
                        # carry the previous level forward rather than dropping
                        # to 0, so an out-of-range temper level can never make
                        # an item unrepairable.
                        levels[str(lvl)] = (
                            float(v) if v else levels[str(lvl - 1)]
                        )
                    else:
                        # Untemperable item (e.g. a unique with CannotBeTempered):
                        # every level is the base value.
                        levels[str(lvl)] = levels[str(lvl - 1)]
                table[u] = levels
    return table, stats


def build_repair_costs(objs):
    """templateId -> gold to repair from zero condition to full."""
    costs = {}
    surcharges = 0
    for tt in by_class(objs, "RepairRecipeList"):
        for r in tt.get("_recipes") or []:
            tpl = uid(r.get("_inputItemType"))
            if not tpl:
                continue
            costs[tpl] = gold_of(r.get("_inputs"))
            for group in ("_temperLevels", "_enchantmentLevels"):
                for lv in r.get(group) or []:
                    if gold_of(lv.get("_inputs")):
                        surcharges += 1
    return costs, surcharges


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="deploy/static",
                    help="directory to write item_durability.json + repair_costs.json")
    args = ap.parse_args()

    objs = load_bundles()

    durability, stats = build_durability(objs)
    costs, surcharges = build_repair_costs(objs)

    meta = {
        "_source": "APK Unity bundles (reference/apk/blades.apk), Unity 2019.4.37f1 "
                   "il2cpp metadata v24",
        "_apk_sha256": APK_SHA256,
        "_extractor": "script/extract_item_repair_data.py",
        "_il2cpp": {
            "maxDurability": "ItemTemplate._maxDurability (temper 0) + "
                             "ItemTemplate._temperProperties[level-1]._maxDurability "
                             "(temper 1..10)",
            "repairCost": "RepairRecipe._inputs gold, scaled at runtime by the "
                          "item's condition deficit (RepairRecipe.AddInputs)",
        },
        "_default_durability": DEFAULT_DURABILITY,
    }

    os.makedirs(args.out, exist_ok=True)
    dur_path = os.path.join(args.out, "item_durability.json")
    cost_path = os.path.join(args.out, "repair_costs.json")

    with open(dur_path, "w") as f:
        json.dump(dict(durability, _meta=meta), f, sort_keys=True, indent=1)
    with open(cost_path, "w") as f:
        json.dump(dict(costs, _meta=meta), f, sort_keys=True, indent=1)

    print("[durability] %s: %d breakable templates × %d temper levels "
          "(%d templates seen, %d temperable)"
          % (dur_path, len(durability), MAX_TEMPER_LEVEL + 1,
             stats["templates"], stats["temperable"]), file=sys.stderr)
    print("[repair] %s: %d recipes; nonzero temper/enchant surcharges: %d"
          % (cost_path, len(costs), surcharges), file=sys.stderr)


if __name__ == "__main__":
    main()
