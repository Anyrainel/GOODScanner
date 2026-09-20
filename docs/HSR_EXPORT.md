# Star Rail export format / 《星穹铁道》导出格式

GOODCapture writes **one** JSON file: `star_rail_capture_<timestamp>.json`.
The screenshot scanner writes the same v4 shape to `star_rail_scan_<timestamp>.json`.

The file is **HSR-Scanner format v4**, the same interchange used by
[HSR-Scanner](https://github.com/kel-z/HSR-Scanner) (`source: "HSR-Scanner"`)
and [Reliquary Archiver](https://github.com/IceDynamix/reliquary-archiver)
(`source: "reliquary_archiver"`). Fribbels Optimizer imports either source.
GOODCapture uses `source: "HSR-Scanner"` because Fribbels allowlists that
string; `generator` records the actual producer.

抓包与截图扫描写同一套 v4 JSON，但文件名前缀不同（`star_rail_capture_` /
`star_rail_scan_`），避免互相覆盖。GOODScanner 的 `source` 固定为 `HSR-Scanner`，
真正的生成器写在 `generator` 里。截图扫描会读取画面上能看到的技能/行迹文字；
无法从画面读出的字段（成就数据包、`ability_version` 等）省略，不另起一套 schema。

## Required v4 fields / 必填 v4 字段

| Field | Meaning |
|---|---|
| `source` | `"HSR-Scanner"` (format discriminator, not the binary name) |
| `build` | `"v1.2.0"` (Fribbels compatibility identifier) |
| `version` | `4` |
| `metadata.uid` | always `null` (account IDs are not exported) |
| `metadata.trailblazer` | `"Stelle"` or `"Caelus"`, else `null` |
| `characters[]` | `id` (string), `name`, `path`, `level`, `ascension`, `eidolon`. Packet captures also include `ability_version`. Screenshot OCR fills `skills`, `traces`, and optional `memosprite` from the traces screen when readable; `ability_version` is omitted because it is not on-screen text. |
| `light_cones[]` | `id` (string), `name`, `level`, `ascension`, `superimposition`, `location`, `lock`, `_uid` |
| `relics[]` | all six slots; `set_id`, `name`, `slot`, `rarity`, `level`, `mainstat`, `substats[{key,value}]`, `location`, `lock`, `discard`, `_uid` |

`path` uses Fribbels names: Destruction, Hunt, Erudition, Harmony, Nihility,
Preservation, Abundance, Remembrance, Elation.

`location` is the equipped character template ID as a string, or `""`.
`_uid` is a local synthetic instance id, never a server item id.

Percent substats use a trailing `_` (`CRIT Rate_`, `HP_`). Flat stats do not.

## GOODScanner extensions / 扩展（省略 = 未观察）

Unknown fields must be ignored by consumers. Omit a field when that data was
not captured; do not invent empty coverage.

未知字段必须忽略。某类数据未被抓取时**省略该字段**，不要写成空集合冒充“已观察”。

| Field | When present |
|---|---|
| `generator` | `{name:"GOODScanner", version, captureRevision, compatibility}` |
| `coverage` | `{characters, lightCones, relics}` each `complete` / `unknown` / … |
| `metadata.current_trailblazer_path` | current Trailblazer path, same names as `characters[].path` |
| `achievements` | sorted unique completed achievement IDs. `[]` means zero completed. **Omitted** means achievements were not observed |

Same rule as Genshin GOOD: a missing `achievements` array leaves consumer
achievement state untouched; a present array (including empty) replaces it.

与原神 GOOD 相同：没有 `achievements` 时消费者应保留原成就数据；有该数组
（包括 `[]`）时以文件为准。

## Minimal example / 最小示例

```json
{
  "source": "HSR-Scanner",
  "build": "v1.2.0",
  "version": 4,
  "generator": {
    "name": "GOODScanner",
    "version": "0.0.0",
    "captureRevision": "reliquary-shape-4.5-v1",
    "compatibility": "HSR-Scanner v1.2.0 / format v4"
  },
  "metadata": {
    "uid": null,
    "trailblazer": "Stelle",
    "current_trailblazer_path": "Remembrance"
  },
  "coverage": {
    "characters": "complete",
    "lightCones": "complete",
    "relics": "complete"
  },
  "characters": [],
  "light_cones": [],
  "relics": [],
  "achievements": [4010101]
}
```

Character, Light Cone, and Relic object shapes follow
[Fribbels' Kelz parser](https://github.com/fribbels/hsr-optimizer/blob/main/src/lib/importer/kelzFormatParser.tsx).
Reliquary's exporter is a second producer of the same v4 inventory objects;
it does not define `achievements` or `current_trailblazer_path`.
