# Experimental Honkai: Star Rail feasibility report / 《崩坏：星穹铁道》实验性可行性报告

Audit date: 2026-08-30. Branch: `codex/hsr-capture`. Base revision: `76dc3b9c8e566f984abe6228d81b5b2bf3bc2d1b`.

审计日期：2026-08-30。分支：`codex/hsr-capture`。基线修订：`76dc3b9c8e566f984abe6228d81b5b2bf3bc2d1b`。

## Conclusion / 结论

The safe evidence-backed milestone is feasible as an isolated semantic-data slice, not yet as a live scanner. The implemented code converts sanitized observations and versioned bilingual references into a deterministic experimental export containing Characters, Light Cones, Relics, and Planar Ornaments. It contains no live capture, input control, packet decryption, account identity, or mutation path.

基于现有证据，安全里程碑可实现为隔离的语义数据切片，但尚不能称为实时扫描器。已实现代码将脱敏观测和版本化双语参考数据转换为确定性的实验导出，覆盖角色、光锥、遗器和位面饰品；其中不包含实时捕获、输入控制、数据包解密、账号身份或状态变更路径。

Two later capture directions are technically possible but have different contracts. OCR/controller capture observes rendered UI and can avoid protocol secrets, but current public implementations depend on fixed 16:9 positions and OCR heuristics. Packet capture preserves exact hidden values, but current HSR tooling depends on patch-specific KCP/XOR/protobuf definitions, command IDs, and session-key negotiation. Neither mechanism is enabled here.

后续有两条技术路线，但契约不同。OCR/控制器捕获读取渲染后的界面，可避免协议密钥，但现有公开实现依赖固定 16:9 坐标和 OCR 启发式。抓包可保留隐藏精确值，但当前 HSR 工具依赖每个版本变化的 KCP/XOR/protobuf 定义、命令 ID 和会话密钥协商。本里程碑未启用任何一种机制。

## Repository and upstream history / 仓库与上游历史

Exact local Git evidence shows that GOODScanner descends from `wormtql/yas`, whose current audited upstream revision is [`757689f062d0f3c2dc0bd48e16544f5f6e8e78d1`](https://github.com/wormtql/yas/commit/757689f062d0f3c2dc0bd48e16544f5f6e8e78d1) (`v0.1.21`). That revision is an ancestor of this fork.

本地 Git 精确证据表明 GOODScanner 源自 `wormtql/yas`；本次审计的上游修订为 [`757689f062d0f3c2dc0bd48e16544f5f6e8e78d1`](https://github.com/wormtql/yas/commit/757689f062d0f3c2dc0bd48e16544f5f6e8e78d1)（`v0.1.21`），并且是当前分叉的祖先。

| Revision / 修订 | Evidence / 证据 |
|---|---|
| `2359d4b9960a` | Initial Star Rail support on 2023-09-03. / 2023-09-03 首次加入星穹铁道支持。 |
| `2401e39a6a74` | Introduced the dedicated `yas-starrail` crate. / 引入独立 `yas-starrail` crate。 |
| `94eb2a460a50` | Read relic lock/discard fields from the UI. / 从界面读取遗器锁定与弃置字段。 |
| `433e2ff8ab25` | Added legacy HSR-Scanner v3 export. / 加入旧版 HSR-Scanner v3 导出。 |
| [`730d8845505c04c7f4c91709de076503401ff40c`](https://github.com/wormtql/yas/commit/730d8845505c04c7f4c91709de076503401ff40c) | Last HSR path change, “Support StarRail 2.5,” on 2024-09-11. / 最后一次 HSR 路径变更，2024-09-11 的“Support StarRail 2.5”。 |
| `49a7deaab39790394bf5f5f467a6b4b8686d3abc` | This fork removed all legacy HSR and Wuthering Waves paths on 2026-03-09. / 当前分叉于 2026-03-09 删除全部旧 HSR 与鸣潮路径。 |

The legacy HSR implementation was OCR/controller capture only. It used a fixed 1920×1080 layout, a fixed 9×5 relic grid, exact pixel/color tables, simulated clicks and scrolling, and Chinese OCR. Repository searches found no HSR packet, protobuf, UDP, pktmon, dispatch-key, or command-ID implementation and no HSR fixtures/tests.

旧 HSR 实现仅属于 OCR/控制器捕获。它使用固定 1920×1080 布局、固定 9×5 遗器网格、精确像素/颜色表、模拟点击与滚动以及中文 OCR。仓库搜索未发现 HSR 抓包、protobuf、UDP、pktmon、派发密钥或命令 ID 实现，也没有 HSR 夹具/测试。

It scanned Relics only. Planar Sphere and Link Rope were represented as Relic slots, while legacy v3 export always emitted empty `characters` and `light_cones` arrays. Its HSR tree did not change after the 2.5 revision through upstream `757689f`; wholesale restoration would revive stale coordinates, data, dependencies, and nightly-coroutine assumptions.

它只扫描遗器。位面球和连结绳作为遗器槽位表示，旧 v3 导出的 `characters` 与 `light_cones` 始终为空数组。从 2.5 修订到上游 `757689f`，HSR 文件树没有变化；整体恢复会重新引入过时坐标、数据、依赖和 nightly coroutine 假设。

Only genuinely generic contracts are suitable for later reuse: `yas_core` capture/OCR traits, game-window discovery, positioning utilities, cancellation, and bilingual logging. The current Genshin controller/backpack scanner hard-codes Genshin window/world checks, panel geometry, an 8×5 grid, and scroll behavior, so it is not reused by this slice.

未来只适合复用真正通用的契约：`yas_core` 捕获/OCR trait、游戏窗口发现、定位工具、取消机制和双语日志。当前原神控制器/背包扫描器硬编码了原神窗口与世界状态检查、面板几何、8×5 网格及滚动行为，因此本切片未复用它。

## Maintained public implementations / 维护中的公开实现

The following revisions were inspected directly rather than inferred from project names or release age.

以下修订均经过直接检查，而不是根据项目名称或发布时间推测。

| Project / 项目 | Mechanism and exact revision / 机制与精确修订 | Finding / 结论 |
|---|---|---|
| [Kel-Z HSR-Scanner](https://github.com/kel-z/HSR-Scanner/tree/cad008e2509b6b086fd892b140c8b277424fcb03) | OCR plus simulated mouse/keyboard/virtual-controller input; HEAD [`cad008e2509b6b086fd892b140c8b277424fcb03`](https://github.com/kel-z/HSR-Scanner/commit/cad008e2509b6b086fd892b140c8b277424fcb03), 2026-07-21; release `v1.5.0`. / OCR 加模拟鼠标、键盘、虚拟控制器输入；HEAD 为上述 2026-07-21 修订，发布版 `v1.5.0`。 | Maintained full-inventory scanner for Characters, Light Cones, and all six gear slots, but requires English, 16:9, screenshots, and many normalized fixed positions. Hidden Speed decimals are not observable. / 维护中的全背包扫描器，覆盖角色、光锥和六个装备槽位，但要求英文、16:9、截图和大量归一化固定坐标，且无法观察隐藏速度小数。 |
| [Kel-Z HSR-Data](https://github.com/kel-z/HSR-Data/tree/ac490325782c5e6aeb345a04e4d3f59064302b7d) | Scanner-pinned `v6` branch at [`ac490325782c5e6aeb345a04e4d3f59064302b7d`](https://github.com/kel-z/HSR-Data/commit/ac490325782c5e6aeb345a04e4d3f59064302b7d), 2026-07-21. / 扫描器固定使用 `v6` 分支及上述 2026-07-21 修订。 | Generated reference declares HSR 4.4 while packet tooling below is already 4.5. It supports a provider-boundary design but must not be treated as current without revision validation. / 生成参考数据声明 HSR 4.4，而下方抓包工具已到 4.5；它证明提供者边界可行，但必须校验修订，不能无条件视为最新。 |
| [Reliquary Archiver](https://github.com/IceDynamix/reliquary-archiver/tree/cb109f17a4a15b7604cfe9d078a8735e7735cd25) | Passive packet capture/export; HEAD [`cb109f17a4a15b7604cfe9d078a8735e7735cd25`](https://github.com/IceDynamix/reliquary-archiver/commit/cb109f17a4a15b7604cfe9d078a8735e7735cd25), 2026-08-29; release `v0.18.0`. / 被动抓包与导出；HEAD 为上述 2026-08-29 修订，发布版 `v0.18.0`。 | Reads bag, roster, Light Cones, Relics, materials, and live deltas. It dispatches generated patch-specific command IDs and protobuf types. / 读取背包、角色、光锥、遗器、材料和实时增量；按版本生成的命令 ID 与 protobuf 类型分派。 |
| [Reliquary](https://github.com/IceDynamix/reliquary/tree/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b) | HSR network/protobuf library [`d5cf3b7e7e66470d2d8efff6676aa18762b21d3b`](https://github.com/IceDynamix/reliquary/commit/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b), `v23.0.0`, 2026-08-28, “Update protos for 4.5.” / HSR 网络/protobuf 库，上述修订为 `v23.0.0`，2026-08-28 “Update protos for 4.5”。 | Exact-value source and strongest protocol evidence, but its major version changes with every HSR update. / 精确数值来源及最强协议证据，但其主版本随每次 HSR 更新而变化。 |
| [Fribbels HSR Optimizer guide](https://github.com/fribbels/hsr-optimizer/blob/e694f0ecc6508d45eb753703908670e84994cb43/docs/guides/en/optimizer.md#step-1---import-relics) | Revision `e694f0ecc6508d45eb753703908670e84994cb43`, 2026-08-28. / 修订 `e694f0ecc6508d45eb753703908670e84994cb43`，2026-08-28。 | Independently describes Reliquary as instant/exact hidden-Speed import and Kel-Z as a 5–10 minute OCR import without hidden decimals. / 独立说明 Reliquary 可即时导入隐藏速度精确值，而 Kel-Z OCR 约需 5–10 分钟且缺少隐藏小数。 |

No maintained project found in this audit implements an account-safe write/apply workflow. Reliquary Archiver observes traffic; it does not inject state-changing packets.

本次审计未发现维护中的项目实现账号安全的写入/应用工作流。Reliquary Archiver 只观察流量，不注入改变状态的数据包。

## Capture protocol evidence / 捕获协议证据

The current Reliquary decoder at `d5cf3b7` implements this read path:

当前 Reliquary 解码器 `d5cf3b7` 实现以下只读路径：

```text
Ethernet or raw IP / 以太网或原始 IP
  -> IPv4 or IPv6 UDP, ports 23301-23302 / IPv4 或 IPv6 UDP，端口 23301-23302
  -> custom connection envelope / 自定义连接封装
  -> KCP segment and reassembly / KCP 分段与重组
  -> XOR-encrypted game command / XOR 加密游戏命令
  -> generated protobuf / 生成的 protobuf
```

Capture must start before login because `PlayerGetTokenScRsp` provides `secret_key_seed`, which derives the session key. The command frame has a fixed framing structure around a command ID, header length, data length, and protobuf body, but the command IDs and protobuf field names are patch-generated. See pinned [`network/mod.rs`](https://github.com/IceDynamix/reliquary/blob/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b/src/network/mod.rs) and [`command/mod.rs`](https://github.com/IceDynamix/reliquary/blob/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b/src/network/command/mod.rs).

捕获必须在登录前开始，因为 `PlayerGetTokenScRsp` 提供用于派生会话密钥的 `secret_key_seed`。命令帧在命令 ID、头长度、数据长度和 protobuf 主体外有固定封装，但命令 ID 与 protobuf 字段名由版本生成。固定链接见上述 `network/mod.rs` 与 `command/mod.rs`。

For HSR 4.5 only, generated IDs include `PlayerLoginScRsp=13`, `PlayerGetTokenScRsp=19`, `GetAvatarDataScRsp=313`, `GetBagScRsp=513`, and `PlayerSyncScNotify=632`. These numbers are revision evidence, not GOODScanner constants. A future parser must identify the negotiated game version, pin compatible protobuf/key material, validate semantic packet shape, and fail closed on an unknown version instead of guessing IDs.

仅对 HSR 4.5，生成的 ID 包括 `PlayerLoginScRsp=13`、`PlayerGetTokenScRsp=19`、`GetAvatarDataScRsp=313`、`GetBagScRsp=513` 和 `PlayerSyncScNotify=632`。这些数字只是修订证据，不是 GOODScanner 常量。未来解析器必须识别协商后的游戏版本，固定兼容的 protobuf/密钥材料，验证数据包语义形状，并在未知版本上关闭失败，而不是猜测 ID。

This differs fundamentally from OCR/controller capture: OCR obtains screenshots and changes only UI selection/navigation, then recognizes visible strings and pixels. It has no access to hidden stat precision and should discover selected panels, tab labels/icons, grid bounds, row/column spacing, and scroll transitions from the current frame before calculating click targets. Kel-Z and legacy YAS coordinates are calibration evidence, not a durable contract.

这与 OCR/控制器捕获有根本区别：OCR 获取截图，只改变界面选择/导航，然后识别可见文字和像素。它无法读取隐藏属性精度，应先从当前画面发现选中面板、页签文字/图标、网格边界、行列间距和滚动转换，再计算点击目标。Kel-Z 与旧 YAS 坐标只能作为校准证据，不能作为持久契约。

## Community format evidence / 社区格式证据

The maintained HSR community import is “loosely based” on GOOD rather than being GOOD v3. Fribbels accepts Kel-Z/Reliquary version 4 and legacy YAS version 3. Version 4 has top-level `characters`, `light_cones`, and one `relics` array; Planar Sphere and Link Rope are relic slots rather than a separate top-level collection. See the pinned [import configuration](https://github.com/fribbels/hsr-optimizer/blob/e694f0ecc6508d45eb753703908670e84994cb43/src/lib/importer/importConfig.ts#L13-L54) and [Kel-Z parser types](https://github.com/fribbels/hsr-optimizer/blob/e694f0ecc6508d45eb753703908670e84994cb43/src/lib/importer/kelzFormatParser.tsx#L33-L120).

维护中的 HSR 社区导入格式只是“宽松基于”GOOD，而不是 GOOD v3。Fribbels 接受 Kel-Z/Reliquary v4 和旧 YAS v3。v4 顶层包含 `characters`、`light_cones` 和单一 `relics` 数组；位面球与连结绳是遗器槽位，而不是独立顶层集合。固定证据见上述导入配置与 Kel-Z 解析类型。

This milestone intentionally uses `goodscanner.hsr.experimental` schema version 1. It separates `relics` and `planarOrnaments` internally to prove all requested categories and to avoid losing semantic kind. A future Fribbels v4 adapter must merge both collections into `relics` and map slots explicitly; the current output must not be presented as community-compatible.

本里程碑有意使用 `goodscanner.hsr.experimental` 架构 v1。内部将 `relics` 与 `planarOrnaments` 分开，以验证全部请求类别并避免丢失语义类型。未来 Fribbels v4 适配器必须将两者合并到 `relics` 并显式映射槽位；当前输出不得宣称社区兼容。

The v4 parser also declares account-level `metadata.uid` and per-item `_uid`. That conflicts with this milestone's privacy boundary. Any future adapter must omit/redact account UID, generate deterministic scan-local item IDs inside the pipeline, and prove by tests that no Reliquary server `unique_id` survives. A consumer that requires real account/server identifiers is intentionally unsupported.

v4 解析器还声明账号级 `metadata.uid` 与逐物品 `_uid`，这与本里程碑隐私边界冲突。任何未来适配器都必须省略/遮盖账号 UID，在管线内部生成确定性的扫描本地物品 ID，并通过测试证明 Reliquary 服务器 `unique_id` 不会保留。需要真实账号/服务器标识的消费者将被有意判定为不支持。

## Working code / 已实现代码

- `experimental/hsr` is its own Cargo workspace and package. The root workspace explicitly excludes it.
- `experimental/hsr` 是独立 Cargo 工作区与软件包，根工作区显式排除它。
- `HSRScannerExperimental` requires the non-default `experimental-hsr` feature. Official application manifests, GUI routing, updater, server, and binary definitions contain no HSR target or feature.
- `HSRScannerExperimental` 需要非默认 `experimental-hsr` 功能。正式应用清单、GUI 路由、更新器、服务器及二进制定义均不包含 HSR 目标或功能。
- `ReferenceProvider` loads a versioned normalized `ReferenceSnapshot`; `ReferenceCache` validates schema version, duplicate IDs and stable keys, bilingual names, and gear category/slot agreement.
- `ReferenceProvider` 加载版本化规范 `ReferenceSnapshot`；`ReferenceCache` 校验架构版本、重复 ID 与稳定键、双语名称及装备类别/槽位一致性。
- `FixtureObservationSource` rejects account/session identity keys before typed deserialization and only accepts `sanitizedFixture` evidence.
- `FixtureObservationSource` 在类型反序列化前拒绝账号/会话身份字段，并且只接受 `sanitizedFixture` 证据。
- The conversion pipeline resolves semantic references, sorts deterministically, separates cavern Relics from Planar Ornaments by provider-owned slot/category semantics, and preserves unknown lock/discard state as JSON `null` instead of inventing `false`.
- 转换管线解析语义参考、确定性排序、依据提供者拥有的槽位/类别语义区分隧洞遗器与位面饰品，并将未知锁定/弃置状态保留为 JSON `null`，不会虚构为 `false`。
- Output privacy metadata explicitly states that account identifiers and raw packet data are absent. Run-local IDs are generated inside the validated pipeline from category/index, and are never caller strings or hashes of server GUIDs. Equipped locations are resolved from public character template IDs to provider-owned canonical keys.
- 输出隐私元数据明确声明不包含账号标识与原始数据包。运行内本地 ID 由已验证管线根据类别/索引生成，绝不是调用者字符串或服务器 GUID 的哈希；装备位置从公开角色模板 ID 解析为提供者规范键。

## Fixture proof / 夹具证明

All committed fixtures are synthetic semantic documents. They contain no screenshot, PCAP, ETL, raw protobuf, game UID, server GUID/unique ID, token, cookie, session key, account name, or player name.

所有提交夹具均为合成语义文档，不包含截图、PCAP、ETL、原始 protobuf、游戏 UID、服务器 GUID/唯一 ID、令牌、Cookie、会话密钥、账号名或玩家名。

The golden fixture proves exactly one Character, one Light Cone, one cavern Relic, and one Planar Ornament. Tests additionally prove golden JSON equality, provider validation, sensitive-field rejection, bilingual readable errors plus full technical details, preservation of unknown status, and official-release isolation.

黄金夹具精确验证一个角色、一个光锥、一件隧洞遗器和一件位面饰品。测试还验证黄金 JSON 完全相等、提供者校验、敏感字段拒绝、双语可读错误与完整技术详情、未知状态保留以及正式发布隔离。

## English and Chinese UX / 中英文用户体验

The executable follows the repository localization convention: `yas::lang::set_lang` selects runtime language and all logs use compile-time bilingual `yas::log_*` macros. `--lang en` and `--lang zh` select English or Chinese. CLI descriptions, help, privacy warning, readable error hints, technical-detail labels, success counts, README, and this report support both languages. Exported reference names retain both `zhCn` and `en`.

可执行文件遵循仓库本地化约定：`yas::lang::set_lang` 选择运行时语言，所有日志使用编译期双语 `yas::log_*` 宏。`--lang en` 与 `--lang zh` 分别选择英文或中文。CLI 描述、帮助、隐私警告、可读错误提示、技术详情标签、成功计数、README 和本报告均支持双语；导出参考名称同时保留 `zhCn` 与 `en`。

## Live-device gaps / 实机缺口

The following claims are not yet proven and are not implemented:

以下事项尚未得到证明，也未实现：

- Current HSR window discovery, localization variants, HDR behavior, resolution/aspect-ratio support, inventory tab anchors, selected-item indicators, adaptive grid geometry, scroll-end detection, panel OCR regions, or OCR accuracy.
- 当前 HSR 窗口发现、本地化变体、HDR 行为、分辨率/宽高比支持、背包页签锚点、选中物品指示、自适应网格几何、滚动结束检测、面板 OCR 区域或 OCR 准确率。
- Live KCP reassembly, HSR version negotiation, XOR key acquisition, protobuf shape discovery, loss/reordering behavior, official/Bilibili channel behavior, packet-filter permissions, or safe cancellation.
- 实时 KCP 重组、HSR 版本协商、XOR 密钥获取、protobuf 形状发现、丢包/乱序行为、官服/B 服行为、抓包过滤权限或安全取消。
- Exact community v4 adapter behavior for current character skills/traces, memosprite fields, preview/reroll substats, and hidden precision.
- 当前角色技能/行迹、忆灵字段、预览/重掷副词条及隐藏精度的社区 v4 适配行为。
- Any live lock/unlock/discard-mark action or post-action verification.
- 任何实时锁定/解锁/弃置标记动作或动作后验证。

No live game was opened or controlled, and no real traffic was captured for this milestone.

本里程碑没有打开或控制实时游戏，也没有捕获真实流量。

## Upstream and data blockers / 上游与数据阻塞项

The audited local GIlore revision `25c7b8f3398fbffac84d035376b3c5275717a922` currently provides a Genshin-oriented datamine pipeline and no normalized HSR snapshot matching this provider contract. The boundary is implemented, but a GIlore HSR ingestion source, bilingual naming policy, stable keys, schema versioning, revision provenance, and generated validation fixtures remain future work.

本次审计的本地 GIlore 修订 `25c7b8f3398fbffac84d035376b3c5275717a922` 当前提供面向原神的数据挖掘管线，没有与本提供者契约匹配的规范化 HSR 快照。边界已经实现，但 GIlore HSR 摄取源、双语命名策略、稳定键、架构版本、修订来源和生成式校验夹具仍属后续工作。

Kel-Z HSR-Data was one patch behind the audited Reliquary revision (4.4 versus 4.5), so it cannot be silently adopted as a fallback. Reliquary changes its major version per HSR patch and includes obfuscated protobuf fields, so packet support requires an explicit supported-version matrix and fail-closed update process.

在本次审计中，Kel-Z HSR-Data 比 Reliquary 落后一个版本（4.4 对 4.5），因此不能静默作为后备。Reliquary 每个 HSR 版本都会更改主版本，并含有混淆 protobuf 字段，因此抓包支持需要明确的支持版本矩阵和关闭失败的更新流程。

## Future apply-status boundary / 未来状态应用边界

Read evidence confirms these HSR states in current protocol/UI exports: current [`Relic`](https://github.com/IceDynamix/reliquary/blob/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b/src/network/command/proto/Relic.rs#L27-L57) protobuf data exposes protected/lock and discard-mark state, while the pinned [Fribbels converters](https://github.com/IceDynamix/reliquary-archiver/blob/cb109f17a4a15b7604cfe9d078a8735e7735cd25/src/export/fribbels/converters.rs#L34-L90) map protected state for Relics and Light Cones. There is no evidence for a Light Cone discard state. “Discard” here means a reversible mark, not salvage, deletion, consumption, or destruction.

当前协议/界面导出的读取证据确认：当前 [`Relic`](https://github.com/IceDynamix/reliquary/blob/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b/src/network/command/proto/Relic.rs#L27-L57) protobuf 数据暴露保护/锁定与弃置标记状态，固定修订的 [Fribbels 转换器](https://github.com/IceDynamix/reliquary-archiver/blob/cb109f17a4a15b7604cfe9d078a8735e7735cd25/src/export/fribbels/converters.rs#L34-L90) 映射遗器和光锥的保护状态。没有证据表明光锥存在弃置状态。此处“弃置”是可逆标记，不是分解、删除、消耗或销毁。

| Candidate action / 候选动作 | Protocol evidence at Reliquary `d5cf3b7` / Reliquary `d5cf3b7` 协议证据 | Milestone status / 里程碑状态 |
|---|---|---|
| Relic lock/unlock / 遗器锁定/解锁 | [`LockRelicCsReq`](https://github.com/IceDynamix/reliquary/blob/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b/src/network/command/proto/LockRelicCsReq.rs) contains desired `is_locked` and relic IDs; generated 4.5 ID is `540`. / 包含目标 `is_locked` 与遗器 ID；4.5 生成 ID 为 `540`。 | Read state only; no action code. / 仅读取状态，无动作代码。 |
| Light Cone lock/unlock / 光锥锁定/解锁 | [`LockEquipmentCsReq`](https://github.com/IceDynamix/reliquary/blob/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b/src/network/command/proto/LockEquipmentCsReq.rs) contains desired `is_locked` and an obfuscated ID vector; generated 4.5 ID is `544`. / 包含目标 `is_locked` 与混淆 ID 向量；4.5 生成 ID 为 `544`。 | Read state only; obfuscated request field unresolved. / 仅读取状态；混淆请求字段未解析。 |
| Relic discard-mark set/clear / 设置/清除遗器弃置标记 | [`DiscardRelicCsReq`](https://github.com/IceDynamix/reliquary/blob/d5cf3b7e7e66470d2d8efff6676aa18762b21d3b/src/network/command/proto/DiscardRelicCsReq.rs) contains IDs, a boolean, and additional obfuscated fields; generated 4.5 ID is `574`. / 包含 ID、布尔值及额外混淆字段；4.5 生成 ID 为 `574`。 | Direction semantics unproven; requires redacted request/response/UI correlation. / 方向语义未证明；需要脱敏请求/响应/UI 对照。 |

Packet injection is not authorized for a future apply path. The evidence-backed direction is attended UI automation only after independent live-device proof. Any future design must satisfy every gate below before it can be enabled:

未来状态应用路径不授权数据包注入。基于证据的方向只能是在独立实机验证后进行有人值守的 UI 自动化。任何未来设计都必须满足以下全部门槛后才能启用：

1. Read a fresh complete inventory and compute an exact desired-state diff. Items already at target state are no-ops.
2. 读取最新完整背包并计算精确目标状态差异；已处于目标状态的物品为空操作。
3. Require explicit per-run, per-device, per-action user authorization. Scan/export permission never implies mutation permission; lock, unlock, mark, and unmark are separate scopes.
4. 要求用户按每次运行、每台设备、每种动作显式授权。扫描/导出权限绝不蕴含变更权限；锁定、解锁、标记和取消标记是不同权限范围。
5. Verify the expected HSR process/window is foreground, input is idle, semantic anchors are present, the correct inventory tab is selected, and an emergency abort is active.
6. 验证预期 HSR 进程/窗口位于前台、输入空闲、语义锚点存在、选择了正确背包页签，并启用紧急中止。
7. Identify each item from immutable visible fields plus before-state. Never use an OCR scan-local `_uid` as a server mutation ID and never retain a server GUID in normal data.
8. 使用不可变可见字段和动作前状态识别每件物品。绝不把 OCR 扫描本地 `_uid` 当作服务器变更 ID，也绝不在普通数据中保留服务器 GUID。
9. Apply one reversible canary action, then capture the same panel and verify the exact target icon/state before any bounded batch.
10. 先应用一个可逆金丝雀动作，再捕获同一面板并验证精确目标图标/状态，之后才允许有界批处理。
11. Retry only when a fresh read proves the before-state is unchanged. An ambiguous timeout must stop; it must never trigger a blind toggle or automatic rollback.
12. 只有最新读取证明动作前状态未变时才可重试。模糊超时必须停止，绝不能触发盲目切换或自动回滚。
13. Verify persistence after closing/reopening the inventory and, during enablement testing, after client restart. Record redacted before/after device evidence and per-item outcome.
14. 关闭并重新打开背包后验证持久性；启用测试阶段还需在客户端重启后验证。记录脱敏的动作前后设备证据和逐物品结果。
15. Roll back only on explicit user instruction after a confirmed state read. Salvage, delete, consume, enhance, equip/unequip, and packet writes remain out of scope.
16. 只有在确认状态读取后且用户明确指示时才回滚。分解、删除、消耗、强化、装备/卸下及数据包写入继续排除在范围外。

Required device evidence before enablement includes bilingual UI captures for every supported resolution/localization, selected and unselected icon states, lock/unlock and discard-mark transitions, focus loss, scroll/page boundaries, ambiguous animation timing, inventory reopen persistence, restart persistence, and abort behavior. Captures must crop/redact the on-screen UID and unrelated personal UI.

启用前所需设备证据包括：每个支持分辨率/语言的双语 UI 截图、选中与未选中图标状态、锁定/解锁及弃置标记转换、失焦、滚动/分页边界、模糊动画时序、背包重开持久性、重启持久性和中止行为。截图必须裁剪/遮盖屏幕 UID 与无关个人界面。

## Sensitive-data rules / 敏感数据规则

- Never request or persist an account password, API credential, login token, cookie, session key, UID, player name, server GUID, or server unique item ID in normal logs/exports.
- 绝不在普通日志/导出中请求或保存账号密码、API 凭据、登录令牌、Cookie、会话密钥、UID、玩家名、服务器 GUID 或服务器唯一物品 ID。
- Never commit raw PCAP/ETL, protobuf payloads, or full screenshots as fixtures. Use synthetic semantic fixtures or narrowly cropped/redacted UI evidence.
- 绝不提交原始 PCAP/ETL、protobuf 载荷或完整截图作为夹具；只使用合成语义夹具或严格裁剪/脱敏的 UI 证据。
- Never copy Reliquary trace behavior that logs base64 payloads or the session-key seed. Capture logs must be metadata-only and redacted by construction.
- 绝不复制 Reliquary 记录 base64 载荷或会话密钥种子的 trace 行为；捕获日志必须从设计上只含元数据且完成脱敏。
- Any future local stream endpoint must bind loopback only, require an ephemeral capability, reject cross-origin/untrusted clients, and omit identity by default.
- 任何未来本地流端点必须只绑定回环地址、要求临时能力令牌、拒绝跨源/不受信客户端，并默认省略身份信息。

## Next steps / 后续步骤

1. Add a GIlore-owned HSR reference ingestion package that emits the normalized bilingual schema with revision provenance and patch compatibility metadata.
2. 增加由 GIlore 管理的 HSR 参考摄取包，输出带修订来源与版本兼容元数据的规范化双语架构。
3. Add a privacy-filtered Fribbels/Kel-Z v4 adapter and fixture corpus, explicitly merging Planar Ornaments into the community `relics` array while retaining internal category semantics; omit account UID and use pipeline-generated scan-local `_uid` values only.
4. 增加经过隐私过滤的 Fribbels/Kel-Z v4 适配器与夹具语料，显式将位面饰品合并到社区 `relics` 数组，同时保留内部类别语义；省略账号 UID，只使用管线生成的扫描本地 `_uid`。
5. Build a screenshot-only discovery harness that records sanitized crops and measures semantic tab/panel/grid detection without issuing input.
6. 构建纯截图发现工具，记录脱敏裁剪，并在不发送输入的情况下测量语义页签/面板/网格检测。
7. If packet work is separately authorized, first parse sanitized decoded semantic/protobuf fixtures behind a distinct experimental feature, pinned to an explicit HSR version; do not start with live capture.
8. 如果之后单独授权抓包工作，先在独立实验功能后解析脱敏的已解码语义/protobuf 夹具，并固定到明确 HSR 版本；不要从实时捕获开始。
9. Collect independent redacted live-device evidence before proposing any controller navigation or apply-status implementation.
10. 在提出任何控制器导航或状态应用实现前，收集独立脱敏实机证据。

## Reproduction and validation / 复现与验证

Repository history / 仓库历史：

```powershell
git log --all --regexp-ignore-case --grep='honkai|star rail|hsr'
git show --stat 49a7deaab39790394bf5f5f467a6b4b8686d3abc
git show 757689f062d0f3c2dc0bd48e16544f5f6e8e78d1:yas-starrail/src/export/relic/hsr.rs
git grep -n -i -E 'packet|protobuf|sniff|udp|pktmon|command.?id|dispatch' 757689f062d0f3c2dc0bd48e16544f5f6e8e78d1 -- yas-starrail
git diff --quiet 730d8845505c04c7f4c91709de076503401ff40c 757689f062d0f3c2dc0bd48e16544f5f6e8e78d1 -- yas-starrail
```

Experimental code / 实验代码：

```powershell
cargo test --manifest-path experimental/hsr/Cargo.toml --all-features
cargo run --manifest-path experimental/hsr/Cargo.toml --features experimental-hsr --bin HSRScannerExperimental -- --lang en --input experimental/hsr/tests/fixtures/observations.json --references experimental/hsr/tests/fixtures/reference_cache.json --output hsr-experimental-en.json
cargo run --manifest-path experimental/hsr/Cargo.toml --features experimental-hsr --bin HSRScannerExperimental -- --lang zh --input experimental/hsr/tests/fixtures/observations.json --references experimental/hsr/tests/fixtures/reference_cache.json --output hsr-experimental-zh.json
```

The validation outputs and any temporary public-source clones are local audit artifacts only and are not release inputs.

验证输出与任何临时公开源码克隆仅为本地审计产物，不属于发布输入。
