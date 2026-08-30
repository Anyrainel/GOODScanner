# Experimental HSR inventory export / 实验性 HSR 背包导出

This package is an isolated, read-only feasibility slice for Honkai: Star Rail. It is excluded from the root Cargo workspace, has no default feature, and cannot appear in the official `GOODScanner` or `GOODCapture` binaries.

此软件包是《崩坏：星穹铁道》的隔离只读可行性切片。它被根 Cargo 工作区排除、没有默认功能，并且不会出现在正式 `GOODScanner` 或 `GOODCapture` 二进制文件中。

## What works / 已实现

- A normalized experimental export represents Characters, Light Cones, Relics, and Planar Ornaments as separate collections. Bilingual reference names are retained.
- 规范化实验导出将角色、光锥、遗器和位面饰品表示为独立集合，并保留双语参考名称。
- A versioned `ReferenceProvider`/`ReferenceCache` boundary can accept future normalized HSR reference snapshots from GIlore.
- 版本化的 `ReferenceProvider`/`ReferenceCache` 边界可在未来接收 GIlore 生成的规范化 HSR 参考快照。
- Synthetic fixtures prove deterministic conversion, unknown lock/discard states, sensitive-field rejection, and English/Chinese diagnostics.
- 合成夹具验证确定性转换、未知锁定/弃置状态、敏感字段拒绝以及中英文诊断。
- The validated pipeline generates run-local IDs itself and resolves equipped locations from public character template IDs to canonical provider keys.
- 已验证管线自行生成运行本地 ID，并将装备位置从公开角色模板 ID 解析为提供者规范键。

## What is intentionally absent / 有意不包含的功能

- No live game capture, OCR navigation, mouse/keyboard/controller input, packet capture, packet decryption, credentials, session keys, or raw protobuf payloads.
- 不包含实时游戏捕获、OCR 导航、鼠标/键盘/控制器输入、抓包、数据包解密、凭据、会话密钥或原始 protobuf 载荷。
- No lock, unlock, discard-mark, salvage, equip, or other game-state mutation.
- 不包含锁定、解锁、弃置标记、分解、装备或其他游戏状态变更。
- The schema is an internal experimental contract, not a claim of GOOD v3 or Fribbels/Kel-Z v4 compatibility.
- 此架构是内部实验契约，不声称兼容 GOOD v3 或 Fribbels/Kel-Z v4。

## Run the synthetic slice / 运行合成切片

English / 英文：

```powershell
cargo run --manifest-path experimental/hsr/Cargo.toml --features experimental-hsr --bin HSRScannerExperimental -- --lang en --input experimental/hsr/tests/fixtures/observations.json --references experimental/hsr/tests/fixtures/reference_cache.json --output hsr-experimental-en.json
```

Chinese / 中文：

```powershell
cargo run --manifest-path experimental/hsr/Cargo.toml --features experimental-hsr --bin HSRScannerExperimental -- --lang zh --input experimental/hsr/tests/fixtures/observations.json --references experimental/hsr/tests/fixtures/reference_cache.json --output hsr-experimental-zh.json
```

The output path must not exist; the executable uses create-new semantics and will not overwrite a file.

输出路径必须不存在；该可执行文件使用“仅新建”语义，不会覆盖现有文件。

## Test / 测试

```powershell
cargo test --manifest-path experimental/hsr/Cargo.toml --all-features
```

See the bilingual [feasibility report](../../docs/HSR_EXPERIMENTAL_FEASIBILITY.md) for repository history, maintained public mechanisms, protocol evidence, safety boundaries, live-device gaps, and next steps.

仓库历史、维护中的公开机制、协议证据、安全边界、实机缺口和后续步骤，请参阅双语[可行性报告](../../docs/HSR_EXPERIMENTAL_FEASIBILITY.md)。
