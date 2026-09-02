# HSRScannerExperimental operator runbook / HSRScannerExperimental 操作手册

This Windows-only experimental executable exports Honkai: Star Rail Characters, Light Cones, Cavern Relics, and Planar Ornaments, and provides an attended manager for reversible lock/discard-mark changes. It is an isolated package: it is excluded from the root workspace, has no default feature, and is never included in official `GOODScanner` or `GOODCapture` releases.

此 Windows 实验程序可导出《崩坏：星穹铁道》的角色、光锥、隧洞遗器和位面饰品，并提供有人值守的管理器，只处理可逆的锁定和弃置标记变更。它是隔离软件包：不属于根工作区、没有默认功能，也绝不会进入正式版 `GOODScanner` 或 `GOODCapture`。

## Build and verify / 构建与验证

Run with Rust 1.89 or newer from the GOODScanner repository root. The release executable is written to `experimental\hsr\target\release\HSRScannerExperimental.exe` and carries its own experimental identity and administrator manifest.

请使用 Rust 1.89 或更高版本，在 GOODScanner 仓库根目录运行。发布版程序位于 `experimental\hsr\target\release\HSRScannerExperimental.exe`，并使用独立的实验程序标识和管理员清单。

```powershell
cargo build --locked --release --manifest-path .\experimental\hsr\Cargo.toml --features experimental-hsr --bin HSRScannerExperimental
$exe = ".\experimental\hsr\target\release\HSRScannerExperimental.exe"
& $exe --lang en --help
```

One reproducible production-CLI fixture export uses the committed sanitized observations and GIlore reference bundle. `hsr-fixture-v2.json` must not already exist.

以下命令使用仓库内已提交的脱敏观测和 GIlore 参考数据包，通过正式 CLI 路径生成可复现夹具。运行前 `hsr-fixture-v2.json` 必须不存在。

```powershell
& $exe --lang en fixture-export `
  --reference-bundle .\experimental\hsr\tests\fixtures\gilore_bundle `
  --input .\experimental\hsr\tests\fixtures\observations.json `
  --output .\hsr-fixture-v2.json

cargo test --locked --manifest-path .\experimental\hsr\Cargo.toml --all-features --test cli_contract fixture_export_runs_through_the_production_cli_and_never_overwrites -- --exact
```

That safe run truthfully emits `source.kind: "sanitizedFixture"`. The scanner-owned cross-repo golden in `tests\fixtures\expected_export.json` represents `screenCapture`; a regression proves that its semantic payload differs from the sanitized run only in the explicit source kind and source revision. GGStarRail consumes the screen golden byte-for-byte.

上述安全路径会如实输出 `source.kind: "sanitizedFixture"`。扫描器拥有的跨仓库黄金文件 `tests\fixtures\expected_export.json` 表示 `screenCapture`；回归测试证明它与脱敏输出的语义载荷只有显式来源类型和来源修订不同。GGStarRail 会逐字节读取该截图黄金文件。

Every export command uses create-new output semantics: it fails instead of overwriting an existing file.

所有导出命令都只允许新建输出文件：目标已存在时会失败，不会覆盖。

The committed `tests\fixtures\gilore_bundle` is intentionally tiny and is only for tests. For real inventory operations, point every `--reference-bundle` below at a current, complete GGStarRail-generated `hsr-reference` directory (containing its verified `manifest.json`), for example:

仓库内的 `tests\fixtures\gilore_bundle` 刻意保持为极小测试数据，不能用于真实库存。执行下列实机操作时，请让每个 `--reference-bundle` 指向 GGStarRail 当前生成的完整 `hsr-reference` 目录（其中应包含已校验的 `manifest.json`），例如：

```powershell
$referenceBundle = "F:\Codes\genshin\GGStarRail\src\generated\hsr-reference"
```

Live screenshot scanning, live helper capture, and both manager commands reject a merely self-consistent tiny catalog. They require production-sized Character, Light Cone, Relic, stat, slot, and main-affix progression coverage before any game device is created or helper process is launched. The tiny committed bundle remains supported by the deterministic fixture-export and offline import paths.

实时截图扫描、实时辅助抓包和两条管理器命令都会拒绝仅在内部自洽的极小目录。程序会先要求角色、光锥、遗器、属性、部位及主属性成长表达到完整的生产规模，再创建设备或启动辅助进程。仓库内的极小数据包仍可用于确定性夹具导出和离线导入路径。

## Live screenshot scan / 实时截图扫描

Open the HSR client, leave it visible, and keep it in the foreground. The following explicit selection scans Characters, Light Cones, and gear; `--relics` includes both Cavern Relics and Planar Ornaments. Enter the exact number of Characters owned by this account as one required completion check. A matching count is not sufficient when any identity remains visually ambiguous; in that case coverage stays `unknown`.

打开 HSR 客户端，保持窗口可见并置于前台。以下命令明确扫描角色、光锥和装备；`--relics` 同时包括隧洞遗器和位面饰品。请输入该账号实际拥有的角色总数，作为完整性判断的一项必要检查。如果仍有角色身份无法仅凭画面消歧，即使数量相符，覆盖范围也会保持为 `unknown`。

```powershell
$expectedCharacters = Read-Host "Exact number of Characters owned"
& $exe --lang en scan `
  --reference-bundle $referenceBundle `
  --characters --light-cones --relics `
  --expected-characters $expectedCharacters `
  --output .\hsr-live-v2.json
```

With no category flags, `scan` defaults to all categories. Without `--expected-characters`, Character coverage remains `unknown` rather than being overstated. Even with the expected count, completeness also requires one more stable next-panel transition to wrap to the first Character. Trailblazer and any other identity that the visible name/path cannot distinguish are omitted with a warning and also keep coverage `unknown`. The default next-character key is `E`; use `--next-character-key` if the live client uses another binding.

未指定分类参数时，`scan` 默认扫描全部分类。缺少 `--expected-characters` 时，角色覆盖保持为 `unknown`，而不会被夸大为完整。即使数量一致，也必须再进行一次稳定的下一角色切换并回绕到首名角色，才能证明完整。开拓者及其他无法通过画面中的名称/命途消歧的角色会被省略并产生警告，同时使覆盖范围保持为 `unknown`。默认切换下一个角色的按键是 `E`；实机绑定不同则使用 `--next-character-key`。

Light Cone and Relic coverage becomes `complete` only when three inventory-quantity reads agree, the next cell is absent, two attended wheel events both show no advance, and the visible scrollbar thumb independently remains at the bottom. Quantity disagreement, observed advance, uncertain motion, or missing scrollbar-bottom proof keeps coverage `unknown`; manager commands then refuse mutation.

光锥和遗器只有在三次数量读数一致、不存在下一格、两次有人值守的滚轮输入均未继续前进，且可见滚动条滑块独立证明仍位于底端时，才会标记为 `complete`。数量不一致、观察到继续滚动、滚动状态不确定或缺少底端滑块证据都会使覆盖状态保持 `unknown`；管理器随后会拒绝变更。

## Read-only capture / 只读抓包

Supply your own Reliquary Archiver **v0.18.0** executable. This project does not download or bundle it. Independently approve the SHA-256 for that exact file before passing it to the scanner; a mismatch fails before the helper runs. The scanner copies the approved bytes into a private temporary directory, re-hashes that staged copy, and executes only the staged path so replacement of the original path cannot change the process image selected for launch.

请自行提供 Reliquary Archiver **v0.18.0** 可执行文件；本项目不会下载或捆绑它。运行前请独立核准该文件的 SHA-256，并将同一摘要交给扫描器；摘要不匹配时，辅助程序不会启动。扫描器会把已核准的字节复制到私有临时目录，再次计算该副本的哈希，并且只执行这个暂存路径，因此原始路径随后被替换也不会改变所选的启动映像。

1. Start HSR and stop at **Click to Start**; do not enter the world yet.
2. Run the command below. After the helper is waiting, click **Click to Start** and enter the world.
3. Keep HSR and the terminal open until the export completes or the timeout is reported.

1. 启动 HSR 并停在**点击开始**界面，暂时不要进入游戏世界。
2. 运行以下命令。辅助程序开始等待后，点击**点击开始**并进入游戏世界。
3. 保持 HSR 和终端开启，直到导出完成或程序报告超时。

```powershell
$archiver = "C:\Tools\ReliquaryArchiver-v0.18.0.exe"
$approvedArchiverSha256 = Read-Host "Paste the independently approved SHA-256"
& $exe --lang en capture `
  --reference-bundle $referenceBundle `
  --archiver $archiver `
  --archiver-sha256 $approvedArchiverSha256 `
  --timeout-seconds 120 `
  --output .\hsr-capture-v2.json
```

To normalize an already existing Reliquary/Fribbels v4 JSON without launching the helper:

如需离线转换已有的 Reliquary/Fribbels v4 JSON，而不启动辅助程序：

```powershell
& $exe --lang en import-capture `
  --reference-bundle $referenceBundle `
  --input C:\path\to\reliquary-archive.json `
  --output .\hsr-imported-v2.json
```

Offline imports preserve `unknown` coverage for omitted upstream collections. A live helper run attempts explicit removal of its private staged executable and output on every return. If removal cannot be proven, the command fails with `HSR-CAPTURE-CLEANUP` and reports the private path for attended cleanup instead of claiming success. Account UIDs and server-item identifiers are discarded before normalization; embedded raw-packet data is rejected. None of these fields is exported.

离线导入会将上游缺失分类的覆盖状态保留为 `unknown`。实时辅助抓包会在每条返回路径上显式尝试删除私有的暂存程序与输出；如果无法证明删除成功，命令会以 `HSR-CAPTURE-CLEANUP` 失败并报告私有路径，供有人值守清理，而不会声称成功。账号 UID 和服务器物品标识会在规范化前丢弃；嵌入的原始数据包内容会被拒绝。上述字段均不会进入导出文件。

## Manager: preview, then separately apply / 管理器：先预览，再单独应用

`manager preview` performs a fresh complete gear scan and prints the exact matchers, observed states, proposed changes, required scopes, inventory fingerprint, and full confirmation digest. It never changes the game.

`manager preview` 会重新完整扫描装备，并打印精确匹配字段、当前状态、计划变更、所需授权范围、库存指纹和完整确认摘要。此命令绝不会修改游戏。

```powershell
$instructions = "C:\path\to\ggstarrail-manager-instructions.json"
& $exe --lang en manager preview `
  --reference-bundle $referenceBundle `
  --instructions $instructions
```

Review every entry. Copy the complete `Confirmation digest` exactly. Then run a separate `manager apply` with the same instructions and reference bundle. For a new journal, `apply` rescans and rebuilds the plan; changed inventory or a different plan invalidates the digest. For a restart with that journal, it recovers the original persisted plan before scanning and reprints that exact plan before any possible mutation. It also requires `--confirm-live-mutation` and each scope actually shown by the preview. This lock-only example deliberately grants only lock permission:

逐项检查预览，并完整复制 `Confirmation digest`。随后用相同指令文件和参考数据包单独运行 `manager apply`。使用新日志时，`apply` 会重新扫描并重建计划；库存或计划发生变化时，摘要将失效。使用同一日志恢复中断任务时，它会在扫描前恢复原先持久化的计划，并在任何可能的变更前重新打印完全相同的计划。还必须提供 `--confirm-live-mutation`，并逐项授权预览中实际出现的范围。以下锁定示例只授予锁定权限：

```powershell
$digest = Read-Host "Paste the exact Confirmation digest"
& $exe --lang en manager apply `
  --reference-bundle $referenceBundle `
  --instructions $instructions `
  --confirm-digest $digest `
  --journal C:\path\to\durable\hsr-manager-journal.jsonl `
  --confirm-live-mutation `
  --allow-lock
```

The least-privilege scope flags are `--allow-lock`, `--allow-unlock`, `--allow-mark-discard`, and `--allow-unmark-discard`. Grant only those required by the reviewed preview. A locked Relic can never be marked for discard in the same plan: it is preview-only, and unlocking then marking discard require two separately scanned, reviewed, and digest-confirmed runs. The journal is durable and append-only; retain it and reuse the same path when recovering the same interrupted plan. A per-user operating-system controller lease prevents `scan`, `manager preview`, and `manager apply` from driving the HSR window concurrently, even when apply commands use different journal paths. Apply also holds a journal-specific lease through reconciliation, input, and durable post-verification.

最小权限范围参数为 `--allow-lock`、`--allow-unlock`、`--allow-mark-discard` 和 `--allow-unmark-discard`。只授权已审核预览所需的范围。已锁定遗器绝不会在同一计划中被标记弃置：它只能预览；解锁后标记弃置必须分别重新扫描、审核并确认摘要。日志是持久化追加式文件；请保留它，并在恢复同一中断计划时继续使用相同路径。每用户操作系统控制器锁会阻止 `scan`、`manager preview` 与 `manager apply` 并发驱动 HSR 窗口，即使应用命令使用不同日志路径也不例外。应用流程还会在核对、输入与持久化后验证期间持有日志专用锁。

## Attended safety boundary / 有人值守安全边界

- Keep the selected HSR client in the foreground. Input is refused if focus is lost; right-click is the emergency abort during scanning, waits, and mutation.
- Manager operations are limited to lock, unlock, mark discard, and unmark discard. They never salvage, delete, consume, or equip gear.
- Equipped gear and gear whose equipped state is unknown are preview-only and protected from mutation. Unknown or mismatched before-state is also not changed.
- Automated tests and replay devices exercise the capture/import, scanner, and manager contracts, but **no real-device screenshot scan, live packet capture, or live game mutation has been proven by those tests**. Treat live use as attended experimental operation.
- The official Genshin release boundary remains isolated: root builds and official `GOODScanner`/`GOODCapture` artifacts do not contain this executable or feature.

- 请始终将选定的 HSR 客户端保持在前台。焦点丢失时程序拒绝输入；扫描、等待和变更期间可用鼠标右键紧急中止。
- 管理器仅支持锁定、解锁、标记弃置和取消弃置标记；绝不会分解、删除、消耗或装备物品。
- 已装备物品和装备状态未知的物品只能预览，并受到变更保护。变更前状态未知或不匹配时也不会修改。
- 自动化测试和回放设备覆盖了抓包导入、扫描器和管理器契约，但**这些测试没有证明实机截图扫描、实时抓包或实机游戏变更**。实机使用必须视为有人值守的实验操作。
- 正式原神发布边界保持隔离：根工作区构建和正式 `GOODScanner`/`GOODCapture` 产物均不包含本程序或其功能。

Licensing, pinned research revisions, and the non-bundled Reliquary boundary are documented in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

许可证、固定研究版本以及不捆绑 Reliquary 的边界详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
