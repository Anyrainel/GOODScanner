# Star Rail capture

Select **Star Rail → Capture**, start capture as administrator, then log in
from the game's title screen. Characters, Light Cones, and Relics (including
Planar Ornaments), and Achievements each have a checkbox, matching the Genshin
export settings. All four default to selected; selections persist independently
of OCR settings. Unchecked categories are excluded from the export and completion
requirements. Capture stops and saves one export file after the requested data arrives. The default destination is beside the
executable; existing saved output destinations are retained. The result shows
the actual file path.

All HSR app flows download the current reference from
`https://hsr.ggartifact.com/good/hsr_data_cache.json`. Like Genshin, the app
checks again after two hours and keeps a validated local cache, separately at
`data/hsr/hsr_data_cache.json`. A network failure uses the last valid cache;
a first download failure reports an error. No installed game-reference bundle
is used as a fallback. There is no reference-folder or domain setting.

**Refresh game data** bypasses the two-hour cache in Capture's Advanced section,
and is also available in the HSR Scanner and Manager. Failed or malformed refreshes
preserve the previous cache and report the error. Character, Light Cone, Relic,
achievement, affix, and character-form mapping updates all arrive together.

GIlore's normal `hsr_data reference` command generates `capture_data_cache.json`
from the same validated source and normalized bundle. GGStarRail's normal
`npm run data:update` copies it to `public/good/hsr_data_cache.json`; the website
build checks its revision and catalog coverage against the website data. Publishing
that file updates installed clients without rebuilding GOODCapture. Protocol code
changes can still require an application update. The old embedded reference API
remains only for deterministic offline fixtures and historical build tooling.

Application config v1 migrates to v2 by dropping the obsolete developer
reference path while preserving scan preferences, output path, and navigation.


## Advanced settings and export files

Like Genshin, **Advanced** contains **Dump decrypted packets** and **Only keep
latest export**. Both default off and persist independently of Genshin and OCR.
Packet diagnostics go beside the executable in `debug_capture/hsr/hsr_<session>/`,
with numbered `.bin` protobuf bodies. Each capture gets its own folder. This is
an explicit diagnostic dump, separate from sanitized exports; it is not a PCAP
or ETL transport recording. A write failure is surfaced with its full path.

Each completed capture writes one file: `star_rail_export_<timestamp>.json`.
That file is HSR-Scanner format v4 plus GOODScanner extensions (achievements,
Trailblazer gender and current path). See [`HSR_EXPORT.md`](HSR_EXPORT.md).
Unchecked categories are omitted rather than reported as empty-complete.
Existing `star_rail_export_`, `star_rail_fribbels_`, and
`star_rail_achievements_` files are removed only after the new file has been
written and synced. Cleanup matches generated HSR filenames and dates,
excludes newer/current exports, directories and symlinks, and never matches
`genshin_export_` or packet dumps.

Fribbels currently allowlists `HSR-Scanner` as a source string. The interchange
file uses that required v4 compatibility discriminator and its compatible
build identifier; a separate `generator` object identifies GOODScanner and
its capture revision. It does not claim to have been captured by Kel-Z's tool.
Server item IDs are replaced with local synthetic identifiers; account UID is
null. The primary format references are
[Fribbels' parser](https://github.com/fribbels/hsr-optimizer/blob/main/src/lib/importer/kelzFormatParser.tsx)
and [import configuration](https://github.com/fribbels/hsr-optimizer/blob/main/src/lib/importer/importConfig.ts).

Validation replayed generated inventory through the current Fribbels v4 parser
with real public item/affix tables (UI notifications and post-parse augmentation
isolated), and through GGStarRail's v4 schema. Live game capture and uploads to
every third-party site are not claimed by these tests.

## Protocol approach

Research checked the public heads of
[Reliquary](https://github.com/IceDynamix/reliquary/tree/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b)
(v23.0.0 / HSR 4.5, 2026-08-28) and
[Reliquary Archiver](https://github.com/IceDynamix/reliquary-archiver/tree/cb109f17a4a15b7604cfe9d078a8735e7735cd25)
(v0.18.0, 2026-08-29). English and Chinese searches also identified
[StarDB Exporter](https://github.com/juliuskreutz/stardb-exporter), its
[Chinese importer](https://stardb.gg/zh-tw/import), and
[HSR Scanner](https://github.com/kel-z/HSR-Scanner). The first two Reliquary
projects provide the maintained packet-capture reference; HSR Scanner uses OCR.
See the [license/provenance notice](../experimental/hsr/THIRD_PARTY_NOTICES.md).

The old native path could export only achievements and stopped as soon as an
achievement response was recognized. Its transport had one shared pair of KCP
decoders, unlike current Reliquary's independent conversations. Its achievement
matcher also rejected the packed counters allowed by current Quest records.
These are code-level defects; without the failing live session they cannot be
claimed as the confirmed cause of every login that waits indefinitely.

The replacement uses:

- pktmon on HSR UDP ports 23301/23302, with cancelable stream ownership.
- Separate KCP and decryption state per conversation. Login seed inference is
  limited to incoming dispatch-encrypted commands, without a token command ID.
- Bounded traversal of valid protobuf wrappers. Every repeated field is tested
  against inventory record types and public reference IDs. Container field
  numbers and byte positions are not assumed.
- Base Avatar progression joined to AvatarPathData using public multipath
  definitions, including Trailblazer/March variants. The complete-roster flag
  distinguishes a full avatar response from a normal avatar update.
- Equipment and Relic normalization, retaining equipped character, lock and
  discard states; affix IDs/roll counts become the existing semantic stat units.
- Separate achievement readiness. All records in a candidate group must parse;
  an invalid member cannot silently disappear from an authoritative snapshot.
- A 120-second deadline with readable errors distinguishing absent traffic,
  unsuccessful decryption, and missing categories. Full errors include packet,
  decrypted-command, and category counts plus the last transport failure.

There are no executable offsets, process-memory reads, injected code, helper
executables, or packet writes. This follows the Genshin capture architecture's
distinction between unknown outer protobuf containers and validated inner types.

## Compatibility limits

The inner inventory schemas are the reviewed HSR 4.5 protobuf types. Command
and outer-field rotation are handled, but arbitrary inner-field or encryption
changes can still require an update. Scalar fields with identical shapes
cannot always be inferred unambiguously from values alone.

The inventory matcher requires nonempty Light Cone and Relic groups together;
missing groups do not prove an empty inventory. Such sessions time out instead
of exporting an authoritative empty collection. Capture writes HSR-Scanner v4;
see [`HSR_EXPORT.md`](HSR_EXPORT.md). OCR and manager flows still use the
GGStarRail schema.

## Verification

Automated coverage includes interleaved encrypted KCP conversations, inferred
seed tag changes, reordering/duplicates/truncation, rotated container tags,
nested wrappers, packed Quest counters, incomplete/ambiguous data, multipath
normalization, timeout/cancellation, and off-screen mixed-export completion.
The native Capture screen was inspected in English and Chinese.

A fresh login against the rebuilt binary remains the final live check. Replay
fixtures are synthetic, not captured player traffic. No live-success claim is
made from those fixtures.
