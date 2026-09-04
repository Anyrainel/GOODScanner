# Honkai: Star Rail support / 《崩坏：星穹铁道》支持

`hsr_scanner` is the Star Rail library linked into the normal `GOODScanner`
and `GOODCapture` Windows applications. It is not a separate product and does
not build, download, or launch an HSR-specific executable. The shared
applications keep their existing binary names and let the player switch
between Genshin Impact and Star Rail.

`hsr_scanner` 是正式 `GOODScanner` 与 `GOODCapture` Windows 程序共用的星穹铁道功能库，
不是独立产品，也不会构建、下载或启动额外的 HSR 专用程序。程序名称保持不变，玩家可在应用内切换
《原神》和《崩坏：星穹铁道》。

## Capabilities / 功能

- Screenshot scanning for Characters, Light Cones, Cavern Relics, and Planar
  Ornaments, with explicit category coverage.
- Attended preview/apply management for the reversible lock and discard-mark
  states. Equipped or uncertain gear remains protected, and destructive game
  actions are outside this boundary.
- In-process read-only capture of completed achievements in `GOODCapture`.
  This uses Windows `pktmon` on HSR UDP ports `23301` and `23302`; no helper
  download is required.
- Versioned `goodscanner.hsr` schema-version 3 export for GGStarRail. Inventory
  evidence and achievement packet evidence carry independent coverage.

- 通过截图扫描角色、光锥、隧洞遗器和位面饰品，并分别记录各分类的覆盖状态。
- 有人值守的预览/应用管理流程，只处理可逆的锁定与弃置标记。已装备或状态不确定的装备始终受保护，
  分解、删除、消耗、装备等破坏性操作不在功能范围内。
- `GOODCapture` 可在进程内只读抓取已完成成就。程序通过 Windows `pktmon` 监听 HSR UDP
  端口 `23301` 和 `23302`，无需额外下载辅助程序。
- 使用版本化的 `goodscanner.hsr` schema version 3 导出到 GGStarRail；库存证据与成就抓包证据
  分别记录覆盖状态。

The older Reliquary/Fribbels v4 JSON importer remains available as an optional
offline normalization boundary for existing user files. It is not used by the
normal live-capture flow, and the shared binaries never require or invoke
Reliquary Archiver.

Scanner, import, Manager, and achievement-capture flows use the complete,
SHA-256-verified GIlore 1.2.0 public reference snapshot embedded in both shared
binaries. No reference-data folder is required. The GUI reference-folder field
is an advanced explicit override; when nonempty, the selected bundle must pass
the same strict completeness checks and an invalid override fails closed.

旧版 Reliquary/Fribbels v4 JSON 导入器仍可用于离线转换已有文件，但正式实时抓包流程不会使用它，
共用程序也不会要求或调用 Reliquary Archiver。

扫描、导入、管理及成就抓包流程均默认使用内置在两个共用程序中的完整 GIlore 1.2.0 公开参考快照，
并会校验其 SHA-256；无需准备参考数据目录。界面中的参考目录是高级自定义覆盖；一旦填写，所选数据包
必须通过同样严格的完整性校验，无效覆盖会安全拒绝。

## Achievement capture / 成就抓包

Start capture before HSR establishes its session, then enter the game and open
the Achievements screen if the application asks for it. Stop is cancelable
while the packet source is idle. A capture becomes complete only after a
structurally valid full achievement response is recognized against the current
complete GIlore public-ID set.

请在 HSR 建立会话前开始抓包，再进入游戏；若应用提示，请打开“成就”界面。即使暂时没有收到数据包，
“停止”也会立即取消任务。只有当程序依据当前完整的 GIlore 公共成就 ID 集合，确认收到结构有效的完整
成就响应后，覆盖状态才会变为完整。

Only public achievement definition IDs with completed protocol states are
normalized. IDs are sorted and deduplicated. The export retains no account
identifier, packet bytes, command IDs, raw protocol status, or completion
timestamp. A recognized response with no completed achievements is represented
as a present complete snapshot with an empty `entries` array; no recognized
response is represented by omitting `achievements`.

程序只规范化公共成就定义 ID 及“已完成”状态，并对 ID 排序去重。导出不会保留账号标识、数据包字节、
命令 ID、原始协议状态或完成时间。若已确认完整响应但没有已完成成就，`entries` 会是空数组；若尚未
确认成就响应，则完全省略 `achievements`。

The source revision written by this boundary is `auto-reliquary-1.2.0`. The
dependency itself is pinned by commit in `Cargo.toml`; the decoder does not use
auto-reliquary's sentinel-based achievement matcher. Field layout is inferred
from the complete public-ID set and conservative structural checks.

此边界写入的来源修订为 `auto-reliquary-1.2.0`。依赖已在 `Cargo.toml` 固定到具体提交；解码器不会
使用 auto-reliquary 依赖单一哨兵 ID 的成就匹配器，而是根据完整公共 ID 集合和保守的结构检查推断字段。

## Build and deterministic verification / 构建与确定性验证

Run from the GOODScanner repository root with Rust 1.89 or newer:

请在 GOODScanner 仓库根目录使用 Rust 1.89 或更高版本运行：

```powershell
cargo fmt --all -- --check
cargo test --locked -p hsr_scanner --all-features
cargo clippy --locked -p hsr_scanner --all-targets --all-features -- -D warnings
cargo build --locked --release -p good_tools_app --bin GOODScanner
cargo build --locked --release -p good_tools_app --bin GOODCapture --features capture
```

The achievement fixtures are synthetic decrypted protobuf-command payloads.
They cover field-number rotation, completed-status normalization, unknown public
IDs, sorting/deduplication, truncation/noise, absence, and a recognized empty
snapshot. They contain no PCAP/ETL data and no real account or session data.
Passing them proves replay/normalization behavior only; it does **not** prove a
live HSR capture.

成就夹具是人工合成的已解密 protobuf 命令载荷，覆盖字段编号变化、已完成状态规范化、未知公共 ID、
排序去重、截断/噪声、未识别响应及“已识别但结果为空”等情况。夹具不包含 PCAP/ETL，也不包含真实
账号或会话数据。测试通过只证明回放和规范化逻辑，**不能视为 HSR 实机抓包验证**。

Live screenshot scanning and manager application are attended Windows
operations. Keep the chosen game window in the foreground; focus loss, window
geometry drift, the GUI Stop action, or right-click emergency abort prevents
further input. Automated replay tests likewise do not prove live screenshot
recognition or live game mutation.

实机截图扫描和管理器应用属于有人值守的 Windows 操作。请保持选定游戏窗口位于前台；焦点丢失、
窗口位置/尺寸变化、点击界面“停止”或鼠标右键紧急中止，都会阻止后续输入。自动回放测试同样不能证明
实机截图识别或实机游戏状态变更。

Dependency licenses and the embedded protocol-key provenance are recorded in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

依赖许可证与内置协议密钥来源详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
