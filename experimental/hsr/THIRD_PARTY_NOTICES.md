# Third-party notices and research provenance

`HSRScannerExperimental` is licensed under GPL-2.0-or-later, as declared in
this crate's `Cargo.toml`. The following projects were checked at pinned
revisions to understand current Honkai: Star Rail behavior and interoperability.

## External normalized reference input

Live operations consume a separately generated, manifest-verified
`ggstarrail-reference` bundle through the GIlore/GGStarRail provider boundary.
The full TurnBasedGameData-derived catalog is not bundled or redistributed by
this crate. Its manifest remains the source of the exact upstream revision and
license-status metadata; the committed `tests/fixtures/gilore_bundle` contains
only a deliberately small sanitized test subset and is rejected by live paths.

## MIT-licensed interoperability references

- [Reliquary Archiver](https://github.com/IceDynamix/reliquary-archiver/tree/cb109f17a4a15b7604cfe9d078a8735e7735cd25),
  revision `cb109f17a4a15b7604cfe9d078a8735e7735cd25` (v0.18.0), MIT.
- [Reliquary](https://github.com/IceDynamix/reliquary/tree/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b),
  revision `d5cf3b7e7e66470d2d8efff6676aa18762b21d3b` (v23.0.0), MIT.
- [Fribbels Star Rail Optimizer](https://github.com/fribbels/hsr-optimizer/tree/df630a0488a64eeb740e4e0c14f265d96b9f6f8f),
  revision `df630a0488a64eeb740e4e0c14f265d96b9f6f8f`, MIT.

The experimental packet-capture adapter interoperates with a separately
installed Reliquary Archiver executable and imports a sanitized form of its
documented export. Those projects are not bundled or redistributed here.

## GPL-licensed research-only references

- [Kel-Z HSR-Scanner](https://github.com/kel-z/HSR-Scanner/tree/cad008e2509b6b086fd892b140c8b277424fcb03),
  revision `cad008e2509b6b086fd892b140c8b277424fcb03` (v1.5), GPL-3.0.
- [Original YAS Honkai: Star Rail scanner](https://github.com/wormtql/yas/tree/730d8845505c04c7f4c91709de076503401ff40c/yas-starrail),
  revision `730d8845505c04c7f4c91709de076503401ff40c`, GPL-2.0.

These two projects were consulted only as historical behavioral references.
No source code, assets, OCR data, or bundled game data from either project was
copied into this experimental crate. HSR-specific capture geometry, parsing,
normalization, and manager behavior were independently implemented against this
crate's contracts and the pinned interoperability references above. The runtime
intentionally reuses this repository's existing GPL `yas_core` capture, input,
and PaddleOCR primitives where their contracts match; those components remain
covered by the root project's notices and source license.
