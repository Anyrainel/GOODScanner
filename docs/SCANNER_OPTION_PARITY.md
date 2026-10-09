# Scanner and manager options

Both games default to keeping only the latest capture/scan export. Existing
explicit saved choices remain intact. OCR export scans default to skipping
unreadable entries and logging their causes. **Stop on OCR errors** is an
opt-in diagnostic setting; it stops at the first failed entry. Game-controller,
focus, navigation, and runtime failures still stop the task.

Detailed logs exposes the existing field-level OCR debug logs. Warnings and
complete errors remain available even when detailed logs is off. OCR pool
counts are automatic; the old v4/v5 user overrides have been removed.

Genshin management defaults to filtering the requested artifact sets and
stopping once targets are matched. Management no longer refreshes the full
inventory; use the separate scan action, including recently acquired artifacts.
Old `update_inventory` config values are retired. Normal inventory scan APIs
and their complete snapshots remain available.

## Star Rail character scan timing

Characters alternate Details → Traces → Eidolons and Eidolons → Traces →
Details. The next portrait is selected on the final tab, so each character
needs only two tab switches and one screenshot per tab. Header, level and
Traces OCR run on a scoped worker while the device switches and captures
subsequent tabs. The header is read once on the first tab to identify the
Traces layout in both directions. Every tab's header must match before the
screenshots can be combined.

New timing defaults are 500 ms for Details/Eidolons and Traces, and 200 ms
before checking a portrait selection. Selection still requires a changed,
stable name region; the wait alone never proves a successful character switch.
Existing saved tab waits are preserved. Reset **Panel switch** and **Open
Traces** in the timing settings to adopt the new defaults. **Character switch**
is a separate setting and defaults to 200 ms when absent in old configs.
Ambient menu animation is diagnostic-only and no longer emits repeated warnings.

## Remaining Star Rail work

- TODO(hsr-manager-set-filter): implement game-side Relic set filtering, then
  prove completeness within the selected sets before matching. The current
  manager still scans the complete Relic inventory. Do not expose a checkbox
  that would claim this optimization already works.
- OCR Achievement scanning.
- Relic equip/unequip operations.

## Request data and recovery records

**Save request data** defaults off in both games. When enabled, bounded request
bodies are written to `log/`; Star Rail uses `hsr_manage_…` / `hsr_scan_…` names.
Genshin can also save complete job exports for debugging. Failures to write
debug logs are reported without interrupting game operations.

Star Rail's recovery journal is independent and always enabled for management.
It records the original operation identity, desired state, and each transition:
pending, mutation started, verified, or needs review. Before a click it saves
the intent; afterward it rereads the game and saves the verified outcome.
After interruption, resubmitting the same operation triggers fresh observations
to reconcile its record. It does not blindly replay clicks: locking and discard
marking toggle state, so replay could undo a change already completed. An
uncertain outcome remains for manual review; unfinished records prevent
unrelated requests from replacing the recovery context. Completed records
are archived.

## Saved settings

Genshin `good_config.json` now uses schema version 1. Legacy explicit
`continue_on_failure` values migrate to the inverse `stop_on_failure`, preserving
their behavior. Missing values use the new default (false). Retired pool and
inventory-refresh keys are removed on save. Star Rail adds its diagnostic
settings with false defaults in the current application config schema.
