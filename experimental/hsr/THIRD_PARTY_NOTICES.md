# Third-party notices and research provenance

The `hsr_scanner` library is licensed under GPL-2.0-or-later, as declared in
this crate's `Cargo.toml`. It is linked into the shared GOODScanner application
targets and does not produce a separate HSR executable.

## Native achievement packet boundary

- [auto-reliquary](https://github.com/hashblen/auto-reliquary/tree/bc23b48cb3b1b994a5d4405cefea42eb0e1d3735),
  version 1.2.0 at revision
  `bc23b48cb3b1b994a5d4405cefea42eb0e1d3735`, MIT License, Copyright (c)
  2024 IceDynamix. The dependency is pinned to this revision in `Cargo.toml`.
- [stardb-exporter](https://github.com/juliuskreutz/stardb-exporter/tree/50c04597d37cf366290de6e316aaca98dd57acfc)
  was used as a pinned interoperability reference for the Windows pktmon ports,
  completed-status behavior, and dispatch-key file shape. At that revision its
  package manifest and repository root do not declare a project license, so no
  license is inferred here.

The embedded `keys/hsr.json` interoperability dataset was copied byte-for-byte
from `stardb-exporter/keys/hsr.json` at revision
`50c04597d37cf366290de6e316aaca98dd57acfc`. Its SHA-256 is
`85a98f5abf9b4041d6752e8f60b6db760d5a9753ad73874a9d5744f9c1d7944a`.
It contains only version-indexed protocol dispatch keys: no account, player,
session, achievement, or captured packet data.

This project uses auto-reliquary's transport/session decoder but independently
decodes the decrypted achievement command. In particular, it does not use the
upstream single-achievement sentinel matcher. Repeated fields are inferred
against the complete public achievement-ID set supplied through the GIlore
reference bundle, and only normalized completed IDs leave the capture boundary.

## Embedded normalized reference and optional override

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

An advanced user may explicitly select a separately generated,
manifest-verified bundle through the same provider boundary. It must pass the
same production completeness checks; an invalid override does not silently
fall back to the embedded snapshot. Small synthetic or sanitized test bundles
remain rejected by production-sized live paths.

## Optional legacy offline import interoperability

- [Reliquary Archiver](https://github.com/IceDynamix/reliquary-archiver/tree/cb109f17a4a15b7604cfe9d078a8735e7735cd25),
  revision `cb109f17a4a15b7604cfe9d078a8735e7735cd25` (v0.18.0), MIT.
- [Reliquary](https://github.com/IceDynamix/reliquary/tree/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b),
  revision `d5cf3b7e7e66470d2d8efff6676aa18762b21d3b` (v23.0.0), MIT.
- [Fribbels Star Rail Optimizer](https://github.com/fribbels/hsr-optimizer/tree/df630a0488a64eeb740e4e0c14f265d96b9f6f8f),
  revision `df630a0488a64eeb740e4e0c14f265d96b9f6f8f`, MIT.

The library can normalize an existing Reliquary/Fribbels v4 JSON document.
This optional legacy import code is not part of the normal native achievement
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
