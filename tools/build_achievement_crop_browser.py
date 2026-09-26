#!/usr/bin/env python3
"""Build a local HTML browser for achievement OCR crop dumps.

Reads debug_images/achievement (or --dir), re-crops title/status from list
dumps using the scanner's pixel bands, recrops categories from full-window
dumps, and writes browse.html next to those images.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
from pathlib import Path

from PIL import Image

ROW_DIVIDER_MAX = 216
LIST_DIVIDER_DARK_RATIO = 0.80
MIN_ROW_HEIGHT = 40
MIN_CARD_HEIGHT = 100
PARTIAL_ROW_HEIGHT_RATIO = 2.0 / 3.0
TRAILING_PARTIAL_RATIO = 0.72
TRAILING_MAX_RATIO = 1.45

# Keep in sync with genshin/src/scanner/achievement/layout.rs
# (x_frac, y_px, w_frac, h_px)
FIELDS = {
    "title": (0.078, 34, 0.50, 30),
    "status": (0.855, 12, 0.140, 108),
}
CATEGORY_ROW_H = 100
LIST_PANEL_GRAY = 200
LIST_ROW_BRIGHT_RATIO = 0.55
CATEGORY_PROBE_X0 = 0.04
CATEGORY_PROBE_X1 = 0.36
CATEGORY_NAME_SHIFT_X = 28
CARD_HEIGHT = 125
CATEGORY_SELECTED_MAX_H = 160

MISS_ROW_RE = re.compile(r"^(c\d{2})_(.+)_miss_(\d{3})_(\d+)\.png$")
LIST_RE = re.compile(r"^(c\d{2})_(.+)_list_(\d{3})\.png$")
FULL_RE = re.compile(r"^(c\d{2})_(.+)_full\.png$")
FIELD_FILE_RE = re.compile(
    r"^(c\d{2})_(.+)_(list|miss)_(\d{3})(?:_(\d+))?_e(\d{2})_(title|status|card)\.png$"
)


def luma(px: tuple[int, int, int]) -> int:
    return (px[0] + px[1] + px[2]) // 3


def is_divider_row(img: Image.Image, y: int) -> bool:
    w = img.width
    x0 = min(w * 12 // 100, w - 1)
    x1 = max(w * 55 // 100, x0 + 1)
    dark = 0
    n = 0
    x = x0
    while x < x1:
        if luma(img.getpixel((x, y))) <= ROW_DIVIDER_MAX:
            dark += 1
        n += 1
        x += 2
    return n > 0 and (dark / n) >= LIST_DIVIDER_DARK_RATIO


def drop_leading_scraps(rows: list[tuple[int, int, int, int]]) -> list[tuple[int, int, int, int]]:
    while len(rows) > 1 and rows[0][3] < MIN_CARD_HEIGHT:
        rows.pop(0)
    return rows


def split_row_bands(img: Image.Image) -> list[tuple[int, int, int, int]]:
    w, h = img.size
    dividers: list[int] = []
    cluster_start = None
    for y in range(h):
        if is_divider_row(img, y):
            if cluster_start is None:
                cluster_start = y
        elif cluster_start is not None:
            dividers.append(y)
            cluster_start = None

    rows: list[tuple[int, int, int, int]] = []
    prev = 0
    for divider in dividers:
        gap = divider - prev
        if gap >= MIN_ROW_HEIGHT:
            rows.append((0, prev, w, divider - prev))
            prev = divider
        elif prev == 0:
            prev = divider
    if h - prev >= MIN_ROW_HEIGHT:
        rows.append((0, prev, w, h - prev))

    rows = drop_leading_scraps(rows)

    merged: list[tuple[int, int, int, int]] = []
    for row in rows:
        if merged and merged[-1][3] < MIN_CARD_HEIGHT:
            px, py, _, _ = merged[-1]
            bottom = row[1] + row[3]
            merged[-1] = (px, py, w, bottom - py)
        else:
            merged.append(row)
    rows = merged
    if not rows:
        return rows
    heights = sorted(r[3] for r in rows)
    median = heights[len(heights) // 2]
    while len(rows) > 1 and rows[0][3] < median * PARTIAL_ROW_HEIGHT_RATIO:
        rows.pop(0)
    while len(rows) > 1 and (
        rows[-1][3] < median * TRAILING_PARTIAL_RATIO
        or rows[-1][3] > median * TRAILING_MAX_RATIO
    ):
        rows.pop()
    return [r for r in rows if r[3] >= 16]


def crop_px(img: Image.Image, x_frac: float, y: int, w_frac: float, h: int) -> Image.Image | None:
    x = int(round(img.width * x_frac))
    w = int(round(img.width * w_frac))
    if w <= 0 or h <= 0 or x >= img.width or y >= img.height:
        return None
    w = min(w, img.width - x)
    h = min(h, img.height - y)
    if w <= 0 or h <= 0:
        return None
    return img.crop((x, y, x + w, y + h))


def crop_title(row: Image.Image) -> Image.Image | None:
    x0 = int(round(row.width * FIELDS["title"][0]))
    # Same narrow probe as split.rs title_ink_top: short titles never fill
    # the full title column, so a wide ratio locks onto the subtitle.
    x1 = min(row.width, x0 + 160)
    y0 = min(12, max(0, row.height - 1))
    y1 = min(80, row.height)
    px = row.load()
    top = None
    for y in range(y0, y1):
        dark = 0
        n = 0
        x = x0
        while x < x1:
            r, g, b = px[x, y]
            if (r + g + b) // 3 <= 190:
                dark += 1
            n += 1
            x += 2
        if n > 0 and dark / n >= 0.08:
            top = y
            break
    y = max(0, (top - 4) if top is not None else FIELDS["title"][1])
    return crop_px(row, FIELDS["title"][0], y, FIELDS["title"][2], FIELDS["title"][3])


def crop_card(row: Image.Image) -> Image.Image | None:
    if row.height >= CARD_HEIGHT:
        y = row.height - CARD_HEIGHT
        return row.crop((0, y, row.width, y + CARD_HEIGHT))
    if row.height < 40:
        return None
    return row


def is_progress_banner(row: Image.Image) -> bool:
    if row.width < 32 or row.height < 16:
        return False
    for y_num in (4, 5, 6, 7, 8):
        y = min(row.height * y_num // 10, row.height - 1)
        gold = 0
        x = 0
        while x < row.width:
            p = row.getpixel((x, y))
            if p[0] > 180 and p[1] > 140 and p[2] < 110:
                gold += 1
            x += 2
        if gold > row.width / 16:
            return True
    return False


def item(
    src: str,
    intent: str,
    cat: str,
    label: str,
    frame: int | None,
    entry: int | None,
    source: str,
) -> dict:
    return {
        "src": src.replace("\\", "/"),
        "intent": intent,
        "cat": cat,
        "label": label,
        "frame": frame,
        "entry": entry,
        "source": source,
    }


def save_field_crops(
    row: Image.Image,
    out_dir: Path,
    stem: str,
    entry: int,
    source: str,
    cat: str,
    label: str,
    frame: int | None,
    items: list[dict],
) -> None:
    for intent, frac in FIELDS.items():
        if intent == "title":
            crop = crop_title(row)
        else:
            crop = crop_px(row, *frac)
        if crop is None:
            continue
        name = f"{stem}_e{entry:02d}_{intent}.png"
        crop.save(out_dir / name)
        items.append(item(f"fields/{name}", intent, cat, label, frame, entry, source))
    if (card := crop_card(row)) is not None:
        name = f"{stem}_e{entry:02d}_card.png"
        card.save(out_dir / name)
        items.append(item(f"fields/{name}", "card", cat, label, frame, entry, source))


def parse_field_file(path: Path, items: list[dict]) -> None:
    m = FIELD_FILE_RE.match(path.name)
    if not m:
        return
    cat, label, kind, frame_s, _idx, entry_s, intent = m.groups()
    items.append(
        item(
            f"fields/{path.name}",
            intent,
            cat,
            label,
            int(frame_s),
            int(entry_s),
            "list",
        )
    )


def longest_run(values: list[float], threshold: float, gap: int) -> tuple[int, int] | None:
    best = (0, 0)
    start = None
    dark = 0
    for y, v in enumerate(values):
        if v >= threshold:
            if start is None:
                start = y
            dark = 0
        elif start is not None:
            dark += 1
            if dark > gap:
                end = y - dark
                length = end - start + 1
                if length > best[1]:
                    best = (start, length)
                start = None
                dark = 0
    if start is not None:
        length = len(values) - start
        if length > best[1]:
            best = (start, length)
    return best if best[1] else None


def detect_selected_category_rect(image: Image.Image) -> tuple[int, int, int, int] | None:
    w, h = image.size
    x0 = int(w * CATEGORY_PROBE_X0)
    x1 = int(w * CATEGORY_PROBE_X1)
    px = image.load()
    samples = max(1, (x1 - x0 + 1) // 2)
    row_bright = []
    for y in range(h):
        count = 0
        x = x0
        while x < x1:
            if luma(px[x, y]) >= LIST_PANEL_GRAY:
                count += 1
            x += 2
        row_bright.append(count / samples)
    hit = longest_run(row_bright, LIST_ROW_BRIGHT_RATIO, 8)
    if not hit:
        return None
    y0, panel_h = hit
    if panel_h < 24 or panel_h > CATEGORY_SELECTED_MAX_H:
        return None
    if panel_h < CATEGORY_ROW_H:
        extra = CATEGORY_ROW_H - panel_h
        y0 = max(0, y0 - extra)
        panel_h = CATEGORY_ROW_H
    inset_x = 12
    inset_y = 6
    x = x0 + inset_x + CATEGORY_NAME_SHIFT_X
    return (
        x,
        y0 + inset_y,
        max(8, x1 - x),
        max(8, panel_h - inset_y * 2),
    )


def parse_existing(path: Path, items: list[dict]) -> None:
    return


def generate_from_lists(src_dir: Path, field_dir: Path, items: list[dict]) -> None:
    for path in sorted(src_dir.glob("*_list_*.png")):
        m = LIST_RE.match(path.name)
        if not m:
            continue
        cat, label, frame_s = m[1], m[2], m[3]
        frame = int(frame_s)
        img = Image.open(path).convert("RGB")
        rows = split_row_bands(img)
        entry = 0
        for x, y, w, h in rows:
            row = img.crop((x, y, x + w, y + h))
            if is_progress_banner(row):
                continue
            entry += 1
            stem = f"{cat}_{label}_list_{frame_s}"
            save_field_crops(row, field_dir, stem, entry, "list", cat, label, frame, items)

    for path in sorted(src_dir.glob("*_full.png")):
        m = FULL_RE.match(path.name)
        if not m:
            continue
        cat, label = m[1], m[2]
        img = Image.open(path).convert("RGB")
        rect = detect_selected_category_rect(img)
        if not rect:
            continue
        x, y, w, h = rect
        crop = img.crop((x, y, x + w, y + h))
        name = f"{cat}_{label}_category.png"
        crop.save(field_dir / name)
        items.append(item(f"fields/{name}", "category", cat, label, None, None, "list"))


HTML = r"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Achievement crop browser</title>
<style>
  :root {
    --bg: #14151a;
    --panel: #1e2028;
    --line: #2e3140;
    --text: #e8e6df;
    --muted: #9a9588;
    --accent: #c4a574;
    --chip: #2a2d38;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    font: 13px/1.4 "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif;
    background: var(--bg);
    color: var(--text);
  }
  header {
    position: sticky; top: 0; z-index: 5;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    padding: 10px 16px 12px;
  }
  h1 { font-size: 15px; font-weight: 600; margin: 0 0 8px; }
  h1 span { color: var(--muted); font-weight: 400; margin-left: 8px; }
  .row { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin-top: 6px; }
  .lbl { color: var(--muted); margin-right: 4px; }
  button, select, input {
    background: var(--chip);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 4px 10px;
    font: inherit;
    cursor: pointer;
  }
  button.active { background: var(--accent); color: #1a140c; border-color: var(--accent); }
  input[type=search] { min-width: 180px; cursor: text; }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
    padding: 16px;
  }
  .card {
    background: var(--panel);
    border: 1px solid var(--line);
    overflow: visible;
    cursor: pointer;
  }
  .card img {
    display: block;
    width: 100%;
    height: auto;
    max-height: 160px;
    object-fit: contain;
    object-position: left center;
    background: #0f1014;
    border-radius: 0;
  }
  .card.tall img { max-height: 260px; }
  .meta { padding: 6px 8px 8px; color: var(--muted); font-size: 12px; }
  .meta b { color: var(--text); font-weight: 600; }
  .empty { padding: 48px 16px; text-align: center; color: var(--muted); }
  dialog {
    border: 1px solid var(--line);
    background: var(--panel);
    color: var(--text);
    border-radius: 0;
    max-width: min(96vw, 1200px);
    padding: 0;
  }
  dialog::backdrop { background: rgba(0,0,0,.72); }
  dialog img { display: block; max-width: 96vw; max-height: 80vh; background: #0f1014; }
  dialog .bar { padding: 8px 12px; display: flex; justify-content: space-between; gap: 12px; }
  kbd { background: var(--chip); border: 1px solid var(--line); border-radius: 4px; padding: 0 5px; }
</style>
</head>
<body>
<header>
  <h1>Achievement crops <span id="count"></span></h1>
  <div class="row" id="intents"></div>
  <div class="row" id="entryRow">
    <span class="lbl">Entry</span>
    <div id="entries"></div>
  </div>
  <div class="row">
    <span class="lbl">Category</span>
    <select id="cat"><option value="">All</option></select>
    <input id="q" type="search" placeholder="Filter label / file">
  </div>
</header>
<div class="grid" id="grid"></div>
<div class="empty" id="empty" hidden>No crops match these filters.</div>
<dialog id="dlg">
  <div class="bar">
    <span id="dlgcap"></span>
    <span><kbd>←</kbd> <kbd>→</kbd> <kbd>Esc</kbd></span>
  </div>
  <img id="dlgimg" alt="">
</dialog>
<script>
const ITEMS = __ITEMS__;
const INTENTS = [
  ["title", "Title"],
  ["status", "Status"],
  ["category", "Category"],
  ["card", "Full card"],
];
const TALL = new Set(["card"]);
const WIDE = new Set(["title", "status", "category"]);

const state = { intent: "title", entry: "", cat: "", q: "" };
let shown = [];
let cur = 0;

function unique(arr) { return [...new Set(arr)]; }

function setup() {
  const intents = document.getElementById("intents");
  INTENTS.forEach(([id, label], i) => {
    const b = document.createElement("button");
    b.textContent = `${label} (${ITEMS.filter(x => x.intent === id).length})`;
    b.dataset.id = id;
    if (i === 0) b.classList.add("active");
    b.onclick = () => {
      state.intent = id;
      const hasEntry = ITEMS.some(x => x.intent === id && x.entry != null);
      document.getElementById("entryRow").style.display = hasEntry ? "flex" : "none";
      if (!hasEntry) {
        state.entry = "";
        setActive(document.getElementById("entries"), "");
      }
      setActive(intents, id);
      render();
    };
    intents.appendChild(b);
  });

  const entries = document.getElementById("entries");
  const maxEntry = Math.max(0, ...ITEMS.map(x => x.entry || 0));
  const eb = document.createElement("button");
  eb.textContent = "All";
  eb.dataset.id = "";
  eb.classList.add("active");
  eb.onclick = () => { state.entry = ""; setActive(entries, ""); render(); };
  entries.appendChild(eb);
  for (let i = 1; i <= Math.min(maxEntry, 12); i++) {
    const b = document.createElement("button");
    b.textContent = i === 1 ? "1st" : i === 2 ? "2nd" : i === 3 ? "3rd" : `${i}th`;
    b.dataset.id = String(i);
    b.onclick = () => { state.entry = String(i); setActive(entries, String(i)); render(); };
    entries.appendChild(b);
  }

  const cats = unique(ITEMS.map(x => x.cat).filter(Boolean)).sort();
  const sel = document.getElementById("cat");
  cats.forEach(c => {
    const o = document.createElement("option");
    const label = ITEMS.find(x => x.cat === c && x.label && x.label !== "before" && x.label !== "after");
    o.value = c;
    o.textContent = label ? `${c} ${label.label}` : c;
    sel.appendChild(o);
  });
  sel.onchange = () => { state.cat = sel.value; render(); };
  document.getElementById("q").oninput = (e) => { state.q = e.target.value.trim().toLowerCase(); render(); };

  const dlg = document.getElementById("dlg");
  dlg.addEventListener("click", (e) => { if (e.target === dlg) dlg.close(); });
  document.addEventListener("keydown", (e) => {
    if (!dlg.open) return;
    if (e.key === "ArrowRight") show(cur + 1);
    if (e.key === "ArrowLeft") show(cur - 1);
    if (e.key === "Escape") dlg.close();
  });
  render();
}

function setActive(root, id) {
  [...root.querySelectorAll("button")].forEach(b => b.classList.toggle("active", b.dataset.id === id));
}

function match(it) {
  if (state.intent && it.intent !== state.intent) return false;
  if (state.entry && String(it.entry || "") !== state.entry) return false;
  if (state.cat && it.cat !== state.cat) return false;
  if (state.q) {
    const blob = `${it.src} ${it.label} ${it.cat} ${it.intent}`.toLowerCase();
    if (!blob.includes(state.q)) return false;
  }
  return true;
}

function cap(it) {
  const bits = [it.intent, it.cat, it.label];
  if (it.frame != null) bits.push(`frame ${it.frame}`);
  if (it.entry != null) bits.push(`entry ${it.entry}`);
  return bits.filter(Boolean).join(" · ");
}

function render() {
  shown = ITEMS.filter(match);
  document.getElementById("count").textContent = `${shown.length} / ${ITEMS.length}`;
  const grid = document.getElementById("grid");
  const empty = document.getElementById("empty");
  grid.innerHTML = "";
  empty.hidden = shown.length > 0;
  shown.forEach((it, i) => {
    const el = document.createElement("div");
    el.className = "card";
    if (TALL.has(it.intent)) el.classList.add("tall");
    if (WIDE.has(it.intent)) el.classList.add("wide");
    el.innerHTML = `<img loading="lazy" alt="" src="${encodeURI(it.src)}"><div class="meta"><b>${escapeHtml(it.label || it.cat)}</b><br>${escapeHtml(cap(it))}</div>`;
    el.onclick = () => show(i);
    grid.appendChild(el);
  });
}

function show(i) {
  if (!shown.length) return;
  cur = (i + shown.length) % shown.length;
  const it = shown[cur];
  document.getElementById("dlgimg").src = encodeURI(it.src);
  document.getElementById("dlgcap").textContent = `${cur + 1}/${shown.length}  ${cap(it)}`;
  document.getElementById("dlg").showModal();
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;","'":"&#39;"}[c]));
}

setup();
</script>
</body>
</html>
"""


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--dir",
        default=str(Path("debug_images/achievement")),
        help="Dump directory (default: debug_images/achievement)",
    )
    parser.add_argument(
        "--keep-fields",
        action="store_true",
        help="Reuse existing fields/ crops instead of re-splitting list dumps",
    )
    args = parser.parse_args()
    src_dir = Path(args.dir).resolve()
    if not src_dir.is_dir():
        raise SystemExit(f"missing dump dir: {src_dir}")

    field_dir = src_dir / "fields"
    items: list[dict] = []
    for path in sorted(src_dir.glob("*.png")):
        parse_existing(path, items)

    if args.keep_fields and field_dir.is_dir():
        for path in sorted(field_dir.glob("*.png")):
            parse_field_file(path, items)
    else:
        if field_dir.exists():
            shutil.rmtree(field_dir)
        field_dir.mkdir()
        generate_from_lists(src_dir, field_dir, items)

    source_rank = {"dump": 0, "list": 1, "miss": 2, "badname": 3, "bottom": 4, "idle": 5}
    items.sort(
        key=lambda it: (
            it["intent"],
            it["cat"],
            it["frame"] if it["frame"] is not None else -1,
            it["entry"] if it["entry"] is not None else -1,
            source_rank.get(it["source"], 9),
            it["src"],
        )
    )

    html = HTML.replace("__ITEMS__", json.dumps(items, ensure_ascii=False))
    out = src_dir / "browse.html"
    out.write_text(html, encoding="utf-8")
    by_intent: dict[str, int] = {}
    for it in items:
        by_intent[it["intent"]] = by_intent.get(it["intent"], 0) + 1
    print(f"wrote {out} ({len(items)} crops)")
    for k, v in sorted(by_intent.items()):
        print(f"  {k}: {v}")


if __name__ == "__main__":
    main()
