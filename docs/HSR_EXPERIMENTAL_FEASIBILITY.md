# Honkai: Star Rail integration status / 《崩坏：星穹铁道》集成状态

Status date: 2026-09-04. Branch: `codex/hsr-capture`. Integration base: `4cea0d7`.

状态日期：2026-09-04。分支：`codex/hsr-capture`。集成基线：`4cea0d7`。

## Unified product contract / 统一产品契约

Honkai: Star Rail is integrated into the existing Windows products. Users do not download or run a separate HSR executable:

- `GOODScanner.exe` contains Genshin and Star Rail Scanner, Export, and Manager flows.
- `GOODCapture.exe` contains those same flows plus in-process achievement packet capture for both games.
- The GUI begins with two equal-width, visibly clickable Genshin and Star Rail choices. Selecting a game changes the available workflow while each game's settings and last selected page remain independent.
- Existing Genshin CLI behavior, `data/good_config.json`, GOOD v3 output, and the optional GOOD achievement extension remain compatible.
- Cross-game navigation and Star Rail settings use the separate versioned `data/good_app_config.json` boundary.

《星穹铁道》现已集成到既有 Windows 产品中，用户无需下载或运行单独的 HSR 可执行文件：

- `GOODScanner.exe` 同时提供《原神》和《星穹铁道》的扫描、导出与管理流程。
- `GOODCapture.exe` 包含上述全部能力，并在程序内为两款游戏提供成就抓包。
- 窗口顶部首先显示两个等宽、静止时也清晰可点击的“原神 / 星穹铁道”选项。切换游戏会切换可用流程，同时分别保留两款游戏的设置与上次页面。
- 现有《原神》命令行行为、`data/good_config.json`、GOOD v3 输出及可选 GOOD 成就扩展保持兼容。
- 跨游戏导航与《星穹铁道》设置使用独立且带版本的 `data/good_app_config.json`。

Star Rail retains screenshot/OCR scanning for Characters, Light Cones, Cavern Relics, and Planar Ornaments; deterministic export; offline Reliquary/Fribbels v4 import; and attended Manager preview/apply for reversible Relic lock and discard-mark changes. Manager apply remains digest-bound, journaled, re-read before each action, and stops on drift or uncertain evidence. It does not salvage, delete, consume, enhance, equip, mutate Light Cones, or inject packets.

《星穹铁道》保留以下能力：通过截图与 OCR 扫描角色、光锥、隧洞遗器和位面饰品；确定性导出；离线导入 Reliquary/Fribbels v4；以及有人值守的管理预览/应用，用于可逆的遗器锁定和弃置标记操作。管理应用仍绑定预览摘要、记录持久日志、每次动作前重新读取，并在状态漂移或证据不确定时停止。它不会分解、删除、消耗、强化、装备、修改光锥或注入数据包。

## Completed-achievement capture / 已完成成就抓包

`GOODCapture.exe` captures HSR UDP traffic in process and emits a snapshot only after recognizing a structurally complete achievement response. It does not download, stage, or launch Reliquary Archiver or any other helper executable. Packet transport/decryption support is compiled into GOODCapture from the revision-pinned dependency and bundled protocol keys; GOODScanner does not contain the capture feature.

`GOODCapture.exe` 在程序内捕获 HSR UDP 流量，并且只会在识别到结构完整的成就响应后生成快照。它不会下载、暂存或启动 Reliquary Archiver 或其他辅助可执行文件。数据包传输/解密能力通过固定修订的依赖和内置协议密钥编译进 GOODCapture；GOODScanner 不包含抓包功能。

Only completed public definition IDs are retained. Raw protocol statuses `2` and `3` both normalize to `"completed"`. IDs are validated against the loaded GIlore references, sorted ascending, and deduplicated. UIDs, player names, account/session/device identifiers, timestamps, progress, raw statuses, server item IDs, packet bytes, protobuf payloads, and PCAP/ETL files are not exported.

程序只保留公开定义中的已完成成就 ID。原始协议状态 `2` 和 `3` 均规范化为 `"completed"`。ID 会根据已加载的 GIlore 参考数据校验、按升序排序并去重。UID、玩家名、账号/会话/设备标识、时间戳、进度、原始状态、服务器物品 ID、数据包字节、protobuf 载荷及 PCAP/ETL 文件均不会导出。

## `goodscanner.hsr` v3 export contract / `goodscanner.hsr` v3 导出契约

New Star Rail exports use `schema: "goodscanner.hsr"` and `schemaVersion: 3`. Inventory provenance remains in top-level `source`; achievement provenance is independent and nested under the optional `achievements` field.

新的《星穹铁道》导出使用 `schema: "goodscanner.hsr"` 与 `schemaVersion: 3`。库存来源继续位于顶层 `source`；成就来源独立记录在可选的 `achievements` 字段中。

```json
{
  "schema": "goodscanner.hsr",
  "schemaVersion": 3,
  "source": {
    "kind": "packetCapture",
    "revision": "auto-reliquary-1.2.0",
    "coverage": {
      "characters": "unknown",
      "lightCones": "unknown",
      "relics": "unknown"
    }
  },
  "reference": {
    "schemaVersion": 1,
    "provider": "gilore.ggstarrail-reference",
    "revision": "<GIlore bundle revision>"
  },
  "privacy": {
    "accountIdentifiersIncluded": false,
    "rawPacketDataIncluded": false,
    "serverItemIdentifiersIncluded": false
  },
  "characters": [],
  "lightCones": [],
  "relics": [],
  "planarOrnaments": [],
  "achievements": {
    "source": {
      "kind": "packetCapture",
      "revision": "auto-reliquary-1.2.0"
    },
    "coverage": "complete",
    "entries": [
      { "achievementId": 4010101, "status": "completed" },
      { "achievementId": 4040201, "status": "completed" }
    ]
  }
}
```

Merge semantics are intentionally tri-state:

- `achievements` omitted: achievements were unavailable or not observed; preserve the consumer's existing achievement state.
- `achievements` present with `coverage: "complete"`: replace the consumer's completed-achievement state from `entries`.
- `achievements.entries: []` with complete coverage: a confirmed empty completed set; clear only the prior completed-achievement state.
- An achievement-only export has empty inventory arrays and `unknown` inventory coverage; consumers preserve existing inventory.

合并语义刻意保留三种状态：

- 省略 `achievements`：成就不可用或本次未观测；导入方保留现有成就状态。
- 存在 `achievements` 且 `coverage: "complete"`：导入方以 `entries` 替换已完成成就状态。
- 完整覆盖且 `achievements.entries: []`：这是已经确认的空集合；仅清空原有已完成成就状态。
- 仅含成就的导出会保留空库存数组并把库存覆盖标为 `unknown`；导入方应保留现有库存。

The achievement object is strict: its only source kind is `packetCapture`, its only coverage value is `complete`, and its only entry status is `completed`. Unknown fields/literals, zero or unknown IDs, duplicate IDs, and unsorted IDs are rejected. The canonical v3 fixture is `experimental/hsr/tests/fixtures/achievement_only_export_v3.json`. The old `goodscanner.hsr.experimental` v2 golden remains unchanged only as a regression fixture; new exports use v3.

成就对象采用严格校验：唯一来源类型是 `packetCapture`，唯一覆盖值是 `complete`，唯一条目状态是 `completed`。未知字段/字面量、零值或未知 ID、重复 ID 及未排序 ID 都会被拒绝。规范 v3 夹具为 `experimental/hsr/tests/fixtures/achievement_only_export_v3.json`。旧 `goodscanner.hsr.experimental` v2 黄金文件仅作为回归夹具原样保留；新导出使用 v3。

## GIlore reference contract / GIlore 参考数据契约

Both shared binaries embed a compact, checksummed snapshot of the complete public GIlore `1.2.0` reference. It is the default for Scanner, offline import, Manager, and achievement capture/export, so a normal installation requires only the existing executable. The embedded loader verifies its bytes and metadata before constructing the same strict `ReferenceCache` used for external bundles; failure stops the requested flow.

The snapshot was derived from clean GIlore commit `7ef3650a63622c204b89234406c99dc221e01d85`, public source revision `8cdb905dc2f8e6fffa9be4eb07af3e34435d6091`, and source-manifest SHA-256 `9899cc8fdde578cdbd744ec9f8b2705cd2f11d43670232e871f489fc3d549b5f`. Its deterministic normalized JSON is 331,391 bytes with SHA-256 `46fdddc9252046580154823d596315fb09d88dab570ffa958787286c99d76782`; it contains public reference metadata only, with no icon binaries, account/session/capture data, or raw packets.

Advanced users may explicitly select an external GIlore reference folder. A nonblank selection is an override, never a soft hint: it must be a complete, checksummed production bundle and an invalid override fails closed instead of silently falling back to embedded data. Its `manifest.json` must include the established inventory files plus these exact members:

- `achievement_categories.json`, with `collection: "achievement_categories"`
- `achievements.json`, with `collection: "achievements"`

两个共用程序都内置一份紧凑且带校验和的完整公开 GIlore `1.2.0` 参考快照。扫描、离线导入、管理以及成就抓包/导出默认都使用这份数据，因此正常安装只需要既有可执行文件。内置加载器会先验证数据字节与元数据，再构造与外部数据包相同的严格 `ReferenceCache`；验证失败会停止当前流程。

该快照来自干净的 GIlore 提交 `7ef3650a63622c204b89234406c99dc221e01d85`、公开源修订 `8cdb905dc2f8e6fffa9be4eb07af3e34435d6091`，源清单 SHA-256 为 `9899cc8fdde578cdbd744ec9f8b2705cd2f11d43670232e871f489fc3d549b5f`。确定性规范化 JSON 共 331,391 字节，SHA-256 为 `46fdddc9252046580154823d596315fb09d88dab570ffa958787286c99d76782`；其中仅含公开参考元数据，不含图标二进制、账号/会话/抓包数据或原始数据包。

高级用户可以明确选择外部 GIlore 参考目录。非空选择属于强制覆盖而非建议路径：目录必须是完整、校验和正确的生产数据包；覆盖无效时会安全拒绝，不会静默回退到内置数据。其 `manifest.json` 除既有库存文件外，还必须包含以下准确成员：

- `achievement_categories.json`，其中 `collection: "achievement_categories"`
- `achievements.json`，其中 `collection: "achievements"`

Each member identifies `bundle_id: "ggstarrail-reference"`, `game_id: "honkai_star_rail"`, `schema_version: "1.2.0"`, and the same public `source_revision`. Every achievement record's `id` is a JSON number that must fit a nonzero `u32`; duplicates, overflow, missing members, count/checksum mismatches, and unrecognized IDs fail closed. Leaving the GUI override blank selects the verified embedded snapshot.

每个成员都标识 `bundle_id: "ggstarrail-reference"`、`game_id: "honkai_star_rail"`、`schema_version: "1.2.0"` 以及一致的公开 `source_revision`。每条成就记录的 `id` 都是 JSON 数值，必须能表示为非零 `u32`；重复、溢出、成员缺失、数量/校验和不符或无法识别的 ID 都会安全拒绝。图形界面的自定义覆盖留空时，即选择已校验的内置快照。

## Normal builds and validation / 正常构建与验证

Run from the repository root. These are the normal shared product targets; no HSR-specific executable or release target exists.

请在仓库根目录运行。以下是正常的共用产品目标；不存在 HSR 专用可执行文件或发布目标。

```powershell
cargo fmt --all -- --check
cargo test --locked --workspace --exclude good_tools_app --all-targets
cargo test --locked -p good_tools_app --features test-as-invoker --all-targets
cargo test --locked -p genshin_scanner --features capture --all-targets
cargo test --locked -p hsr_scanner --all-features --all-targets
cargo test --locked -p good_tools_app --features "capture,test-as-invoker" --all-targets
cargo check --locked --workspace --all-targets
cargo check --locked -p genshin_scanner --features capture --all-targets
cargo check --locked -p hsr_scanner --all-features --all-targets
cargo check --locked -p good_tools_app --features "capture,test-as-invoker" --all-targets
cargo clippy --locked -p hsr_scanner --all-features --all-targets --no-deps -- -D warnings
cargo clippy --locked -p good_tools_app --features "capture,test-as-invoker" --all-targets --no-deps -- -D warnings
cargo build --locked --release -p good_tools_app --bin GOODScanner
cargo build --locked --release -p good_tools_app --features capture --bin GOODCapture
```

The workspace does not use one blanket `--all-features` invocation because `yas_core` exposes mutually exclusive OCR backends. The supported default and capture feature matrices above are checked separately.

工作区不会执行一次笼统的 `--all-features`，因为 `yas_core` 暴露互斥的 OCR 后端。上面会分别检查受支持的默认组合与抓包组合。

The concrete build outputs remain:

```text
target\release\GOODScanner.exe
target\release\GOODCapture.exe
```

The branch must remain isolated: do not merge, push, deploy, replace release binaries, or copy these artifacts over the main checkout as part of this work.

本分支必须保持隔离：本次工作不得合并、推送、部署、替换发布文件，也不得把这些产物覆盖到主工作区。

## Evidence boundary and live validation / 证据边界与实机验证

Protocol fixtures, packet replay, generated/sanitized images, and simulated controllers prove deterministic parsing, normalization, export, routing, and safety behavior. They do **not** prove live HSR capture, current-client OCR coordinates, focus/input behavior, HDR or non-16:9 layouts, channel variants, or live Manager mutations.

协议夹具、数据包回放、生成/脱敏图像与控制器模拟可以证明确定性解析、规范化、导出、路由及安全行为；它们**不能**证明 HSR 实时抓包、当前客户端 OCR 坐标、焦点/输入行为、HDR 或非 16:9 布局、渠道服差异或实时管理变更。

The smallest remaining achievement-capture validation is attended and read-only: open the current HSR client at its “Click to Start” screen, run this branch's `target\release\GOODCapture.exe` as administrator, choose Star Rail → Capture, start capture, and enter the game once. Verify that the resulting v3 JSON imports into GGStarRail. Until that is performed against a live client, live HSR capture remains explicitly unvalidated. Do not perform a Manager mutation during this calibration.

剩余最小的成就抓包验证需要有人值守，但只读：把当前 HSR 客户端停在“点击进入”界面，以管理员身份运行本分支的 `target\release\GOODCapture.exe`，选择“星穹铁道 → 抓包”，开始抓包后进入游戏一次，再确认生成的 v3 JSON 可导入 GGStarRail。在当前实机客户端完成该步骤前，HSR 实时抓包仍明确视为未验证。校准期间不得执行管理变更。
