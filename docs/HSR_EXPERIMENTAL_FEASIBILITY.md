# Experimental Honkai: Star Rail implementation status / 《崩坏：星穹铁道》实验实现状态

Status date: 2026-09-02. Branch: `codex/hsr-capture`. Starting revision: `754cf8e`.

状态日期：2026-09-02。分支：`codex/hsr-capture`。起始修订：`754cf8e`。

## Outcome and evidence boundary / 结果与证据边界

`experimental/hsr` is now an isolated Windows application with four real application paths:

- screenshot capture, HSR UI navigation, grid scrolling, and bilingual OCR for Characters, Light Cones, Cavern Relics, and Planar Ornaments;
- privacy-filtered import from a user-supplied, SHA-256-pinned Reliquary Archiver v0.18.0 helper, plus offline Reliquary/Fribbels v4 JSON import;
- deterministic `goodscanner.hsr.experimental` v2 export using a complete versioned GGStarRail/GIlore reference bundle; and
- exact manager preview plus a separate, digest-bound apply command for reversible Relic lock/unlock and discard-mark/unmark actions.

`experimental/hsr` 现已成为隔离的 Windows 应用，并提供四条真实应用路径：

- 对角色、光锥、隧洞遗器和位面饰品进行截图捕获、HSR 界面导航、网格滚动及双语 OCR；
- 通过用户自行提供且 SHA-256 固定的 Reliquary Archiver v0.18.0 辅助程序进行隐私过滤导入，并支持离线导入 Reliquary/Fribbels v4 JSON；
- 使用完整且版本化的 GGStarRail/GIlore 参考数据生成确定性的 `goodscanner.hsr.experimental` v2 导出；
- 提供精确管理预览，以及单独运行、绑定摘要的应用命令，仅处理可逆的遗器锁定/解锁和弃置标记/取消标记。

The implementation has fixture, generated-image OCR, replay-controller, privacy, compatibility, and release-isolation tests. Those tests do **not** prove current real-client coordinates, OCR accuracy, packet capture, or any live manager mutation. No real-device scan or mutation is claimed as validated. Live mutation must not be attempted without a separate explicit user confirmation after an attended preview and calibration pass.

实现包含夹具、生成图像 OCR、控制器回放、隐私、兼容性及发布隔离测试。这些测试**不能**证明当前实机客户端坐标、OCR 准确率、实时抓包或任何实时管理变更。本文不声称实机扫描或变更已验证。在有人值守完成预览与校准后，仍须获得用户单独明确确认，方可尝试实时变更。

## Isolation / 隔离边界

- The root Cargo workspace excludes `experimental/hsr`.
- `HSRScannerExperimental` requires the non-default `experimental-hsr` feature and has distinct executable metadata and an administrator manifest.
- Official `GOODScanner` and `GOODCapture` manifests, GUI routes, updater paths, binaries, and release workflows contain no HSR feature or target.
- This work must not be merged, pushed, deployed, or substituted for an official release as part of this experiment.

- 根 Cargo 工作区排除 `experimental/hsr`。
- `HSRScannerExperimental` 需要非默认的 `experimental-hsr` 功能，并使用独立可执行文件元数据和管理员清单。
- 正式 `GOODScanner` 与 `GOODCapture` 的清单、GUI 路由、更新器、二进制和发布流程均不包含 HSR 功能或目标。
- 本实验不得合并、推送、部署，也不得替换正式发布文件。

## Pinned implementation research / 固定的实现研究来源

Only behavior and contracts were studied from the following revisions. No GPL code or assets were copied.

以下修订仅用于研究行为和契约；未复制 GPL 代码或资源。

| Project / 项目 | Pinned revision / 固定修订 | Use / 用途 |
|---|---|---|
| [Reliquary Archiver](https://github.com/IceDynamix/reliquary-archiver/tree/cb109f17a4a15b7604cfe9d078a8735e7735cd25) | `cb109f17a4a15b7604cfe9d078a8735e7735cd25`, v0.18.0 | Current HSR 4.5 full-account capture behavior and Fribbels v4 export boundary; MIT. / 当前 HSR 4.5 全账号捕获行为及 Fribbels v4 导出边界；MIT。 |
| [Reliquary](https://github.com/IceDynamix/reliquary/tree/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b) | `d5cf3b7e7e66470d2d8efff6676aa18762b21d3b`, v23.0.0 | HSR 4.5 protocol provenance only. The experimental app does not implement or inject the protocol. / 仅用于 HSR 4.5 协议来源；实验应用不实现也不注入该协议。 |
| [Kel-Z HSR-Scanner](https://github.com/kel-z/HSR-Scanner/tree/cad008e2509b6b086fd892b140c8b277424fcb03) | `cad008e2509b6b086fd892b140c8b277424fcb03`, v1.5.0 | Current OCR fallback behavior; bundled reference data reported HSR 4.4, so it is not used as authoritative data; GPL-3.0. / 当前 OCR 后备行为；其内置参考数据为 HSR 4.4，因此不作为权威数据；GPL-3.0。 |
| [Original YAS HSR](https://github.com/wormtql/yas/tree/730d8845505c04c7f4c91709de076503401ff40c/yas-starrail) | `730d8845505c04c7f4c91709de076503401ff40c` | Historical generic capture/input concepts only; last HSR update was 2.5 and is stale; GPL-2.0. / 仅参考历史通用捕获/输入概念；最后更新停在 HSR 2.5，已经过时；GPL-2.0。 |
| [Fribbels HSR Optimizer](https://github.com/fribbels/hsr-optimizer/tree/df630a0488a64eeb740e4e0c14f265d96b9f6f8f) | `df630a0488a64eeb740e4e0c14f265d96b9f6f8f` | Community interoperability semantics; MIT. / 社区互操作语义；MIT。 |

The application does not download or bundle Reliquary Archiver. Live helper execution requires the operator to supply the exact v0.18.0 executable and an independently approved SHA-256. The approved bytes are copied into a private temporary directory, the staged copy is re-hashed, and only that staged path is executed. Helper stdout/stderr are suppressed and raw output is bounded. Account and server-item identifiers are discarded before normalization, while raw-packet fields are rejected. Explicit cleanup is attempted on every return; if absence cannot be proven, the command aborts with `HSR-CAPTURE-CLEANUP` and reports the private path for attended cleanup.

应用不会下载或捆绑 Reliquary Archiver。实时运行辅助程序时，操作员必须提供准确的 v0.18.0 可执行文件及独立核准的 SHA-256。已核准字节会复制到私有临时目录，暂存副本会再次校验哈希，并且只执行该暂存路径。辅助程序的标准输出和错误输出会被抑制，原始输出受大小限制。账号和服务器物品标识会在规范化前丢弃，原始数据包字段则会被拒绝。每条返回路径都会显式尝试清理；如果无法证明路径已删除，命令会以 `HSR-CAPTURE-CLEANUP` 中止并报告私有路径，供有人值守清理。

## Data and privacy contract / 数据与隐私契约

The current export is `schema: "goodscanner.hsr.experimental"`, `schemaVersion: 2`. `source.kind` is exactly `screenCapture`, `packetCapture`, or `sanitizedFixture`. Coverage for Characters, Light Cones, and Relics is explicit and never upgraded from `unknown` without source evidence. Public item identity is the GIlore string `key` plus numeric public `gameId`; stat records contain only `{key,name,value}`. Ratio values are display percentage points.

当前导出使用 `schema: "goodscanner.hsr.experimental"` 与 `schemaVersion: 2`。`source.kind` 只能是 `screenCapture`、`packetCapture` 或 `sanitizedFixture`。角色、光锥和遗器覆盖范围均显式记录；没有来源证据时绝不会将 `unknown` 升级。物品公开身份由 GIlore 字符串 `key` 和数字公开 `gameId` 构成；属性只包含 `{key,name,value}`，比例值使用界面显示的百分点。

The three export privacy flags are literal `false`: account identifiers, raw packet data, and server item identifiers are absent. Cookies, tokens, UIDs, player names, server GUIDs, PCAP/ETL, protobuf payloads, and full screenshots are neither accepted as normal fixtures nor emitted. Run-local IDs are generated from deterministic category/index order, never from server IDs.

三个导出隐私标志固定为 `false`：不包含账号标识、原始数据包和服务器物品标识。Cookie、令牌、UID、玩家名、服务器 GUID、PCAP/ETL、protobuf 载荷及完整截图均不会作为普通夹具接受，也不会被导出。运行内本地 ID 按确定性的类别/索引顺序生成，绝不来自服务器 ID。

Visible Relic identity follows the shared GGStarRail rule. Candidates must share catalog set, slot, rarity, localized English and zh-CN piece names, icon path, and the observed canonical property's exact generated main-affix progression (`maxLevel` plus every level value). A supplied public key/game ID must belong to that equivalence class, which canonicalizes to its lowest numeric public RelicConfig ID; more than one remaining visible/progression class is an error. The Passerby 4-star Body healing variants `51013` and `55001` therefore normalize to `51013`.

可见遗器身份遵循与 GGStarRail 共享的规则。候选项必须具有相同的目录套装、部位、稀有度、中英文遗器名称、图标路径，以及观测到的规范属性所对应的完整生成主属性成长序列（`maxLevel` 与每一级数值）。输入的公开 key/game ID 必须属于该等价类，最终规范化为类中数值最小的公开 RelicConfig ID；若仍跨越多个可见/成长等价类则报错。因而，过客四星躯干治疗属性的 `51013` 与 `55001` 都规范化为 `51013`。

## Screenshot scanner / 截图扫描器

The scanner uses the generic YAS capture and input primitives where their contracts match, while keeping HSR-specific geometry and OCR in the experimental crate. It selects a foreground HSR window, captures through WGC/BitBlt/PrintWindow, discovers visible grid geometry, visits cells row-major, verifies selection/panel changes, scrolls with screenshot-based progress/end checks, and stops on focus loss, right-click cancellation, unstable panels, geometry drift, or ambiguous terminal evidence. Light Cone and gear completeness requires two agreeing initial quantity reads, a confirming terminal quantity read, no next cell, two attended wheel events that both show no advance, and two independent scrollbar-thumb readings at the bottom. Characters use bounded identity-change/wrap detection; complete Character coverage additionally requires the operator's exact expected count, a stable next-panel wrap to the first Character, and no unresolved visible identity.

扫描器在契约匹配处复用通用 YAS 捕获与输入基础设施，HSR 专属几何和 OCR 则留在实验 crate。它会选择前台 HSR 窗口，通过 WGC/BitBlt/PrintWindow 截图，发现可见网格几何，按行优先访问单元格，校验选择与面板变化，依据截图验证滚动进度和终点；失焦、右键取消、面板不稳定、几何漂移或终点证据不明确时立即停止。光锥和装备只有在两次初始数量读数一致、终点数量再次确认、不存在下一格、两次有人值守的滚轮输入均未继续前进，且两次滚动条滑块读数都独立证明位于底端时，才会声明完整。角色扫描使用有界的身份变化/回绕检测；只有操作员提供准确的预期角色数量、下一张稳定面板回绕至首名角色，且不存在未解决的画面身份歧义时，才能声明角色覆盖完整。

OCR recognizes English and Chinese visible labels and resolves them against the loaded reference cache. Equipped state and managed icons fail closed: blank, low-confidence, conflicting, or weak evidence becomes unknown and therefore preview-only. Full screenshots are not retained by the normal flow.

OCR 可识别中英文可见标签，并使用已加载的参考缓存进行解析。装备状态和管理图标遵循关闭失败原则：空白、低置信度、冲突或薄弱证据都会变为未知，因此只能预览。正常流程不会保留完整截图。

## Manager contract / 管理器契约

The manager consumes `goodscanner.hsr.manager-instructions` schema version 1. Before any device is created, it validates schema/provider/revision and every public matcher against the loaded reference, including gear identity, set, rarity, slot, maximum level, authoritative main-stat progression value, substat keys, and public Character location.

管理器读取 `goodscanner.hsr.manager-instructions` 架构版本 1。在创建任何设备前，它会校验架构/提供者/修订以及每个公开匹配器，包括装备身份、套装、稀有度、槽位、最高等级、权威主属性成长值、副属性键和公开角色位置。

`manager preview` performs a fresh gear scan and prints an exact bilingual summary plus complete JSON plan. Equipped/possibly equipped gear, ambiguous duplicates, unknown or mismatched before-state, low-confidence state evidence, and any request to mark a currently locked Relic for discard are preview-only. Actionable entries require both fresh lock and discard states to be known and equal the instruction's complete `before` state. Unlocking and marking discard require two separately scanned, reviewed, and digest-confirmed runs.

`manager preview` 会重新扫描装备，并打印精确双语摘要及完整 JSON 计划。已装备/可能已装备物品、重复歧义、未知或不匹配的动作前状态、低置信度状态证据，以及任何试图把当前已锁定遗器标记弃置的请求都只能预览。可执行条目要求最新读取的锁定与弃置状态均已知，且完整等于指令中的 `before` 状态。解锁和标记弃置必须分成两次重新扫描、审核并确认摘要的运行。

For a new journal, `manager apply` rebuilds the plan from another fresh scan. For a retry, the journal is consulted first and the original exact plan is recovered without silently replacing it with current state. Both paths reprint the selected exact plan before mutation. Apply requires a separate `--confirm-live-mutation`, the full preview digest, least-privilege action scopes, a durable append-only journal, a per-user operating-system controller lease shared with screenshot scan and preview, and an additional exclusive lease on that journal. Each item is re-read immediately before one toggle and verified immediately afterward. Any drift or uncertain result stops the batch; restart recovery reconciles a possibly started mutation and never blindly retries it. A truncated final journal line is repaired to the last verified newline under the exclusive lease before any append. Salvage, delete, consume, enhance, equip/unequip, Light Cone mutation, and packet injection do not exist in this path.

使用新日志时，`manager apply` 会再次重新扫描并重建计划；重试时会先查询日志并恢复原先的精确计划，而不会用当前状态静默替换它。两条路径都会在变更前重新打印选定的精确计划。应用命令要求单独的 `--confirm-live-mutation`、完整预览摘要、最小权限动作范围、持久追加式日志、与截图扫描和预览共享的每用户操作系统控制器锁，以及该日志的额外独占锁。每件物品只会在变更前立即重读、切换一次并在之后立即验证。任何漂移或不确定结果都会停止整个批次；重启恢复会核对可能已开始的变更，绝不会盲目重试。日志末尾若截断，会先在独占锁下修复到最后一个已验证换行，再进行任何追加。此路径不存在分解、删除、消耗、强化、装备/卸下、光锥变更或数据包注入。

## Exact safe build and fixture run / 精确安全构建与夹具运行

Run these commands from `C:\Users\vanyr\.codex\worktrees\23ad\GOODScanner`. This path exercises the production CLI and committed sanitized fixture only; it does not open HSR, capture traffic, or issue input. The output path must not already exist because exports never overwrite files.

请在 `C:\Users\vanyr\.codex\worktrees\23ad\GOODScanner` 中运行以下命令。该路径只执行正式 CLI 和已提交的脱敏夹具；不会打开 HSR、捕获流量或发送输入。导出绝不覆盖文件，因此输出路径必须尚不存在。

```powershell
cargo build --locked --release --manifest-path .\experimental\hsr\Cargo.toml --features experimental-hsr --bin HSRScannerExperimental
$exe = ".\experimental\hsr\target\release\HSRScannerExperimental.exe"
& $exe --lang en fixture-export `
  --reference-bundle .\experimental\hsr\tests\fixtures\gilore_bundle `
  --input .\experimental\hsr\tests\fixtures\observations.json `
  --output .\hsr-experimental-safe-fixture.json
cargo test --locked --manifest-path .\experimental\hsr\Cargo.toml --all-features --test cli_contract fixture_export_runs_through_the_production_cli_and_never_overwrites -- --exact
```

The executable's operational CLI surfaces are:

```text
HSRScannerExperimental scan ...
HSRScannerExperimental capture ...
HSRScannerExperimental import-capture ...
HSRScannerExperimental manager preview ...
HSRScannerExperimental manager apply ...
```

For any real inventory operation, use a current complete GGStarRail-generated bundle, not the tiny committed fixture. On this development machine the current bundle path is:

任何真实库存操作都必须使用 GGStarRail 当前生成的完整数据包，而不能使用仓库内的极小测试夹具。本开发机上的当前数据包路径为：

```powershell
$referenceBundle = "F:\Codes\genshin\GGStarRail\src\generated\hsr-reference"
```

See `experimental/hsr/README.md` for complete bilingual command examples and attended safety requirements.

完整双语命令示例和有人值守安全要求见 `experimental/hsr/README.md`。

## Static validation gate / 静态验证门槛

The final branch must pass all of the following from the isolated worktree:

最终分支必须在隔离工作树中通过以下全部检查：

```powershell
cargo fmt --manifest-path .\experimental\hsr\Cargo.toml -- --check
cargo clippy --locked --manifest-path .\experimental\hsr\Cargo.toml --all-features --all-targets -- -D warnings
cargo test --locked --manifest-path .\experimental\hsr\Cargo.toml --all-features --all-targets --no-fail-fast
cargo check --locked --manifest-path .\experimental\hsr\Cargo.toml --all-features --all-targets
cargo build --locked --release --manifest-path .\experimental\hsr\Cargo.toml --features experimental-hsr --bin HSRScannerExperimental
cargo test --locked --manifest-path .\experimental\hsr\Cargo.toml --all-features --test release_isolation
cargo check -p good_tools_app --bin GOODScanner
cargo check -p good_tools_app --features capture --bin GOODCapture
git diff --exit-code 754cf8e -- . ':(exclude)experimental/hsr/**' ':(exclude)docs/HSR_EXPERIMENTAL_FEASIBILITY.md'
git diff --check
```

The canonical scanner-produced v2 JSON golden is `experimental/hsr/tests/fixtures/expected_export.json`. GGStarRail must consume that same file byte-for-byte in its scanner compatibility test; variable provenance or display labels must not be left as an implicit compatibility assumption.

扫描器生成的规范 v2 JSON 黄金文件为 `experimental/hsr/tests/fixtures/expected_export.json`。GGStarRail 必须在扫描器兼容测试中逐字节读取同一文件；不得将可变来源或显示名称作为隐含兼容假设。

The canonical golden's provenance is explicitly `screenCapture/screen-capture-fixture-v2`. The safe fixture CLI emits `sanitizedFixture/sanitized-gilore-v2`; a scanner regression normalizes only those two explicit provenance fields and then requires the complete JSON values to be equal. Display labels are not treated as variable.

规范黄金文件的来源明确为 `screenCapture/screen-capture-fixture-v2`。安全夹具 CLI 输出 `sanitizedFixture/sanitized-gilore-v2`；扫描器回归测试只规范化这两个显式来源字段，随后要求完整 JSON 值相等。显示名称不被视为可变字段。

The finalized consumer proof is GGStarRail commit `58e83cd58319d1e13381de7fc5c87054a1c91aba`. Its scanner-v2 fixture is byte-identical to this scanner's 3,105-byte golden, SHA-256 `3734ff182b1e7d6511a00262457ed9e219a103c7f9a1e1baaf0f596d90a2d373`. The manager-instructions fixtures are also byte-identical at 1,204 bytes, SHA-256 `ae7302fdd6a017c8d92ecadc6b82ecb9e82c54520ea41781309adfc69df67570`, and carry semantic idempotency key `sha256:0525633c88fcce130bdedf285db6ac865f8654654b40e069d1aa86a5f21917ce`.

最终消费端证明位于 GGStarRail 提交 `58e83cd58319d1e13381de7fc5c87054a1c91aba`。其 scanner-v2 夹具与本扫描器 3,105 字节的黄金文件逐字节一致，SHA-256 为 `3734ff182b1e7d6511a00262457ed9e219a103c7f9a1e1baaf0f596d90a2d373`。管理指令夹具也以 1,204 字节逐字节一致，SHA-256 为 `ae7302fdd6a017c8d92ecadc6b82ecb9e82c54520ea41781309adfc69df67570`，并使用语义幂等键 `sha256:0525633c88fcce130bdedf285db6ac865f8654654b40e069d1aa86a5f21917ce`。

## Remaining attended live-device step / 剩余有人值守实机步骤

All current UI coordinates, OCR crops, confidence thresholds, inventory quantities, scroll behavior, character-next input, and icon transitions are proven only by generated/sanitized imagery and replay simulation. A current HSR client must be opened at the requested safe screen for a read-only calibration scan before any real-device success claim. Character traversal's default `E` binding is specifically unproven; public Kel-Z behavior uses virtual-controller right-shoulder navigation. HDR, non-16:9 layouts, localization variants, official/Bilibili channel behavior, capture-helper permissions, persistence after inventory reopen/client restart, and live lock/discard transitions remain unvalidated.

当前所有界面坐标、OCR 裁剪、置信度阈值、库存数量、滚动行为、角色切换输入和图标转换都只由生成/脱敏图像及回放模拟证明。必须先在当前 HSR 客户端的指定安全界面执行只读校准扫描，之后才能声称实机成功。角色遍历默认 `E` 绑定尤其尚未验证；公开 Kel-Z 行为使用虚拟控制器右肩键。HDR、非 16:9 布局、语言变体、官服/B 服行为、捕获辅助程序权限、背包重开/客户端重启后的持久性，以及实时锁定/弃置转换均未验证。

No live manager mutation may be performed during calibration. If the client is unavailable, implementation and static validation can be completed and committed, but the live-device gap must remain explicitly open.

校准期间不得执行任何实时管理变更。如果客户端不可用，可以完成并提交实现与静态验证，但必须明确保留实机验证缺口。
