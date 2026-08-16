# Capture Data Specification

This document specifies the game data JSON file needed by the packet capture feature.
The file should be generated from [AnimeGameData](https://gitlab.com/Dimbreath/AnimeGameData)
and hosted on ggartifact.com for download.

The format is an exact replica of what the
[anime-game-data](https://github.com/konkers/anime-game-data) Rust crate produces
as its `data_cache.json`. If irminsul has bugs in its generation logic, we replicate
them for compatibility.

## Hosting & Download

- **Filename**: `data_cache.json` (same as irminsul's cache file — users can share this file between yas and irminsul)
- **URL**: `https://ggartifact.com/good/data_cache.json`
- **Downloaded to**: `data/data_cache.json` (next to existing `data/mappings.json`)
- **Metadata file**: `data/data_cache_meta.json` (same format as `mappings_meta.json`: `{"lastFetchTime": <unix_seconds>}`)
- **TTL**: 2 hours
- **Fallback**: If fetch fails, use stale cache; if no cache, error

## Top-Level Structure

```json
{
  "version": 0,
  "git_hash": "<AnimeGameData commit hash>",
  "affix_map": { ... },
  "artifact_map": { ... },
  "character_map": { ... },
  "material_map": { ... },
  "property_map": { ... },
  "set_map": { ... },
  "skill_type_map": { ... },
  "weapon_map": { ... }
}
```

- `version` — Must be `0` (constant `DATABASE_VERSION` in irminsul).
- `git_hash` — The AnimeGameData commit hash this data was generated from. Irminsul uses this to check for updates; your generation script should record the commit hash it fetched from.
- All maps use **string keys** (JSON object keys are always strings), but the keys are stringified `u32` game IDs.
- Field names are **snake_case** (Rust serde defaults).
- `material_map` — Include as empty `{}` for irminsul compatibility (irminsul expects the field, though we don't use it). Or populate it if convenient — it's harmless.
- **Irminsul compatibility**: This is the exact same format as irminsul's `data_cache.json`. Users can copy this file into irminsul's storage directory (`%APPDATA%/Irminsul/`) and it will work, and vice versa.

## Important: AnimeGameData has obfuscated fields

The raw AnimeGameData JSON files contain a mix of readable camelCase field names and
obfuscated field names (e.g. `BHKDALKJOOD`, `JAPGANPLPDP`, `CDNBAHDCNJK`). These
obfuscated fields change between game versions.

**Ignore all obfuscated fields.** The generation script only needs the specific
camelCase fields documented below for each source file. Everything else should be
skipped.

## Maps

### `character_map`

Maps `avatar_id` → English character name.

**Source**: `ExcelBinOutput/AvatarExcelConfigData.json` + `TextMap/TextMapEN.json`

**AGD fields used** (camelCase, as they appear in the JSON):
- `id` — avatar ID (u32), becomes the map key
- `nameTextMapHash` — u32 hash, looked up in TextMapEN to get English name

**Logic**: For each entry, look up `nameTextMapHash` in TextMapEN. If the hash
resolves to a non-empty string, include it. If not, skip the entry (via `filter_map`).

```json
{
  "10000002": "Kamisato Ayaka",
  "10000061": "Kirara"
}
```

**Note**: Some entries are test/internal avatars. The irminsul crate does NOT filter
these out at data generation time — it includes all entries with valid names. Filtering
by `avatarType == 1` happens later at packet processing time. So include everything.

### `skill_type_map`

Maps `skill_id` → skill type (Auto / Skill / Burst).

**Source**: `ExcelBinOutput/AvatarSkillDepotExcelConfigData.json`

**AGD fields used**:
- `energySkill` — u32 skill ID for **Burst**
- `skills` — array of u32 skill IDs; index `[0]` is **Auto**, index `[1]` is **Skill**

**Logic** (replicating irminsul exactly):
```
for each entry in data:
    insert(entry.energySkill, "Burst")
    insert(entry.skills[0], "Auto")
    insert(entry.skills[1], "Skill")
```

**Bug replication**: The irminsul code does NOT skip `0` values. If `energySkill` is
`0` or `skills[0]`/`skills[1]` is `0`, it still inserts them. This means `"0"` may
appear as a key in the map. Replicate this behavior. The `skills` array typically has
4 entries (e.g. `[10024, 10018, 10013, 0]`) — only index 0 and 1 are used.

```json
{
  "0": "Skill",
  "10024": "Auto",
  "10018": "Skill",
  "10019": "Burst"
}
```

**Enum values**: `"Auto"`, `"Skill"`, `"Burst"` (PascalCase, exact strings).

### `weapon_map`

Maps `weapon_id` → weapon info.

**Source**: `ExcelBinOutput/WeaponExcelConfigData.json` + `TextMap/TextMapEN.json`

**AGD fields used**:
- `id` — weapon ID (u32), becomes the map key
- `nameTextMapHash` — u32, looked up in TextMapEN
- `rankLevel` — u32 rarity (1–5)

**Logic**: For each entry, look up `nameTextMapHash` in TextMapEN. If found, include.
If not, skip (via `filter_map`).

```json
{
  "11505": {
    "name": "Primordial Jade Cutter",
    "rarity": 5
  },
  "12101": {
    "name": "Waster Greatsword",
    "rarity": 1
  }
}
```

### `artifact_map`

Maps `artifact_item_id` → artifact info (set name, slot, rarity).

**Source**: `ExcelBinOutput/ReliquaryExcelConfigData.json` (depends on `set_map` being built first)

**AGD fields used**:
- `id` — artifact item ID (u32), becomes the map key
- `setId` — u32, looked up in `set_map` to get English set name
- `equipType` — string, mapped to slot enum (see below)
- `rankLevel` — u32 rarity (1–5)

**Logic**: For each entry, look up `setId` in the already-built `set_map`. If found,
and `equipType` maps to a valid slot, include. If either fails, skip (via `filter_map`).

**Slot mapping** (from `equipType` string → JSON value):

| `equipType` | `slot` |
|---|---|
| `"EQUIP_BRACER"` | `"Flower"` |
| `"EQUIP_NECKLACE"` | `"Plume"` |
| `"EQUIP_SHOES"` | `"Sands"` |
| `"EQUIP_RING"` | `"Goblet"` |
| `"EQUIP_DRESS"` | `"Circlet"` |
| anything else | skip entry |

```json
{
  "31534": {
    "set": "Marechaussee Hunter",
    "slot": "Circlet",
    "rarity": 5
  }
}
```

### `property_map`

Maps `main_prop_id` → property type enum string.

**Source**: `ExcelBinOutput/ReliquaryMainPropExcelConfigData.json`

**AGD fields used**:
- `id` — main prop ID (u32), becomes the map key
- `propType` — string (e.g. `"FIGHT_PROP_CRITICAL"`), mapped to enum

**Logic**: For each entry, parse `propType` via the mapping table below. If the
`propType` is not in the table, silently skip the entry (via `filter_map` — not an error).

**Property type mapping** (exhaustive — any `FIGHT_PROP_*` not listed here is skipped):

| `propType` (AGD) | JSON value |
|---|---|
| `FIGHT_PROP_HP` | `"Hp"` |
| `FIGHT_PROP_HP_PERCENT` | `"HpPercent"` |
| `FIGHT_PROP_ATTACK` | `"Attack"` |
| `FIGHT_PROP_ATTACK_PERCENT` | `"AttackPercent"` |
| `FIGHT_PROP_DEFENSE` | `"Defense"` |
| `FIGHT_PROP_DEFENSE_PERCENT` | `"DefensePercent"` |
| `FIGHT_PROP_ELEMENT_MASTERY` | `"ElementalMastery"` |
| `FIGHT_PROP_CHARGE_EFFICIENCY` | `"EnergyRecharge"` |
| `FIGHT_PROP_HEAL_ADD` | `"Healing"` |
| `FIGHT_PROP_CRITICAL` | `"CritRate"` |
| `FIGHT_PROP_CRITICAL_HURT` | `"CritDamage"` |
| `FIGHT_PROP_PHYSICAL_ADD_HURT` | `"PhysicalDamage"` |
| `FIGHT_PROP_WIND_ADD_HURT` | `"AnemoDamage"` |
| `FIGHT_PROP_ROCK_ADD_HURT` | `"GeoDamage"` |
| `FIGHT_PROP_ELEC_ADD_HURT` | `"ElectroDamage"` |
| `FIGHT_PROP_WATER_ADD_HURT` | `"HydroDamage"` |
| `FIGHT_PROP_FIRE_ADD_HURT` | `"PyroDamage"` |
| `FIGHT_PROP_ICE_ADD_HURT` | `"CryoDamage"` |
| `FIGHT_PROP_GRASS_ADD_HURT` | `"DendroDamage"` |

```json
{
  "10001": "Hp",
  "50960": "PyroDamage"
}
```

### `affix_map`

Maps `affix_id` → substat property + value.

**Source**: `ExcelBinOutput/ReliquaryAffixExcelConfigData.json`

**AGD fields used**:
- `id` — affix ID (u32), becomes the map key
- `propType` — string, mapped via the same property table as `property_map`
- `propValue` — f64, the raw stat value

**Logic**: For each entry, parse `propType` via the property mapping table. If
unrecognized, silently skip (via `filter_map`). Then apply value transformation:

**Value transformation**:
- If the property `is_percentage` → multiply `propValue` by 100
- Otherwise → store `propValue` as-is

**Which properties are percentage** (all others are flat):
`HpPercent`, `AttackPercent`, `DefensePercent`, `EnergyRecharge`, `Healing`,
`CritRate`, `CritDamage`, `PhysicalDamage`, `AnemoDamage`, `GeoDamage`,
`ElectroDamage`, `HydroDamage`, `PyroDamage`, `CryoDamage`, `DendroDamage`

**Which properties are flat** (NOT percentage):
`Hp`, `Attack`, `Defense`, `ElementalMastery`

Example: if `propValue` is `0.0583` and the property is `CritRate` (percentage),
store `5.83`. If `propValue` is `239.0` and property is `Hp` (flat), store `239.0`.

```json
{
  "501022": {
    "property": "Hp",
    "value": 239.0
  },
  "501242": {
    "property": "CritRate",
    "value": 3.89
  }
}
```

### `set_map`

Maps `set_id` → English set name.

**Source**: `ExcelBinOutput/DisplayItemExcelConfigData.json` + `TextMap/TextMapEN.json`

**AGD fields used**:
- `displayType` — string, must equal `"RELIQUARY_ITEM"` (filter; skip `"RELIQUARY_ITEM_SMELT"` and others)
- `nameTextMapHash` — u32, looked up in TextMapEN
- `param` — u32, this is the **set ID** (becomes the map key). **NOT the `id` field** — the `id` field is the display item's own ID and is not used.

**Logic**: For each entry where `displayType == "RELIQUARY_ITEM"`, look up
`nameTextMapHash` in TextMapEN. If found, insert `param → name`. If not found, skip.

**Note**: Multiple display items can share the same `param` (set ID) — e.g. different
rarity variants of the same set. Since they all resolve to the same set name, later
inserts just overwrite with the same value. This is fine.

```json
{
  "15031": "Marechaussee Hunter",
  "15001": "Gladiator's Finale"
}
```

### `material_map` (optional)

Maps `material_id` → English material name. We don't use this for capture, but
include it for irminsul compatibility.

**Source**: `ExcelBinOutput/MaterialExcelConfigData.json` + `TextMap/TextMapEN.json`

**AGD fields used**:
- `id` — material ID (u32), becomes the map key
- `nameTextMapHash` — u32, looked up in TextMapEN

Can be `{}` if not needed. Irminsul uses `#[serde(default)]` so a missing field
deserializes as an empty HashMap.

## Generation Script Notes

### Input files from AnimeGameData

Download these from `https://gitlab.com/Dimbreath/AnimeGameData/-/raw/<commit_hash>/`:

1. `ExcelBinOutput/AvatarExcelConfigData.json`
2. `ExcelBinOutput/AvatarSkillDepotExcelConfigData.json`
3. `ExcelBinOutput/WeaponExcelConfigData.json`
4. `ExcelBinOutput/ReliquaryExcelConfigData.json`
5. `ExcelBinOutput/ReliquaryMainPropExcelConfigData.json`
6. `ExcelBinOutput/ReliquaryAffixExcelConfigData.json`
7. `ExcelBinOutput/DisplayItemExcelConfigData.json`
8. `TextMap/TextMapEN.json`

To get the latest commit hash, query the GitLab API:
```
GET https://gitlab.com/api/v4/projects/53216109/repository/commits
```
Take `[0].id` (the first entry's full SHA hash). Use this as both the `git_ref` for
downloading files and as the `git_hash` in the output JSON.

### Build order (dependency matters)

1. Load `TextMapEN.json` into a `HashMap<u32_string_key, string>` (the TextMap JSON has string keys like `"1456643042"`, not numeric keys)
2. Build `set_map` (needs TextMap)
3. Build `artifact_map` (needs `set_map`)
4. Build `property_map` (standalone)
5. Build `affix_map` (standalone, uses same property mapping as property_map)
6. Build `weapon_map` (needs TextMap)
7. Build `character_map` (needs TextMap)
8. Build `skill_type_map` (standalone)
9. Optionally build `material_map` (needs TextMap), or use `{}`

### Error handling

The irminsul crate uses `filter_map` everywhere — entries that fail to parse (unknown
`propType`, missing TextMap hash, unknown `equipType`, missing `setId` in set_map)
are **silently dropped**, not errors. The generation script should do the same.

### Output format

Write with pretty-printing (indentation). The irminsul crate uses
`serde_json::to_writer_pretty`.

### Field ordering in output

Serde's default HashMap serialization produces arbitrary key order. The generation
script does not need to match any particular key ordering — consumers parse by key
name, not position.
