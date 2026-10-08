"""Build an OCR fine-tuning dataset from `--dump-images` scan runs.

Each dumped item folder holds per-field crops plus `ocr_fields.json` (field,
crop file, raw OCR text, parsed result, final GOOD object). Items are aligned
to a groundtruth GOOD export; a crop gets a label rendered from the verified
field value (the exact on-screen text), never from the OCR hypothesis.

Label sources:
  gt-verified   item's scanned object matches groundtruth; label rendered from it
  gt-corrected  identity matches groundtruth but this field differed; label from GT
  ocr-only      no groundtruth alignment; label is the raw OCR text (review before use)

Characters rescanned in phase 2 (debug_images/gi_characters_rescan/<index>) are
aligned via the rescan's object; their crops are stored as item "<index>_rescan".

Output layout (--out):
  images/<field>/<run>_<category>_<item>_<crop>.png
  manifest.jsonl     one record per kept crop (all sources)
  rec_gt.txt         PaddleOCR recognition format: "<image>\t<label>" (GT sources only)
  charset.txt        every character used by GT labels
  coverage.json      per-field pair counts, distinct labels, expected-but-missing values,
                     label characters absent from the PP-OCRv6 tiny dictionary

Usage:
  python tools/build_ocr_dataset.py --gt GT.json --mappings data/mappings.json \
      --config target/release/data/good_config.json --out training_data/ocr_v6_tiny \
      RUN_DIR [RUN_DIR ...]
  python tools/build_ocr_dataset.py --gt GT.json --mappings ... --config ... \
      --eval RUN_DIR [RUN_DIR ...]      # per-field raw-OCR accuracy only, no copying
"""

import argparse
import glob
import hashlib
import json
import os
import re
import shutil
from collections import Counter, defaultdict

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
V6_DICT = os.path.join(REPO, "genshin", "src", "scanner", "common", "models", "ppocrv6_tiny_dict.txt")

ELEMENT_ZH = {
    "pyro": "火", "hydro": "水", "electro": "雷", "cryo": "冰",
    "anemo": "风", "geo": "岩", "dendro": "草",
}
SLOT_ZH = {"flower": "生之花", "plume": "死之羽", "sands": "时之沙", "goblet": "空之杯", "circlet": "理之冠"}
STAT_ZH = {
    "hp": "生命值", "hp_": "生命值", "atk": "攻击力", "atk_": "攻击力", "def": "防御力", "def_": "防御力",
    "eleMas": "元素精通", "enerRech_": "元素充能效率", "critRate_": "暴击率", "critDMG_": "暴击伤害",
    "heal_": "治疗加成", "physical_dmg_": "物理伤害加成",
    **{f"{e}_dmg_": f"{zh}元素伤害加成" for e, zh in ELEMENT_ZH.items()},
}
# Renameable characters: GOOD key -> good_config.json field holding the in-game name.
RENAMEABLE = {
    "Traveler": "traveler_name", "Wanderer": "wanderer_name",
    "Manekin": "manekin_name", "Manekina": "manekina_name",
}
# Keys the remote mappings do not carry yet; names read cleanly by every OCR engine.
EXTRA_ZH_NAMES = {"Vesna": "薇斯纳", "Vodyanitsa": "沃雅妮莎"}

EDGE_NOISE = " ·•.,，。:：'\"‘’“”`、…|;；"
CATEGORY_SINGULAR = {"characters": "character", "weapons": "weapon", "artifacts": "artifact"}
# Phase-2 character rescans dump to a sibling folder keyed by the same index.
RESCAN_DIR = "characters_rescan"
PHASE1, RESCAN = "phase1", "rescan"


def load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


class Names:
    def __init__(self, mappings, config):
        self.char = {c["id"]: c["n"]["zh"] for c in mappings["characters"]}
        self.char_elem = {c["id"]: c.get("e", "") for c in mappings["characters"]}
        self.weapon = {w["id"]: w["n"]["zh"] for w in mappings["weapons"]}
        self.set = {s["id"]: s["n"]["zh"] for s in mappings["artifactSets"]}
        self.char.update(EXTRA_ZH_NAMES)
        for key, field in RENAMEABLE.items():
            if config.get(field):
                self.char[key] = config[field]

    def character(self, key):
        return self.char.get(key)


def normalize(text):
    text = "".join(
        chr(ord(c) - 0xFEE0) if "\uff01" <= c <= "\uff5e" else c for c in text if not c.isspace()
    )
    return text.strip(EDGE_NOISE)


def fmt_stat(key, value):
    # The game groups flat values with commas ("生命值+1,016").
    return f"{value:.1f}%" if key.endswith("_") else f"{int(round(value)):,}"


def render_substat(sub, inactive):
    label = f"{STAT_ZH[sub['key']]}+{fmt_stat(sub['key'], sub['value'])}"
    return label + "（待激活）" if inactive else label


def equip_label(names, location):
    zh = names.character(location) if location else None
    return f"{zh}已装备" if zh else None


def talent_value(final):
    return f"Lv.{final}" if final and final.isdigit() else None


def character_labels(obj, names):
    """Map field name -> label for a verified character object."""
    zh = names.character(obj["key"])
    elem = (obj.get("element") or names.char_elem.get(obj["key"], "")).lower()
    labels = {}
    if zh and elem in ELEMENT_ZH:
        labels["name"] = f"{ELEMENT_ZH[elem]}元素/{zh}"
    return labels


def weapon_labels(obj, names):
    labels = {"refinement": f"{obj['refinement']}精炼{obj['refinement']}阶"}
    if obj["key"] in names.weapon:
        labels["name"] = names.weapon[obj["key"]]
    equip = equip_label(names, obj.get("location"))
    if equip:
        labels["equip"] = equip
    return labels


def artifact_labels(obj, names):
    labels = {
        "slot": SLOT_ZH[obj["slotKey"]],
        "main_stat": STAT_ZH[obj["mainStatKey"]],
        "level": f"+{obj['level']}",
    }
    if obj["setKey"] in names.set:
        labels["set_name"] = f"{names.set[obj['setKey']]}："
    lines = [(s, False) for s in obj["substats"]] + [(s, True) for s in obj.get("unactivatedSubstats", [])]
    for i, (sub, inactive) in enumerate(lines):
        labels[f"sub[{i}]"] = render_substat(sub, inactive)
    equip = equip_label(names, obj.get("location"))
    if equip:
        labels["equip"] = equip
    return labels


# ── Groundtruth alignment ────────────────────────────────────────────────────


def char_fingerprint(c):
    t = c.get("talent", {})
    return (c["key"], c["level"], c["constellation"], c["ascension"], t.get("auto"), t.get("skill"), t.get("burst"))


def weapon_fingerprint(w):
    return (w["key"], w["level"], w["ascension"], w["refinement"], w.get("location", ""), w.get("lock", False))


def artifact_identity(a):
    subs = tuple((s["key"], round(s["value"], 1)) for s in a["substats"])
    unact = tuple((s["key"], round(s["value"], 1)) for s in a.get("unactivatedSubstats", []))
    return (a["setKey"], a["slotKey"], a["rarity"], a["level"], a["mainStatKey"], subs, unact)


def artifact_full(a):
    return artifact_identity(a) + (
        a.get("location", ""), a.get("lock", False), a.get("astralMark", False),
        a.get("elixirCrafted", a.get("elixerCrafted", False)),
    )


class Aligner:
    """Consumes groundtruth entries so each GT object verifies at most one scanned item."""

    def __init__(self, gt, scanned_characters):
        self.chars = defaultdict(list)
        for c in gt.get("characters", []):
            self.chars[char_fingerprint(c)].append(c)
        # A key scanned twice means at least one misidentification; trust neither.
        self.dup_char_keys = {k for k, n in Counter(c["key"] for c in scanned_characters).items() if n > 1}
        self.weapons = Counter(weapon_fingerprint(w) for w in gt.get("weapons", []))
        self.art_full = Counter(artifact_full(a) for a in gt.get("artifacts", []))
        self.art_ident = defaultdict(list)
        for a in gt.get("artifacts", []):
            self.art_ident[artifact_identity(a)].append(a)

    def character(self, obj):
        if obj["key"] in self.dup_char_keys:
            return None
        matches = self.chars.get(char_fingerprint(obj))
        if not matches:
            return None
        return matches.pop(0)

    def weapon(self, obj):
        fp = weapon_fingerprint(obj)
        if self.weapons[fp] <= 0:
            return None
        self.weapons[fp] -= 1
        return obj

    def artifact(self, obj):
        """Returns (gt_object, fully_matched) or (None, False)."""
        candidates = self.art_ident.get(artifact_identity(obj))
        if not candidates:
            return None, False
        full = artifact_full(obj)
        for i, cand in enumerate(candidates):
            if artifact_full(cand) == full:
                return candidates.pop(i), True
        return candidates.pop(0), False


# ── Per-item label resolution ────────────────────────────────────────────────


def crop_fields(manifest):
    return [f for f in manifest["fields"] if f.get("crop") and not f.get("inference_error")]


def resolve_rescanned_character(phase1, rescan, aligner, names):
    """Label phase 1 + rescan crops of one character from the rescan's object.

    Phase 1 level/talent reads are what triggered the rescan, so their labels
    come from the rescan's own re-reads (detail screens show the same boosted
    talent levels as the overview), never from phase 1 finals.
    """
    obj = rescan.get("final_object")
    gt = aligner.character(obj) if isinstance(obj, dict) else None
    parts = [(PHASE1, phase1), (RESCAN, rescan)]
    if gt is None:
        for part, manifest in parts:
            for f in crop_fields(manifest):
                yield part, f, f["raw"], "ocr-only"
        return

    finals = {f["field"]: f["final"] for f in rescan["fields"]}
    labels = character_labels(gt, names)
    if finals.get("level"):
        labels["level"] = f"等级{finals['level']}"
    for kind in ("auto", "skill", "burst"):
        value = talent_value(finals.get(f"talent_detail_{kind}"))
        if value:
            labels[f"talent_{kind}"] = labels[f"talent_detail_{kind}"] = value
    for part, manifest in parts:
        for f in crop_fields(manifest):
            if f["field"] in labels:
                yield part, f, labels[f["field"]], "gt-verified"


def resolve_item(category, manifest, aligner, names):
    """Yield (field_record, label, source) for each crop in one dumped item."""
    obj = manifest.get("final_object")
    fields = crop_fields(manifest)
    if not isinstance(obj, dict):
        for f in fields:
            yield f, f["raw"], "ocr-only"
        return

    gt_labels, corrected = {}, set()
    if category == "characters":
        gt = aligner.character(obj)
        if gt is not None:
            gt_labels = character_labels(gt, names)
            for f in fields:
                if f["field"] == "level" and f["final"]:
                    gt_labels["level"] = f"等级{f['final']}"
                elif f["field"].startswith("talent") and talent_value(f["final"]):
                    gt_labels[f["field"]] = talent_value(f["final"])
    elif category == "weapons":
        if aligner.weapon(obj) is not None:
            gt_labels = weapon_labels(obj, names)
            level = next((f["final"] for f in fields if f["field"] == "level"), "")
            if level:
                gt_labels["level"] = f"Lv.{level}"
    elif category == "artifacts":
        gt, full = aligner.artifact(obj)
        if gt is not None:
            gt_labels = artifact_labels(gt, names)
            if not full and gt.get("location", "") != obj.get("location", ""):
                corrected.add("equip")

    for f in fields:
        label = gt_labels.get(f["field"])
        if label is not None:
            yield f, label, "gt-corrected" if f["field"] in corrected else "gt-verified"
        elif not gt_labels:
            yield f, f["raw"], "ocr-only"
        # Aligned item but no label for this crop (e.g. unequipped equip region
        # showing unrelated text, or a 4th substat crop on a 3-line artifact): skip.


def load_manifests(dump_root, category):
    """Item folder name -> (item_dir, ocr_fields manifest)."""
    paths = glob.glob(os.path.join(dump_root, category, "*", "ocr_fields.json"))
    items = {os.path.basename(os.path.dirname(p)): (os.path.dirname(p), load_json(p)) for p in sorted(paths)}
    return {key: value for key, value in items.items() if value[1].get("game", "genshin") == "genshin"}


def iter_items(run_dir):
    """Yield (dump root, category, scanned objects, item manifests by phase)."""
    export = glob.glob(os.path.join(run_dir, "good_export_*.json"))
    # Accept debug_images directly or discover it under the scanner cwd.
    # Historical unprefixed and timestamped dumps remain readable.
    categories = ("characters", "weapons", "artifacts")
    if any(os.path.isdir(os.path.join(run_dir, "gi_" + c)) for c in categories):
        roots = [(run_dir, "gi_")]
    elif any(os.path.isdir(os.path.join(run_dir, c)) for c in categories):
        roots = [(run_dir, "")]
    else:
        dump_root = os.path.join(run_dir, "debug_images")
        if any(os.path.isdir(os.path.join(dump_root, "gi_" + c)) for c in categories):
            roots = [(dump_root, "gi_")]
        else:
            roots = [(p, "") for p in sorted(glob.glob(os.path.join(dump_root, "genshin", "run_*")))]
            if any(os.path.isdir(os.path.join(dump_root, c)) for c in categories):
                roots.append((dump_root, ""))
    for root, prefix in roots:
        yield from iter_dump_items(root, categories, export, prefix)


def iter_dump_items(root, categories, export, prefix):
    for category in categories:
        items = load_manifests(root, prefix + category)
        if not items:
            continue
        rescans = load_manifests(root, prefix + RESCAN_DIR) if category == "characters" else {}
        scanned = (load_json(export[0]).get(category) or []) if export else []
        if not export:
            scanned = [
                (rescans.get(key, item)[1]).get("final_object")
                for key, item in items.items()
                if isinstance((rescans.get(key, item)[1]).get("final_object"), dict)
            ]
        parts = [
            (item, {PHASE1: entry, **({RESCAN: rescans[item]} if item in rescans else {})})
            for item, entry in items.items()
        ]
        yield root, category, scanned, parts


def resolve_parts(category, parts, aligner, names):
    """Yield (part, field_record, label, source) for every crop of one item."""
    if RESCAN in parts:
        yield from resolve_rescanned_character(parts[PHASE1][1], parts[RESCAN][1], aligner, names)
        return
    for f, label, source in resolve_item(category, parts[PHASE1][1], aligner, names):
        yield PHASE1, f, label, source


def ocr_comparable(field, raw):
    """Artifact level dumps "engine1 | engine2" parsed values; keep the level engine's."""
    if field == "level" and "|" in raw:
        return raw.split("|")[0]
    return raw


def level_digits(text):
    return re.sub(r"[^0-9/]", "", normalize(text))


def ocr_matches(field, ocr_text, label):
    # Weapon/artifact level dumps are re-rendered from parsed digits, so only
    # the digits are comparable across fields named "level".
    if field == "level":
        return level_digits(ocr_text) == level_digits(label)
    return normalize(ocr_text) == normalize(label)


def collect(run_dirs, gt, names):
    records = []
    for run_dir in run_dirs:
        for root, category, scanned, items in iter_items(run_dir):
            root_path = os.path.normcase(os.path.abspath(root))
            run = os.path.basename(os.path.normpath(root)) + "_" + hashlib.sha256(root_path.encode()).hexdigest()[:12]
            aligner = Aligner(gt, scanned if category == "characters" else [])
            for item, parts in items:
                for part, f, label, source in resolve_parts(category, parts, aligner, names):
                    crop_path = os.path.join(parts[part][0], f["crop"])
                    if not os.path.exists(crop_path) or not label:
                        continue
                    records.append({
                        "run": run,
                        "category": CATEGORY_SINGULAR[category],
                        "item": item + ("_rescan" if part == RESCAN else ""),
                        "field": f["field"],
                        "label": label,
                        "source": source,
                        "ocr_text": ocr_comparable(f["field"], f["raw"]),
                        "src_path": crop_path,
                    })
    return records


def field_group(field):
    return "sub" if field.startswith("sub[") else field


def evaluate(records):
    stats = defaultdict(lambda: [0, 0])
    for r in records:
        if r["source"] == "ocr-only":
            continue
        key = (r["run"], r["category"], field_group(r["field"]))
        stats[key][1] += 1
        stats[key][0] += ocr_matches(r["field"], r["ocr_text"], r["label"])
    print(f"{'run':<24} {'category':<10} {'field':<22} {'exact':>7} {'total':>7} {'acc':>8}")
    for (run, cat, field), (ok, n) in sorted(stats.items()):
        print(f"{run:<24} {cat:<10} {field:<22} {ok:>7} {n:>7} {ok / n:>8.2%}")


def expected_values(gt, names):
    chars = {c["key"] for c in gt.get("characters", [])}
    return {
        ("character", "name"): {
            f"{ELEMENT_ZH[(c.get('element') or names.char_elem.get(c['key'], '')).lower()]}元素/{names.character(c['key'])}"
            for c in gt.get("characters", [])
            if names.character(c["key"]) and (c.get("element") or names.char_elem.get(c["key"]))
            and not (c["key"] == "Traveler" and not c.get("element"))
        },
        ("weapon", "name"): {names.weapon.get(w["key"], w["key"]) for w in gt.get("weapons", [])},
        ("weapon", "refinement"): {f"{w['refinement']}精炼{w['refinement']}阶" for w in gt.get("weapons", [])},
        ("weapon", "equip"): {
            f"{names.character(w['location']) or w['location']}已装备" for w in gt.get("weapons", []) if w.get("location")
        },
        ("artifact", "set_name"): {f"{names.set.get(a['setKey'], a['setKey'])}：" for a in gt.get("artifacts", [])},
        ("artifact", "slot"): set(SLOT_ZH.values()),
        ("artifact", "main_stat"): {STAT_ZH[a["mainStatKey"]] for a in gt.get("artifacts", [])},
        ("artifact", "level"): {f"+{a['level']}" for a in gt.get("artifacts", [])},
        ("artifact", "equip"): {
            f"{names.character(a['location']) or a['location']}已装备" for a in gt.get("artifacts", []) if a.get("location")
        },
        ("artifact", "sub"): {
            render_substat(s, inactive)
            for a in gt.get("artifacts", [])
            for s, inactive in [(s, False) for s in a["substats"]] + [(s, True) for s in a.get("unactivatedSubstats", [])]
        },
        ("character", "_keys"): chars,
    }


def build(records, gt, names, out_dir, per_label_cap):
    if os.path.isdir(out_dir):
        shutil.rmtree(out_dir)
    os.makedirs(out_dir)
    kept, per_label = [], Counter()
    # GT-labeled crops first so the cap never spends slots on ocr-only duplicates.
    order = {"gt-verified": 0, "gt-corrected": 1, "ocr-only": 2}
    for r in sorted(records, key=lambda r: order[r["source"]]):
        key = (r["category"], r["field"], r["label"])
        if per_label[key] >= per_label_cap:
            continue
        per_label[key] += 1
        safe_crop = re.sub(r"[\[\]]", "", os.path.basename(r["src_path"]))
        rel = os.path.join("images", field_group(r["field"]), f"{r['run']}_{r['category']}_{r['item']}_{safe_crop}")
        os.makedirs(os.path.join(out_dir, os.path.dirname(rel)), exist_ok=True)
        shutil.copyfile(r["src_path"], os.path.join(out_dir, rel))
        kept.append({**{k: v for k, v in r.items() if k != "src_path"}, "image": rel.replace(os.sep, "/")})

    with open(os.path.join(out_dir, "manifest.jsonl"), "w", encoding="utf-8") as f:
        for r in kept:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    gt_kept = [r for r in kept if r["source"] != "ocr-only"]
    with open(os.path.join(out_dir, "rec_gt.txt"), "w", encoding="utf-8") as f:
        for r in gt_kept:
            f.write(f"{r['image']}\t{r['label']}\n")

    charset = sorted({c for r in gt_kept for c in r["label"]})
    with open(os.path.join(out_dir, "charset.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(charset) + "\n")
    v6_dict = {line.rstrip("\n") for line in open(V6_DICT, encoding="utf-8")}

    observed = defaultdict(set)
    for r in gt_kept:
        observed[(r["category"], field_group(r["field"]))].add(r["label"])
    expected = expected_values(gt, names)
    coverage = {
        "pairs_total": len(kept),
        "pairs_by_source": dict(Counter(r["source"] for r in kept)),
        "label_chars_missing_from_v6_tiny_dict": "".join(c for c in charset if c not in v6_dict),
        "fields": {},
    }
    for cat, field in sorted({(r["category"], field_group(r["field"])) for r in kept}):
        exp = expected.get((cat, field))
        coverage["fields"][f"{cat}.{field}"] = {
            "pairs": sum(1 for r in kept if (r["category"], field_group(r["field"])) == (cat, field)),
            "distinct_gt_labels": len(observed[(cat, field)]),
            "expected_values": len(exp) if exp is not None else None,
            "missing_values": sorted(exp - observed[(cat, field)]) if exp is not None else None,
        }
    with open(os.path.join(out_dir, "coverage.json"), "w", encoding="utf-8") as f:
        json.dump(coverage, f, ensure_ascii=False, indent=2)
    print(json.dumps({k: v for k, v in coverage.items() if k != "fields"}, ensure_ascii=False))
    for name, c in coverage["fields"].items():
        missing = c["missing_values"]
        print(f"{name:<22} pairs={c['pairs']:<6} distinct={c['distinct_gt_labels']:<5} "
              f"expected={c['expected_values']} missing={len(missing) if missing is not None else '-'}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("runs", nargs="+", help="scan run directories (cwd of the scanner: debug_images/ + good_export_*.json)")
    ap.add_argument("--gt", required=True)
    ap.add_argument("--mappings", required=True)
    ap.add_argument("--config", required=True, help="good_config.json with renameable character names")
    ap.add_argument("--out", help="dataset output directory (required unless --eval)")
    ap.add_argument("--eval", action="store_true", help="print per-field raw-OCR accuracy only")
    ap.add_argument("--per-label-cap", type=int, default=30, help="max crops kept per (field, label)")
    args = ap.parse_args()

    gt = load_json(args.gt)
    names = Names(load_json(args.mappings), load_json(args.config))
    records = collect(args.runs, gt, names)
    evaluate(records)
    if args.eval:
        return
    if not args.out:
        ap.error("--out is required unless --eval is given")
    build(records, gt, names, args.out, args.per_label_cap)


if __name__ == "__main__":
    main()
