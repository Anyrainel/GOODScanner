# Star Rail capture

Select **Star Rail → Capture**, start capture as administrator, then log in
from the game's title screen. Characters, Light Cones, and Relics (including
Planar Ornaments), and Achievements each have a checkbox, matching the Genshin
export settings. All four default to selected; selections persist independently
of OCR settings. Unchecked categories are excluded from the export and completion
requirements. Capture stops and saves the selected export files after the requested data arrives. The default destination is beside the
executable; existing saved output destinations are retained. The result shows
the actual file path.

All HSR app flows load the bundled game reference automatically. The application
does not download reference data from `ggartifact.com` or `hsr.ggartifact.com`;
`hsr.ggartifact.com` is the HSR website. The embedded 4.5 reference includes all
97 characters, including collaboration IDs 1014, 1015, 1508, and 1509. GIlore
merges the separate `*LD.json` character, progression, skill, trace, and item tables.
Updating the website alone does not update an already installed executable. There is no
reference-folder selection or separate HSR output-folder setup in the UI.
Application config v1 migrates to v2 by dropping the obsolete developer
reference path while preserving scan preferences, output path, and navigation.


## Advanced settings and export files

Like Genshin, **Advanced** contains **Dump decrypted packets** and **Only keep
latest export**. Both default off and persist independently of Genshin and OCR.
Packet diagnostics go beside the executable in `debug_capture/hsr/hsr_<session>/`,
with numbered `.bin` protobuf bodies. Each capture gets its own folder. This is
an explicit diagnostic dump, separate from sanitized exports; it is not a PCAP
or ETL transport recording. A write failure is surfaced with its full path.

Each completed capture writes the applicable files with one shared timestamp:

- `star_rail_fribbels_<timestamp>.json`: HSR-Scanner format v4 inventory,
  including all six relic slots, numeric definition IDs as strings, exact
  substat values, equipment locations, and captured character skills/traces.
- `star_rail_achievements_<timestamp>.json`: StarDB's
  `{ "hsr_achievements": [completed IDs] }` import shape. Paste the JSON contents
  into StarDB's importer.
- `star_rail_export_<timestamp>.json`: the existing GGStarRail v3 companion,
  including selected inventory and achievements with their coverage metadata.

No file claims an unchecked category was completely scanned. Inventory-only
captures do not write an achievement file; achievement-only captures do not
write a Fribbels inventory file. Existing exports are removed only after every
new file has been written and synced. Cleanup matches generated HSR filenames
and dates, excludes newer/current exports, directories and symlinks, and never
matches `genshin_export_` or packet dumps.

Fribbels currently allowlists `HSR-Scanner` as a source string. The interchange
file uses that required v4 compatibility discriminator and its compatible
build identifier; a separate `generator` object identifies GOODScanner and
its capture revision. It does not claim to have been captured by Kel-Z's tool.
Server item IDs are replaced with local synthetic identifiers; account UID is
null. The primary format references are
[Fribbels' parser](https://github.com/fribbels/hsr-optimizer/blob/main/src/lib/importer/kelzFormatParser.tsx),
[import configuration](https://github.com/fribbels/hsr-optimizer/blob/main/src/lib/importer/importConfig.ts),
and [StarDB's exporter](https://github.com/juliuskreutz/stardb-exporter/blob/master/src/ui/achievements.rs).

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
of exporting an authoritative empty collection. The existing GGStarRail schema
retains character level/ascension/eidolon, equipment progression/location/flags,
relic stats, and completed achievement IDs. It does not yet carry skill-tree
or relic-reroll-preview data.

## Verification

Automated coverage includes interleaved encrypted KCP conversations, inferred
seed tag changes, reordering/duplicates/truncation, rotated container tags,
nested wrappers, packed Quest counters, incomplete/ambiguous data, multipath
normalization, timeout/cancellation, and off-screen mixed-export completion.
The native Capture screen was inspected in English and Chinese.

A fresh login against the rebuilt binary remains the final live check. Replay
fixtures are synthetic, not captured player traffic. No live-success claim is
made from those fixtures.
