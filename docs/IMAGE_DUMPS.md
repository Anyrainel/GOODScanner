# Scanner image dumps

`--dump-images` uses one annotation implementation for Genshin and HSR.
The implementation lives in `genshin/src/scanner/common/{annotator,debug_dump,dump_paths}.rs`;
HSR's `annotator.rs` only adapts normalized coordinates and the game namespace.
HSR already depends on this crate for the embedded OCR models.

## Folder ownership

Each enabled scan/manager initialization replaces that game's previous dumps:

```
debug_images/
  gi_characters/0000/
  hsr_characters/0000/
```

After flushing pending writes, initialization clears only directories with the
active game's prefix (`gi_` or `hsr_`). This removes stale higher indices even
when a new scan finds no items. The other game's dumps are retained. There are
no timestamp or game parent folders; new scans reuse the same category/index paths.
Linked category directories are rejected before cleanup. Save a copy outside
these folders before scanning again if a session should join a training archive.

Item folders are reserved with atomic `create_dir`. Repeating an index within
the same scan reserves `0000_2`, `0000_3`, etc., preserving current retries.
Manager artifact reads use `gi_manager_artifacts`, separate from `gi_artifacts`.
Achievement and failure captures also use game-prefixed categories. Code-owned
category identifiers must be lowercase ASCII, digits, or underscores, so path
sanitization cannot merge distinct category owners. Legacy per-field writes
reserve fresh filenames.
The filter-test helper accepts an explicit external output prefix; its caller
owns that destination, and its PNG writes reserve filenames too.

Within each item, context images, annotations, and field crops share one
case-insensitive filename reservation scheme. Repeated screen labels and repeated
field names receive suffixes; fields cannot overwrite `full.png`, `annotated.png`,
or a context image. Labels are sanitized for Windows paths.

## Captures follow detection

The shared `observe_ocr` boundary records every actual inference, including empty
reads, errors, cross-engine reads, and retries on preprocessed inputs. Nested
reader/model adapters record the inference once. It stores the pixels actually
passed to the recognizer, instead of reconstructing a retry from the original
screenshot. Domain field annotations attach names and coordinates to these inputs
when the field is known or the source pixels match exactly. Unidentified probes
remain separate inputs; they are never assigned blindly to the next field.

Pixel detection functions record the region they inspect. This is one crop per
semantic check, rather than a PNG for every pixel in an inner sampling loop.
HSR covers rarity, lock/discard and equipped-glyph evidence, substat ink, trace
nodes, Eidolon nodes, inventory geometry/selection/scrolling, portrait navigation,
and frame/glyph comparisons. Genshin retains its item annotations and also captures
early icon checks, navigation, grid/scroll evidence, panel stability, and achievement
segmentation/comparisons. Pixel inputs go under `detections` in the manifest and
are excluded from OCR training fields.

An item retains its context screenshot(s), annotated screenshot(s), and each
OCR/detection input. A check outside an item writes a standalone crop and metadata,
without duplicating that crop as a full and annotated screenshot. Skips, failures,
and unfinished items preserve their evidence. `flush()` finishes current-thread
evidence and waits for pending writes.

`ocr_fields.json` identifies the game, category, and index, and contains `fields`,
`detections`, `error`, and `final_object`.
OCR fields include the exact crop filename, raw text, parsed/display values,
coordinates, input dimensions, `exact_input`, and `inference_error`. HSR's current final objects are
debug representations; they need a game-specific verified-label adapter before
automatic HSR groundtruth labeling is possible.

## Existing OCR dataset work

`tools/build_ocr_dataset.py` already builds a **Genshin** recognition dataset from
field manifests and a groundtruth GOOD export. It emits images, `manifest.jsonl`,
PaddleOCR `rec_gt.txt`, charset, and coverage reports. Verified labels come from
groundtruth; OCR-only hypotheses are separated for review. Failed inferences are
excluded. It accepts a scanner working directory or its `debug_images` directory,
using the `gi_` categories and excluding HSR. It still reads historical flat and
timestamped dumps. Other offline evaluators need the game-prefixed category paths.

At this audit there is no committed end-to-end custom PPOCR training plan or
training configuration. The dataset builder and live/offline field evaluators are
the existing foundation. Remaining model work includes an HSR groundtruth label
adapter, label review, deduplication, splits by acquisition session/account,
dictionary coverage decisions, training configuration, held-out per-field
evaluation, and ONNX export/runtime parity checks. Capture completeness alone
does not turn OCR hypotheses into reliable training labels.
