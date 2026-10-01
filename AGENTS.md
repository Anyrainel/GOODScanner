# Yas — Genshin Impact Scanner

## Build Rules

- **NEVER kill GOODScanner.exe or any user process to unblock a build.** If `cargo build` fails with "access denied" because the exe is locked, tell the user and wait. If they confirm the process can be stopped, wait for it to exit on its own or let the user close it.

## Prioritization

- **When instructions are clear, implement them directly.** Do not block one clear item because another item needs discussion. Make all clear changes first, then discuss the unclear ones.

## No Piecemeal Refactoring

- **When receiving feedback that changes a design decision, propagate its full implications** — don't patch only the specific spot the user pointed at. If the feedback implies a different ownership model, naming scheme, or data flow, update all affected sites, not just the one mentioned.

## DRY is Top Priority

- **Never duplicate logic between test binaries and production code.** Core features (UI navigation, OCR scanning, filter operations, grid scanning) must be implemented as well-defined methods in the proper modules (e.g., `manager/ui_actions.rs`). Test binaries should only contain test-specific looping/reporting logic and call shared functions.
- When refactoring for DRY, **ensure logic equivalence** — the extracted function must behave identically to the original inline code.
- Prefer reusing existing functions over writing new code that does the same thing.

## Overview

Yas (Yet Another Scanner) is a Rust application that scans Genshin Impact in-game data (characters, weapons, artifacts) using OCR and exports it in **GOOD v3** (Genshin Open Object Description) format for use with optimizer tools.

## Architecture

### Workspace Crates

- **`yas`** (`yas_core`) — Platform-agnostic core library: screen capture, OCR (PaddlePaddle ONNX models), system control (mouse/keyboard), game window detection, positioning/scaling utilities.
- **`genshin`** (`genshin_scanner`) — Genshin-specific scanner logic: GOOD v3 scanners for characters, weapons, and artifacts. Handles in-game navigation, panel OCR, and name matching via remote mappings.
- **`application`** (`good_tools_app`) — Binary crate. Two user-facing targets: `GOODScanner.exe` (OCR scanner + manager) and `GOODCapture.exe` (capture + OCR scanner + manager, behind the `capture` feature flag).

### Key Modules (genshin)

```
src/
├── cli.rs                     # CLI entry point, orchestrates all scanning + run_server_core()
├── server.rs                  # HTTP server (tiny_http): /manage, /equip, /status, /result, /artifacts
├── updater.rs                 # Auto-update: GitHub release check + self-replace
├── manager/                   # Artifact lock/equip manager (server-driven)
│   ├── orchestrator.rs        # ArtifactManager: top-level execute() and execute_equip()
│   ├── lock_manager.rs        # LockManager: single-pass backpack scan + per-page lock toggle
│   ├── equip_manager.rs       # EquipManager: equip/unequip via character screen navigation
│   ├── matching.rs            # Hard-match artifacts: all fields + 0.1 substat tolerance
│   ├── models.rs              # Request/response types: LockManageRequest, EquipRequest, ManageResult
│   ├── ui_actions.rs          # Game UI helpers: click lock button, open character screen, etc.
│   └── mod.rs
├── scanner/
│   ├── common/                # Shared scanner infrastructure
│   │   ├── game_controller.rs # Mouse/keyboard/capture control
│   │   ├── backpack_scanner.rs# Grid-based inventory navigation
│   │   ├── mappings.rs        # Remote name→GOOD key mappings (from ggartifact.com)
│   │   ├── coord_scaler.rs    # Resolution-independent coordinate scaling (base: 1920x1080)
│   │   ├── models.rs          # GOOD v3 data models (GoodExport, GoodCharacter, etc.)
│   │   ├── stat_parser.rs     # Artifact stat string parsing
│   │   ├── diff.rs            # Groundtruth comparison tooling
│   │   ├── constants.rs       # Grid positions, UI coordinates
│   │   ├── ocr_factory.rs     # OCR backend selection (ppocrv3/v4/v5)
│   │   ├── ocr_pool.rs        # Channel-based pool of N OCR model instances
│   │   ├── pixel_utils.rs     # Color/pixel analysis helpers
│   │   ├── fuzzy_match.rs     # Fuzzy string matching for OCR results
│   │   └── navigation.rs      # Tab/page navigation helpers
│   ├── character/              # Character panel OCR
│   ├── weapon/                 # Weapon panel OCR
│   └── artifact/               # Artifact panel OCR (scanner.rs has identify_artifact, scan_level_only)
```

### Key Modules (application)

```
src/
├── main.rs                    # Entry point: CLI mode or GUI mode (GOODScanner.exe)
├── bin/
│   └── yas.rs                 # Entry point for GOODScanner.exe and GOODCapture.exe
└── gui/
    ├── mod.rs                 # eframe App impl, tab routing
    ├── state.rs               # AppState: all GUI state fields
    ├── worker.rs              # spawn_scan(), spawn_server() — background thread launchers
    ├── manager_tab.rs         # Manager tab UI: server start/stop, update_inventory checkbox
    ├── scan_tab.rs            # Scan tab UI: scan target checkboxes, options
    ├── capture_tab.rs         # Capture tab UI: start/stop packet capture, export
    ├── settings_tab.rs        # Settings tab: config editing
    └── log_tab.rs             # Log viewer tab
```

### Key Modules (genshin/capture — behind `capture` feature flag)

```
src/capture/
├── mod.rs
├── packet_capture.rs          # UDP capture via pktmon on ports 22101–22102
├── monitor.rs                 # CaptureMonitor: orchestrates capture, decryption, data accumulation
├── data_cache.rs              # Downloads/caches data_cache.json from ggartifact.com
├── data_types.rs              # DataCache types (irminsul/anime-game-data format)
├── player_data.rs             # PlayerData: converts captured packets → GOOD v3 export
└── testdata/                  # Binary test fixtures (items.bin, avatars.bin, noise.bin)
```

### How Scanning Works

1. User opens Genshin Impact and navigates to the appropriate screen
2. `GenshinGameController` captures the game window and provides scaled coordinates
3. `BackpackScanner` navigates the grid inventory (weapons/artifacts)
4. Individual scanners OCR each panel's fields (name, level, stats, etc.)
5. OCR results are fuzzy-matched against `MappingManager` data (fetched from ggartifact.com)
6. Results are exported as GOOD v3 JSON

### Config File (`good_config.json`)

On first run, a bilingual prompt asks for custom in-game names for Traveler/Wanderer/Manekin/Manekina (renameable characters). The JSON file is created next to the exe with these names plus all timing/delay defaults:

```json
{
  "traveler_name": "",
  "wanderer_name": "",
  "manekin_name": "",
  "manekina_name": "",
  "char_tab_delay": 500,
  "char_next_delay": 300,
  "char_open_delay": 1500,
  "char_close_delay": 500,
  "inv_scroll_delay": 200,
  "inv_tab_delay": 400,
  "inv_open_delay": 1500,
  "capture_delay": 40
}
```

Existing config files without delay fields are loaded correctly via `#[serde(default)]` and re-saved with new defaults. Old per-scanner field names (`weapon_grid_delay`, `artifact_grid_delay`, etc.) are accepted via serde aliases for backwards compatibility.

## Build & Run

```bash
# Stable Rust toolchain
rustup default stable

# Build
cargo build --release

# The binary is at target/release/GOODScanner.exe
# Run with default (scan artifacts):
GOODScanner.exe

# Scan everything:
GOODScanner.exe --all

# Scan specific categories:
GOODScanner.exe --characters --weapons --artifacts
```

Requires administrator privileges on Windows (for input simulation).

## CLI Flags

All help text is bilingual (Chinese + English). Flags are grouped into four sections:

### Scan Targets
- `--characters` / `--weapons` / `--artifacts` / `--all`

### Global Options
- `-v, --verbose` — detailed scan info
- `--continue-on-failure` — keep scanning when individual items fail
- `--log-progress` — log each scanned item
- `--output-dir <DIR>` — output directory (default: `.`)
- `--ocr-backend <NAME>` — override every category's secondary (v5-slot) backend: character name/level check, weapon equip fallback, artifact level (also achievements)
- `--dump-images` — save OCR region screenshots to `debug_images/`

### Scanner Config
- `--weapon-min-rarity <N>` — min weapon rarity (default: 3)
- `--artifact-min-rarity <N>` — min artifact rarity (default: 4)
- `--char-max-count <N>` / `--weapon-max-count <N>` / `--artifact-max-count <N>` — max items (0 = unlimited)
- `--weapon-skip-delay` / `--artifact-skip-delay` — skip panel delay (faster but less reliable lock/astral detection)
- `--char-ocr <NAME>` — character general backend (default: ppocrv4)
- `--weapon-ocr <NAME>` — weapon general backend (default: ppocrv6tiny)
- `--artifact-substat-ocr <NAME>` — artifact substat/general backend (default: ppocrv6tiny)

### Debug
- `--debug-compare <PATH>` — groundtruth JSON comparison
- `--debug-actual <PATH>` — offline diff (no scanning)
- `--debug-start-at <N>` — skip to item index
- `--debug-char-index <N>` — jump to character index
- `--debug-timing` — per-field OCR timing
- `--debug-rescan-pos <R,C>` — re-scan a grid position
- `--debug-rescan-type <TYPE>` — scanner type for re-scan (default: weapon)
- `--debug-rescan-count <N>` — re-scan iterations (0 = infinite until RMB)

### Architecture Notes
- Character names are set via first-run prompt → `good_config.json` only (no CLI flags)
- Timing/delay settings live in `good_config.json` only (no CLI flags)
- Per-scanner verbose/dump/continue/log flags consolidated into global flags
- Per-scanner configs are plain structs (no clap derives); the orchestrator (`cli.rs`) populates them from global CLI flags + JSON config

## Dependencies & Platform

- **OCR**: ONNX Runtime (`ort` crate) with PaddleOCR models (embedded via `include_bytes!`)
- **Screen capture**: `screenshots` crate (with Win32 BitBlt primary path on Windows)
- **Input simulation**: `enigo` crate
- **Remote mappings**: `reqwest` (blocking HTTP to ggartifact.com)
- **Windows only**: Requires admin, uses Win32 APIs for window detection; packet capture (`capture` feature) is Windows-only (pktmon)
- **Linux** (experimental): native build supports OCR scanning + manager. See "Linux Platform Layer" below.

## Linux Platform Layer

The game runs on Linux only via Wine/Proton and is therefore always an **X11/XWayland window** — one X11 code path covers both X11 sessions and Wayland sessions. Do NOT run the game with the wine-wayland driver (`PROTON_ENABLE_WAYLAND=1` / `Graphics=wayland`): its windows are invisible to X11 and window detection reports this.

- **`yas/src/utils/linux_x11.rs`** — the counterpart of `utils/windows.rs`: x11rb with a thread-local connection; window discovery by title (`_NET_CLIENT_LIST` first, tree-walk fallback), absolute client geometry (`get_client_rect` ≡ GetClientRect+ClientToScreen), EWMH activation, pointer query (RMB cancel), GetImage ZPixmap capture (out-of-window regions padded black, like a clipped BitBlt), XTEST injection + effectiveness self-check
- **Capture**: `X11Capturer` captures the game window's own pixmap (monitor-layout independent); it is the Linux `GenericCapturer`. `capturer_screenshots` / `capturer_libwayshot` features remain as explicit alternatives
- **Input** (`system_control/linux/`): dual backend chosen on first event — X11 session → XTEST; Wayland session → **ydotool socket** (`ydotool.rs` implements the v1.0.x protocol: raw 24-byte `input_event` writes, no handshake). KWin/Mutter advertise XTEST but ignore fake input — never assume XTEST works on Wayland; use `xtest_effective()` to check
- **ydotool absolute positioning** (`PointerState` in `linux_control.rs`): ydotoold injects only *relative* motion, so absolute positioning means slam the pointer into the top-left corner to establish an origin, then move by a relative delta. Three non-obvious requirements: (1) the slam and the delta must land in **separate compositor frames** — sent back-to-back they are coalesced, the delta is swallowed and the pointer stays in the corner, which is what made every click land in empty space; (2) relative motion is multiplied by the session's **pointer-speed factor** (2.5× on a default KDE setup), so deltas are divided by a factor learned from a live X11 pointer readback; (3) the slam is used **only to (re)establish the origin** — every later move is a small relative hop from the last verified position, verified each time. Slamming per move would be slow and would jerk the pointer to the corner in the middle of a scroll burst, where the game needs a stable hover over the grid. A readback that does not move for a non-trivial delta is stale (the game can hold the pointer): the origin is dropped so the next move re-slams, and it is reported once
- **Wheel sign** (`SystemControl::mouse_scroll`): **positive = scroll down** (towards the end of a list) on every platform. enigo's Windows `mouse_scroll_y` negates internally (`length * -120`, enigo 0.1.3) and `macos_control` negates explicitly, so the Linux backend must negate too (X11 button 4 is "up", `REL_WHEEL`'s native sign is "up"). Dropping that negation is silent: each page turn scrolls the inventory *up* instead of down, so a scan past the first page re-reads page 1 forever and reports duplicated artifacts. Measured on the reference KDE setup: one detent = 18 px, so the calibrated 49 ticks ≈ 5 rows.
- **`GameInfo.window_id`** (u32, `#[cfg(target_os = "linux")]`) is the X11 window — the counterpart of `hwnd`, used by focus/geometry-refresh/alive checks
- **ORT runtime** detection order (both platforms): `ORT_DYLIB_PATH` env → exe-dir copy → system install (Linux only) → prompt + auto-download (Windows zip / Linux tgz, gh-proxy mirrors first)
- **Windows-only by design**: self-update (PE assets), packet capture, HSR live capture — each returns a clear bilingual error on Linux instead of compiling out
- **CJK fonts**: Linux candidates under `/usr/share/fonts` (Noto CJK / WenQuanYi / Droid) for GUI and dump annotations
- **Diagnostics**: `cargo run -p yas_core --example x11_probe` lists windows/geometry/XTEST status; `capture <id> <out.png>` grabs a test capture

## Conventions

- All UI coordinates use 1920x1080 as base resolution, scaled at runtime via `CoordScaler`
- Chinese (zh_CN) game client only — OCR models trained on Chinese game text
- GOOD v3 format spec: keys use PascalCase (e.g., `"SkywardHarp"`, `"Furina"`)
- The `data/` directory (gitignored) caches remote mapping files

## Manager & HTTP Server

### Architecture

Two-thread model: HTTP thread (tiny_http) handles requests, execution thread owns the game controller and processes jobs sequentially. Communication via `mpsc` channel + `Arc<Mutex<JobState>>`.

### Data flow

1. Client sends `POST /manage`, `POST /equip`, or `POST /scan` → server validates, returns 202 with `jobId`
2. Client polls `GET /status` for progress
3. Execution thread processes the job (manage/equip/scan)
4. Client fetches `GET /result?jobId=xxx` (idempotent) for final results
5. Client fetches scanned data via `GET /characters?jobId=xxx`, `GET /weapons?jobId=xxx`, `GET /artifacts[?jobId=xxx]`

### API Reference

Endpoints (summary only — full contract, request/response shapes, status codes, and worked examples live in [`docs/MANAGER_API.md`](docs/MANAGER_API.md); update that file when the wire contract changes):

- `POST /manage` — lock/unlock artifacts. Returns 202 with `jobId`.
- `POST /equip` — equip/unequip artifacts. Returns 202 with `jobId`.
- `POST /scan` — OCR scan for characters/weapons/artifacts. Returns 202; one `jobId` covers all requested categories.
- `GET /status` — poll job state. Manage/equip expose `progress` (linear, per-item). Scan exposes `scanProgress` (per-category, one slot each, `pending`/`running`/`complete`/`aborted`).
- `GET /result?jobId=xxx` — final per-instruction results + summary.
- `GET /characters?jobId=xxx`, `GET /weapons?jobId=xxx` — scan data. 503 if that jobId attempted the category but didn't finish.
- `GET /artifacts[?jobId=xxx]` — scan data or manage snapshot. `jobId` optional for back-compat.
- `GET /health` — `{status, enabled, busy, gameAlive}`.

#### `GET /health` — Health check
Returns `{"status":"ok","enabled":bool,"busy":bool,"gameAlive":bool}`.

### Data Caching

Each data type (characters, weapons, artifacts) has an independent `ScanDataCache<T>` storing the latest `(jobId, data)` plus an `incomplete_job_id` slot. All-or-nothing: a scan category populates the cache only if it completes in full during that run; if it aborts/errors/is never reached, the jobId is recorded as incomplete and the cache isn't written — queries for that jobId return 503. Categories the client didn't request leave the cache untouched. Manage/equip jobs that modify in-game state invalidate the artifact cache before execution.

`scan_worker` stops a scan after 10 consecutive item errors. That stop is a failure, not an early finish: the scanners report it via `WorkerHandle::join_with_status()` and `bail!`, so the phase is `Failed` and the (possibly empty) partial list never reaches the cache. Without this, a run where every click missed looked exactly like "this account owns no artifacts".

### Key config flow

GUI `state.update_inventory` (bool, default true) + `state.filter_involved_sets` (bool, default false) → `stop_on_all_matched` / set-filter mode → passed through `cli.rs::run_server_core()` → `ArtifactManager::new()` → `LockManager::execute()`.

### Matching (matching.rs)

All fields are hard-match (reject on mismatch): set, slot, rarity, level, main stat, elixir_crafted, substats, unactivated substats. Substat values allow 0.1 tolerance for OCR rounding. `location`, `lock`, `astral_mark` are NOT matched (they change independently of artifact identity).

### Lock toggle flow (lock_manager.rs)

Per-page: scan all items via pipelined OCR → match against targets → re-click matched positions → toggle lock → verify pixel. Page-skip optimization: in fast mode, OCR the last item's level first; if > max target level, skip the page entirely (inventory sorted by level descending).

### Snapshot (orchestrator.rs)

After a complete manage scan, builds an artifact snapshot reflecting post-toggle state: updates `lock` and clears `astral_mark` on unlock (game forces this). Served via `GET /artifacts?jobId=xxx`.

## Fuzzy Matching (`fuzzy_match.rs`)

5-tier fallback for matching OCR text against name→key maps:

1. **OCR confusion substitution** — char-by-char replacement of known misreads (e.g., 稚→薙, 拉→菈). Tries each pair individually, then applies ALL applicable substitutions simultaneously (needed when OCR garbles multiple chars, e.g. 菈乌玛→拉鸟玛 requires both 拉→菈 and 鸟→乌).
2. **Exact match** on cleaned/normalized text
3. **Substring match** (both directions: OCR added noise, or OCR truncated)
4. **Levenshtein distance** (30% threshold, char-level for CJK)
5. **LCS uniqueness fallback** (≥2 shared CJK chars, unique to one candidate)

### Adding OCR Confusion Pairs

In `OCR_CONFUSIONS` array. Rules:
- Only add `(wrong, correct)` where `wrong` does NOT appear as a standalone char in any legitimate name — otherwise exact match on that name would never be reached (the substitution would mangle it). Even if the substitution doesn't match, it wastes a lookup. Chars with collisions (菈↔莱, 鹮↔鹤/环) rely on Tier 4/5 instead.
- All current pairs are single-char to single-char. The combined pass assumes this.
- The combined pass applies all substitutions in one char-by-char sweep, avoiding cascading issues with bidirectional pairs (e.g., 茲↔兹).

## Artifact Scanner Details

### Dual-Engine OCR Pipeline

`SharedOcrPools` holds one pool pair per category (character / weapon / artifact); each pair has a general (v4-slot) and a secondary (v5-slot) pool, and pairs naming the same backend share instances. Defaults live in `ocr_pool.rs` (`DEFAULT_*_OCR`), chosen from a live eval against groundtruth:
- **Characters**: general ppocrv4 (`--char-ocr`), secondary ppocrv5. v6 tiny read names 70.7% / levels 67.5% vs v4's 94.7% / 99.2%, and its dictionary lacks 魈.
- **Weapons**: ppocrv6tiny in both slots (`--weapon-ocr`). Name 97.5% vs v4's 89%.
- **Artifacts**: level engine ppocrv6tiny, general engine ppocrv6tiny (`--artifact-substat-ocr`) for name, main stat, set, equip, substats. Substats 100% vs v4's 96.4%, set name 99.2% vs 93.7%; level tied at 100% (v4 alone is only 39.4% on level).
- `--ocr-backend` overrides every category's secondary slot. The equip manager reads character names, so it uses the character pair.

Level uses dual-engine (tries both, takes max valid). Substats use only the general engine. Results are collected as `OcrCandidate` lists per line, then validated by the roll solver.

### Roll Solver (`roll_solver.rs`)

Validates substat combinations against game mechanics:
- Uses pre-computed **rollTable** lookup (from `rollTable.json` via `roll_table.rs`) — NOT brute-force f64 enumeration
- Each entry is `(display_value×10: i32, roll_count_bitmask: u8)`, binary searched
- Validates total roll count = init_count + level/4
- **Init preference**: Level 0 → prefer higher init first (lines = init count); Level > 0 → prefer lower init (better accuracy)
- Outputs `totalRolls`, `initialValue` per substat, and `inactive` flag
- The solver treats inactive (待激活) substats identically to active ones — their values are real roll values

### Elixir Crafted Detection

Elixir artifacts display a purple banner ("祝圣之霜定义") that shifts all content down by 40px (`ELIXIR_SHIFT`).
- Detection: 3 pixels at (1510, 1520, 1530), y=423 — checks for purple (blue > 230 && blue > green + 40)
- **Do NOT move to x=1683** — that hits the lock icon and causes massive false positives
- When detected, all subsequent OCR regions are Y-shifted by 40px

### Substat Crop Regions

- Lines 0–2: width 255px (calibrated to avoid OCR noise from wider crops)
- Line 3: width 355px (wider to capture "(待激活)" text on unactivated substats)
- All start at x=1356

### Unactivated Substats (待激活)

- Appear on level-0 artifacts as the 4th substat line with muted font and "(待激活)" appended
- The stat key and value are real (not zero) — it's the value that WILL be added on first level-up
- `stat_parser.rs` detects "(待激活)" text and sets `ParsedStat.inactive = true`, keeping the real value
- `OcrCandidate.inactive` propagates through the solver to `SolvedSubstat.inactive`
- Scanner splits solver results into `substats` (active) and `unactivated_substats` (inactive) in the output

### Pixel-Based Detection (highly reliable)

- **Rarity**: Star pixel color at fixed Y positions
- **Lock**: Pixel color at `ARTIFACT_LOCK_POS1` (1683, 428)
- **Elixir**: Purple banner check at (1510–1530, 423)
- **Astral mark**: Pixel at `ARTIFACT_ASTRAL_POS1`

### Page Turns (vertical scrolling)

The grid scrolls with `mouse_scroll(1)` ticks and `SCROLL_TICKS_PER_PAGE` is calibrated to 5 rows (one detent = 18 px on the reference setup, so 49 ticks ≈ 5 rows; the pacing is not critical, 20–150 ms spacing measures identically). Two traps here, both of which silently corrupt data rather than failing:

- Getting the **wheel sign** backwards (see the platform notes above) scrolls *up*: at the top of the list that is a no-op, so the scan re-reads page 1 for the rest of the run and emits duplicated artifacts.
- After a scroll the detail panel still shows the *previous* page's last item, and `scan_grid` used to `reset_panel_fingerprint()` — i.e. accept any content as the first item of the new page. A dropped first click therefore recorded that artifact twice and lost the real one. It now re-baselines the panel to the real post-scroll content (`ensure_panel_stable`) and, whenever a click leaves the panel unchanged, re-clicks the cell once (`PANEL_CLICK_ATTEMPTS`) before accepting the capture.

Verify a change here with a multi-page scan (`--artifact-max-count 80`): every page-2 item must be one that was *not* on page 1, and the export must contain no duplicate fingerprints.

### Backpack Tab Verification

`backpack_scanner::select_tab_and_read_count()` OCRs the backpack header — the game draws `"<分类名> <current>/<capacity>"` (e.g. `武器208/2000`) — and requires it to name the requested category. A header naming a *different* known category means the tab click was swallowed (mouse injection broken, or the game window not raised): the click is retried up to 3 times and the scan then fails with a bilingual error instead of reading the wrong inventory. Garbled or unrecognised headers keep the previous lenient behaviour, so an OCR miss on the expected label can never fail an otherwise healthy scan. Only full tab labels are listed in `BACKPACK_TAB_LABELS` — the partial forms (道具/物品) are shared between tabs.

### Parallelization

- `OcrPool`: Channel-based pool of N OCR model instances
- `scan_worker`: Generic parallel worker for backpack grid items
- **ALWAYS create separate pools** for main and substat OCR (sharing causes deadlock: N tasks each hold 1 instance, all waiting for a 2nd)

## GOODCapture (Packet Capture Scanner)

GOODCapture is the merged binary (`GOODCapture.exe`) with Capture, Scanner, and Manager tabs. It exports GOOD v3 data either by sniffing game network packets or by OCR. The standalone GOODScanner.exe remains available for users who do not need packet capture.

### Build

```bash
cargo build --release --features capture --bin GOODCapture
```

### How It Works

1. Uses `pktmon` (Windows packet monitor) to capture UDP traffic on ports 22101–22102
2. `GameSniffer` (from `auto-artifactarium` crate) decrypts packets using dispatch keys from `keys/gi.json`
3. `CaptureMonitor` uses **heuristic field-number-agnostic matching** — parses outer protobuf as generic `Unk`, tries every repeated length-delimited field as `Item` or `AvatarInfo`, picks the best match. This survives both command ID rotation AND outer field number changes across game versions.
4. Auto-stops when both character and item packets are received
5. `PlayerData` converts captured data → GOOD v3 JSON

### Dispatch Keys (`keys/gi.json`)

- `HashMap<u16, String>` mapping game version → base64-encoded key
- External key file (`keys/gi.json` next to exe) overrides embedded keys, allowing updates without recompiling
- Keys are per game version, NOT per server channel — same keys work for official (官服) and Bilibili (B服) servers

### Dependencies (capture-only)

- `auto-artifactarium` — packet decryption + protobuf types (from konkers/auto-artifactarium)
- `pktmon` — Windows packet monitor driver interface
- `protobuf` — protobuf parsing
- `tokio` — async runtime for capture loop

## Testing & Validation

### Groundtruth

- `genshin_export.json`: Exported via third-party tool, contains complete artifact/character/weapon data
- Note: GT uses typo `elixerCrafted` (not `elixirCrafted`) — diff report handles both

### Diff Report (`diff_report.py`)

- Compares scan output against groundtruth with Hungarian algorithm matching
- Groups by `(setKey, slotKey, rarity, lock)` — rarity and lock are hard matching requirements (pixel-based, very reliable)
- Three-tier categorization: non-stat diffs, stat-key diffs, stat-value-only diffs
- Always run scans with `--dump-images` so dump images match the scan output
- Use `python diff_report.py <scan.json> <gt.json>` to generate `diff_report.md`

### Other Scripts

- `test_solver.py`: Validates roll solver against groundtruth (expects ~99.7% totalRolls accuracy)
- `gen_roll_table.py`: Generates `roll_table.rs` from `rollTable.json`

### Key Calibration Values

| Parameter | Value | Notes |
|-----------|-------|-------|
| Substat width (lines 0–2) | 255px | Wider causes OCR failures |
| Substat width (line 3) | 355px | Captures "(待激活)" text |
| delay_after_panel | 100ms | Lock/astral mark animation |
| Talent overview width | 90px | Supports 2-digit levels |
| ELIXIR_SHIFT | 40px | Purple banner height |
| Elixir pixel positions | (1510–1530, 423) | Do NOT use x=1683 |
