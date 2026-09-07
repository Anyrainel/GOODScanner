# Third-party notices and research provenance

The `hsr_scanner` library is licensed under GPL-2.0-or-later, as declared in
this crate's `Cargo.toml`. It is linked into the shared GOODScanner application
targets and does not produce a separate HSR executable.

## Native inventory and achievement packet boundary

The transport under `src/network/` and the seven inventory record types under
`src/packet_capture/proto/` are adapted from
[IceDynamix/reliquary](https://github.com/IceDynamix/reliquary/tree/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b),
v23.0.0, commit `d5cf3b7e7e66470d2d8efff6676aa18762b21d3b` (2026-08-28,
HSR 4.5). Its MIT license is reproduced in `LICENSE-reliquary.txt`.

The local adaptation preserves independent KCP conversations, removes raw
packet/seed trace logging, bounds malformed frames, and infers the login seed
from the protobuf shape instead of the upstream token command ID/field tag.
Inventory classification searches repeated fields inside bounded protobuf
wrappers and validates candidates against bundled public definitions. No
executable offsets, memory reads, injected code, or external helper process
are used. The inner inventory record schemas still have explicit protobuf tags;
this does not promise compatibility with arbitrary inner-schema changes.

[reliquary-archiver](https://github.com/IceDynamix/reliquary-archiver/tree/cb109f17a4a15b7604cfe9d078a8735e7735cd25),
v0.18.0, was used to verify base/path character joining, equipment fields, and
relic main/sub-affix conversion. `assets/packet_affixes.json` is a compact
projection of RelicMainAffixConfig, RelicSubAffixConfig, and
MultiplePathAvatarConfig from the same public GIlore source revision
`8cdb905dc2f8e6fffa9be4eb07af3e34435d6091` used by the bundled reference.
The substat calculation is `count * base + step * stepValue`, with ratios
converted to percentage points like the existing HSR observation contract.

The unchanged `keys/hsr.json` public interoperability dataset comes from
[stardb-exporter](https://github.com/juliuskreutz/stardb-exporter/tree/50c04597d37cf366290de6e316aaca98dd57acfc).
SHA-256: `85a98f5abf9b4041d6752e8f60b6db760d5a9753ad73874a9d5744f9c1d7944a`.
It contains version-indexed dispatch keys, never account/session data.
The prior auto-reliquary dependency has been replaced by the local audited
transport. Achievement classification remains independent and accepts packed
Quest counters without retaining them in the completed-achievement export.

## Embedded normalized reference

GOODScanner and GOODCapture embed a compact normalized snapshot generated from
the manifest-verified GIlore/GGStarRail reference boundary at GIlore commit
`7ef3650a63622c204b89234406c99dc221e01d85` and public source revision
`8cdb905dc2f8e6fffa9be4eb07af3e34435d6091`. The source manifest SHA-256 is
`9899cc8fdde578cdbd744ec9f8b2705cd2f11d43670232e871f489fc3d549b5f`;
the deterministic embedded document SHA-256 is
`46fdddc9252046580154823d596315fb09d88dab570ffa958787286c99d76782`.
The snapshot contains the public identifiers, names, aliases, and gameplay
metadata needed for strict normalization, including 1,921 public achievement
definition IDs. It contains no icon binaries, account/session/capture data,
server item identifiers, or packet data. The source manifest remains the
authority for upstream revision and license-status metadata.

All application flows use the bundled reference automatically. The library's
manifest-verified file provider remains available to offline tooling and tests;
it is not a setting that app users need to configure.

## Optional legacy offline import interoperability

- [Reliquary Archiver](https://github.com/IceDynamix/reliquary-archiver/tree/cb109f17a4a15b7604cfe9d078a8735e7735cd25),
  revision `cb109f17a4a15b7604cfe9d078a8735e7735cd25` (v0.18.0), MIT.
- [Reliquary](https://github.com/IceDynamix/reliquary/tree/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b),
  revision `d5cf3b7e7e66470d2d8efff6676aa18762b21d3b` (v23.0.0), MIT.
- [Fribbels Star Rail Optimizer](https://github.com/fribbels/hsr-optimizer/tree/df630a0488a64eeb740e4e0c14f265d96b9f6f8f),
  revision `df630a0488a64eeb740e4e0c14f265d96b9f6f8f`, MIT.

The library can normalize an existing Reliquary/Fribbels v4 JSON document.
This optional legacy import code is not part of the normal native
capture flow. GOODScanner and GOODCapture do not download, require, or invoke a
Reliquary Archiver helper.

## GPL-licensed research-only references

- [Kel-Z HSR-Scanner](https://github.com/kel-z/HSR-Scanner/tree/cad008e2509b6b086fd892b140c8b277424fcb03),
  revision `cad008e2509b6b086fd892b140c8b277424fcb03` (v1.5), GPL-3.0.
- [Original YAS Honkai: Star Rail scanner](https://github.com/wormtql/yas/tree/730d8845505c04c7f4c91709de076503401ff40c/yas-starrail),
  revision `730d8845505c04c7f4c91709de076503401ff40c`, GPL-2.0.

These projects were consulted only as historical behavioral references. No
source code, assets, OCR data, or bundled game data from either project was
copied into this crate. HSR-specific geometry, parsing, normalization, and
manager behavior were implemented against this crate's contracts. The runtime
reuses this repository's GPL `yas_core` capture, input, and PaddleOCR
primitives where their contracts match; those components remain covered by the
root project's notices and source license.
