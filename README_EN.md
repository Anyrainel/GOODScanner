<div align="center">

# GOODScanner

**[中文](README.md) | English**

Windows scanner, capture, and manager for Genshin Impact and Honkai: Star Rail, based on [yas](https://github.com/wormtql/yas)

The same `GOODScanner.exe` and `GOODCapture.exe` serve both games. Genshin data remains compatible [GOOD v3](https://frzyc.github.io/genshin-optimizer/#/doc); Star Rail data uses the `goodscanner.hsr` v3 contract for GGStarRail.

[![Build](https://github.com/Anyrainel/GOODScanner/actions/workflows/rust.yml/badge.svg)](https://github.com/Anyrainel/GOODScanner/actions)

</div>

## Features

- **Game switcher**: equal-width Genshin and Star Rail choices at the top of the window; each game keeps its own settings and current page
- **Genshin scanning and capture**: existing Character, Weapon, Artifact, and GOOD v3 export behavior remains compatible
- **Genshin achievement capture**: GOODCapture exports the account's completed achievement IDs by default
- **Star Rail scanning**: Characters, Light Cones, Cavern Relics, and Planar Ornaments
- **Star Rail export and manager**: `goodscanner.hsr` v3 export plus attended Relic management preview and confirmation-protected reversible actions
- **Star Rail achievement capture**: GOODCapture captures completed achievements in process; no separate HSR application or packet-capture helper download is required
- **Dual-engine OCR**: PPOCRv4 (general) + PPOCRv5 (level-specific), automatically picks the best result
- **Substat validation**: Roll Solver verifies substat combinations against game mechanics

## Quick Start

### Download

The releases page provides two executables. Both let you switch between Genshin and Star Rail at the top of the window:

- `GOODScanner.exe` — OCR scanning, export, and manager flows for both games
- `GOODCapture.exe` — all of the above plus Genshin data capture and completed-achievement capture for both games

Download them from the [Releases](https://github.com/Anyrainel/GOODScanner/releases) page. **There is no separate HSR executable to download.**

### Achievement export semantics

Genshin continues to store achievements in a backwards-compatible GOOD v3 extension field:

```json
{"achievements":[80001,80002,81001]}
```

The field is a compact array of completed achievement IDs. When present, including as an empty array, importers should replace achievement state. When the user disables the Achievements export option, the field is omitted and importers should preserve existing achievement state. Standard GOOD v3 fields and the format version remain unchanged.

Star Rail uses `schema: "goodscanner.hsr"`, `schemaVersion: 3`. Its optional `achievements` snapshot contains only public achievement IDs and the `completed` status. An omitted field means achievements were not observed and importers must preserve existing state. A present snapshot with `coverage: "complete"` is authoritative replacement data; `entries: []` explicitly confirms zero completed achievements. See the [HSR implementation and contract](docs/HSR_EXPERIMENTAL_FEASIBILITY.md).

### Usage

1. Run the executable you need **as administrator**; launching it without command-line arguments opens the GUI
2. Select Genshin or Star Rail at the top, then open that game's Scanner, Manager, or Capture page
3. For Genshin, enter custom Character names (Traveler, Wanderer, and so on) when first prompted; the existing settings remain in `data/good_config.json`
4. Star Rail settings are kept separately in `data/good_app_config.json`. The shared binaries include a verified GIlore reference snapshot; the reference-folder field is only an advanced override
5. Make sure the selected game is running. **Right-click to abort** during scanning
6. Genshin still writes `GOODv3.json`; choose the Star Rail output folder on its page

The built-in Star Rail reference is the complete public GIlore `1.2.0` snapshot used to validate inventory and achievement IDs. An explicitly selected override must be a complete checksummed GIlore bundle; invalid overrides fail closed instead of falling back silently. No reference-data download is required for normal use.

### Scan Targets

These command-line flags remain Genshin-only and preserve their existing GOOD v3 behavior. By default, all categories are scanned; you can also pick specific ones:

```shell
GOODScanner.exe                    # Scan all
GOODScanner.exe --characters       # Characters only
GOODScanner.exe --weapons          # Weapons only
GOODScanner.exe --artifacts        # Artifacts only
GOODScanner.exe --characters --weapons  # Combine targets
```

## Requirements

- **Administrator privileges** (required for input simulation)
- Genshin scanning supports the **Simplified Chinese** client; Star Rail recognizes English and Chinese UI labels
- **16:9 resolution** recommended (1920x1080, 2560x1440, etc.)
- Do not move the mouse during scanning
- Artifacts below 4-star are skipped by default (adjustable via `--artifact-min-rarity`)

## CLI Options

### Global Options

| Flag | Description |
|------|-------------|
| `-v, --verbose` | Show detailed scan info |
| `--continue-on-failure` | Keep scanning when individual items fail |
| `--log-progress` | Log each item as it is scanned |
| `--output-dir <DIR>` | Output directory (default: `.`) |
| `--ocr-backend <NAME>` | Override OCR backend (ppocrv4 or ppocrv5) |
| `--dump-images` | Save OCR region screenshots to `debug_images/` |

### Scanner Config

| Flag | Description |
|------|-------------|
| `--weapon-min-rarity <N>` | Minimum weapon rarity (default: 3) |
| `--artifact-min-rarity <N>` | Minimum artifact rarity (default: 4) |
| `--char-max-count <N>` | Max characters to scan (0 = unlimited) |
| `--weapon-max-count <N>` | Max weapons to scan (0 = unlimited) |
| `--artifact-max-count <N>` | Max artifacts to scan (0 = unlimited) |
| `--weapon-skip-delay` | Skip weapon panel delay (faster but lock detection may be inaccurate) |
| `--artifact-skip-delay` | Skip artifact panel delay (faster but lock/astral detection may be inaccurate) |
| `--artifact-substat-ocr <NAME>` | Substat OCR backend (default: ppocrv4) |

### Config File

Timing parameters and character names are configured via `data/good_config.json` (no CLI flags needed):

```json
{
  "traveler_name": "",
  "wanderer_name": "",
  "manekin_name": "",
  "manekina_name": "",
  "char_tab_delay": 400,
  "char_open_delay": 1200,
  "weapon_grid_delay": 60,
  "weapon_scroll_delay": 200,
  "artifact_grid_delay": 60,
  "artifact_scroll_delay": 200
}
```

## Building from Source

```shell
# Requires stable Rust toolchain
rustup default stable

# Make sure Git LFS is installed
git lfs pull

# Build the normal shared scanner
cargo build --locked --release -p good_tools_app --bin GOODScanner

# Build the shared capture edition
cargo build --locked --release -p good_tools_app --features capture --bin GOODCapture

# The existing output paths remain:
# target/release/GOODScanner.exe
# target/release/GOODCapture.exe
```

Fixtures and replay tests prove parser, export, and simulated-interaction behavior only. They do not prove live scanning, capture, or manager behavior against the current Star Rail client.

## Acknowledgments

- [wormtql/yas](https://github.com/wormtql/yas) — Original project providing the core OCR scanning framework
- [1803233552/yas](https://github.com/1803233552/yas) — Fork that this project is based on
- [Andrewthe13th/Inventory_Kamera](https://github.com/Andrewthe13th/Inventory_Kamera) — Reference implementation for GOOD format scanning

## Feedback

- [GitHub Issues](https://github.com/Anyrainel/GOODScanner/issues)
