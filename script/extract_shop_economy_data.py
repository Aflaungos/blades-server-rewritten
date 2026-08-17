#!/usr/bin/env python3
"""Extract the town-vendor economy tables from the APK.

Why this exists
---------------
Tracker #30: "Merchants don't have money - we should reverse engineer the money
system." Two data gaps sat behind that:

  * The vendor's SELL price for the player's items was a flat placeholder
    (50 gold per instanced item, 5 per stackable unit) — retail's number is the
    item template's own `_sellValue`, and the placeholder was wrong by ~75x at
    the median.
  * Only 5 of the 94 bundles a town vendor can stock had a price and contents,
    so `POST /shops/{id}/purchase` silently skipped the other 89 — the merchant
    listed stock it would not sell.

Both live in the APK. From the il2cpp dump:

    ItemTemplate (dump.cs:559428)
        [SerializeField] protected int _value;       // shop/display value
        [SerializeField] protected int _sellValue;   // what a merchant pays you
    ItemBundleGenerationData -> ItemBundleGenerationDataList
        items[] + price[{currency, quantity}] + townXP

`_sellValue` is authored per template, NOT a fixed fraction of `_value`: the
ratio is 0.15 for 543 templates, 0.10 for 105 and 0.35 for 17, so it has to be
read rather than computed.

Outputs (both written to --out, default `deploy/static/`)
--------------------------------------------------------
  shop_bundles.json      {"<bundleId>": {"currencyId","price","grant"}, "_meta":{}}
                         Same shape `blades_lib::static_data::ShopBundle` already
                         parses. `grant.items[].id` is a placeholder — the handler
                         assigns a fresh instance id per purchase (as chests.rs
                         does), so the baked value is never used.
  item_sell_values.json  {"<templateId>": <gold the merchant pays>, "_meta":{}}

Reproduce
---------
  unzip reference/apk/blades.apk \
      'assets/Bundles/*' 'lib/arm64-v8a/libil2cpp.so' \
      'assets/bin/Data/Managed/Metadata/global-metadata.dat' -d /tmp/blades-apk-extract
  pip install 'UnityPy==1.25.0' 'TypeTreeGeneratorAPI==0.0.10'
  APK_EXTRACT=/tmp/blades-apk-extract python3 script/extract_shop_economy_data.py \
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

NIL_UUID = "00000000-0000-0000-0000-000000000000"

TEMPLATE_LISTS = [
    "ArmorTemplateList", "WeaponTemplateList", "ShieldTemplateList",
    "ItemTemplateList", "ConsumableTemplateList", "BookTemplateList",
    "EmoteTemplateList", "QuestItemTemplateList",
]

# `ItemType` values (from the client's own enum, cross-checked against the
# template list each entry lives in). Instanced types get an item INSTANCE in the
# backpack; everything else is a stackable keyed by template.
TYPE_NAME = {
    1: "consumable", 2: "weapon", 3: "armor", 4: "decoration", 5: "special",
    6: "currency", 7: "key", 8: "material", 9: "shield", 10: "ring",
    11: "jewelry", 12: "quest_item", 14: "emote",
}
INSTANCED_TYPES = {2, 3, 9, 10, 11}  # weapon, armor, shield, ring, jewelry


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
    if not isinstance(d, dict):
        return None
    inner = d.get("_uid", d)
    i = inner.get("_id") if isinstance(inner, dict) else None
    return None if i in (None, "", "0", NIL_UUID) else i


def build_templates(objs):
    """templateId -> {type, tier, sellValue, value, maxDurability}."""
    out = {}
    for cls in TEMPLATE_LISTS:
        for tt in by_class(objs, cls):
            for e in tt.get("_templateList") or []:
                u = uid(e)
                if not u:
                    continue
                out[u] = {
                    "editorName": e.get("_editorName"),
                    "type": e.get("_type"),
                    "tier": e.get("_tier"),
                    "value": int(e.get("_value") or 0),
                    "sellValue": int(e.get("_sellValue") or 0),
                    "maxDurability": float(e.get("_maxDurability") or 0.0),
                    # `_temperProperties[L-1]._value` is a MULTIPLIER on
                    # `_sellValue` for temper level L (1..10), not an absolute —
                    # measured against 508 retail buybacks. Level 0 is 1.0.
                    "temperMult": [
                        float(tp.get("_value") or 0.0)
                        for tp in (e.get("_temperProperties") or [])
                    ],
                }
    return out


def build_bundles(objs, templates):
    """bundleId -> {currencyId, price, grant} in the shape ShopBundle parses."""
    out = {}
    skipped_no_price = 0
    unknown_items = 0
    for tt in by_class(objs, "ItemBundleGenerationDataList"):
        for b in tt.get("_templateList") or tt.get("_itemBundles") or []:
            bid = uid(b)
            if not bid:
                continue
            currency = price = None
            for p in b.get("_prices") or []:
                c = uid(p.get("_itemTemplatePointer"))
                if c:
                    currency, price = c, int(p.get("_quantity") or 0)
                    break
            if currency is None:
                skipped_no_price += 1
                continue

            grant = {}
            stackables = {}
            instanced = []
            for it in b.get("_items") or []:
                tpl = uid(it.get("_itemTemplatePointer"))
                qty = int(it.get("_quantity") or 1)
                if not tpl:
                    continue
                meta = templates.get(tpl)
                if meta is None:
                    unknown_items += 1
                    # Unknown template: treat as a stackable so the purchase at
                    # least grants something rather than silently dropping it.
                    stackables[tpl] = stackables.get(tpl, 0) + qty
                    continue
                if meta["type"] in INSTANCED_TYPES:
                    # One backpack instance per unit, at full condition for temper
                    # 0. `id` is a placeholder; the handler assigns a fresh uuid.
                    for _ in range(max(qty, 1)):
                        instanced.append({
                            "id": NIL_UUID,
                            "itemTemplateId": tpl,
                            "temperingLevel": 0,
                            "durability": meta["maxDurability"],
                        })
                else:
                    stackables[tpl] = stackables.get(tpl, 0) + qty
            if stackables:
                grant["stackableItems"] = stackables
            if instanced:
                grant["items"] = instanced
            town_xp = int(b.get("_townXP") or b.get("_townXp") or 0)
            if town_xp:
                grant["townXp"] = town_xp

            out[bid] = {
                "currencyId": currency,
                "price": price,
                "grant": grant,
            }
    return out, skipped_no_price, unknown_items


def build_enchant_values(objs):
    """propertyId -> {tier(str): gold the enchantment adds to the sell price}.

    `ItemPropertyList._propertyList[]._tiers[]._value` (dump.cs `ItemProperty` /
    `ItemPropertyTier`). Retail's sell price adds this at FACE value, not scaled
    by the 0.15 sell fraction — measured over 508 retail buybacks.
    """
    out = {}
    for tt in by_class(objs, "ItemPropertyList"):
        for p in tt.get("_propertyList") or []:
            pid = uid(p)
            if not pid:
                continue
            tiers = {}
            for t in p.get("_tiers") or []:
                n = t.get("_tierNumber")
                v = t.get("_value")
                if n is None or not v:
                    continue
                tiers[str(int(n))] = int(v)
            if tiers:
                out[pid] = tiers
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="deploy/static")
    args = ap.parse_args()

    objs = load_bundles()
    templates = build_templates(objs)
    bundles, no_price, unknown_items = build_bundles(objs, templates)
    enchant_values = build_enchant_values(objs)

    sell_values = {}
    for t, m in templates.items():
        if m["sellValue"] <= 0:
            continue
        row = {"sellValue": m["sellValue"]}
        if any(v > 0 for v in m["temperMult"]):
            row["temperMult"] = m["temperMult"]
        sell_values[t] = row

    meta = {
        "_source": "APK Unity bundles (reference/apk/blades.apk), Unity 2019.4.37f1 "
                   "il2cpp metadata v24",
        "_apk_sha256": APK_SHA256,
        "_extractor": "script/extract_shop_economy_data.py",
        "_il2cpp": {
            "sellValue": "ItemTemplate._sellValue — authored per template, NOT a "
                         "fixed fraction of _value",
            "temperMult": "ItemTemplate._temperProperties[L-1]._value — a "
                          "MULTIPLIER on _sellValue for temper level L (1..10); "
                          "level 0 is 1.0",
            "enchantValue": "ItemPropertyList._propertyList[]._tiers[]._value — "
                            "added to the sell price at FACE value",
            "bundles": "ItemBundleGenerationDataList -> _items[] + _prices[]",
        },
        "_sell_price_rule": "price = round(sellValue * temperMult(level)) + sum over "
                            "properties.ENCHANTING of enchantValue[id][tier]; "
                            "properties.GRADING contributes nothing. Validated at "
                            "478/508 exact against retail buybacks.",
    }

    os.makedirs(args.out, exist_ok=True)
    paths = {
        "shop_bundles.json": bundles,
        "item_sell_values.json": sell_values,
        "enchant_values.json": enchant_values,
    }
    for name, payload in paths.items():
        with open(os.path.join(args.out, name), "w") as f:
            json.dump(dict(payload, _meta=meta), f, sort_keys=True, indent=1)

    print("[bundles] %d priced bundles (%d had no price, %d unknown item templates)"
          % (len(bundles), no_price, unknown_items), file=sys.stderr)
    print("[sellValue] %d templates with a nonzero sell value (of %d); %d carry a "
          "temper multiplier ladder"
          % (len(sell_values), len(templates),
             sum(1 for r in sell_values.values() if "temperMult" in r)), file=sys.stderr)
    print("[enchantValue] %d enchantment/grading properties with tier values"
          % len(enchant_values), file=sys.stderr)


if __name__ == "__main__":
    main()
