<div align="center">

# GOODScanner

**中文 | [English](README_EN.md)**

基于 [yas](https://github.com/1803233552/yas) 编写的《原神》与《崩坏：星穹铁道》Windows 扫描、抓包与管理工具

同一套 `GOODScanner.exe` 与 `GOODCapture.exe` 同时服务两款游戏。《原神》数据继续导出为兼容现有工具的 [GOOD v3](https://frzyc.github.io/genshin-optimizer/#/doc)；《星穹铁道》数据导出为可供 GGStarRail 导入的 `goodscanner.hsr` v3。

[![Build](https://github.com/Anyrainel/GOODScanner/actions/workflows/rust.yml/badge.svg)](https://github.com/Anyrainel/GOODScanner/actions)

</div>

## 功能

- **游戏切换**：窗口顶部以等宽的“原神 / 星穹铁道”选项切换流程，并分别保留两款游戏的设置和当前页面
- **原神扫描与抓包**：角色、武器、圣遗物及现有 GOOD v3 导出能力保持兼容
- **原神成就抓包**：GOODCapture 默认导出账号已完成的成就 ID
- **星穹铁道扫描**：角色、光锥、隧洞遗器和位面饰品
- **星穹铁道导出与管理**：生成 `goodscanner.hsr` v3，并提供遗器管理预览及受确认保护的可逆操作
- **星穹铁道成就抓包**：GOODCapture 在程序内捕获并导出已完成的成就，无需另行下载 HSR 程序或抓包辅助程序
- **双引擎 OCR**：PPOCRv4（通用）+ PPOCRv5（特殊部分专用），自动选择最优结果
- **副词条验证**：Roll Solver 基于游戏精确度验证词条合法性

## 快速开始

### 下载

发布页提供两个可执行文件；它们都可在窗口顶部切换《原神》和《星穹铁道》：

- `GOODScanner.exe` — 两款游戏的 OCR 扫描、导出与管理
- `GOODCapture.exe` — 在上述能力之外，增加《原神》数据抓包及两款游戏的成就抓包

请从 [Releases](https://github.com/Anyrainel/GOODScanner/releases) 页面下载。**无需也不应寻找单独的 HSR 可执行文件。**

### 成就导出语义

《原神》继续使用向后兼容的 GOOD v3 扩展字段保存成就：

```json
{"achievements":[80001,80002,81001]}
```

该字段是紧凑的已完成成就 ID 数组。字段存在时（包括空数组）表示导入方应替换成就状态；用户关闭“成就”导出选项时字段会省略，导入方应保留原有成就状态。标准 GOOD v3 字段和版本号保持不变。

《星穹铁道》使用 `schema: "goodscanner.hsr"`、`schemaVersion: 3`。其中可选的 `achievements` 快照只包含公开成就 ID 和 `completed` 状态：字段省略表示本次没有观测成就，导入方须保留原状态；字段存在且 `coverage` 为 `complete` 时表示完整替换，即使 `entries` 是空数组也明确表示已完成数为零。详见 [HSR 实现与契约](docs/HSR_EXPERIMENTAL_FEASIBILITY.md)。

### 使用步骤

1. 以**管理员身份**运行所需的可执行文件；不带命令行参数时会打开图形界面
2. 在窗口顶部选择“原神”或“星穹铁道”，再进入当前游戏的扫描、管理或抓包页面
3. 首次运行《原神》时填写旅行者、流浪者等自定义角色名；现有配置仍保存在 `data/good_config.json`
4. 《星穹铁道》设置单独保存在 `data/good_app_config.json`；共用程序已内置并校验 GIlore 参考数据，参考目录仅用于高级自定义覆盖
5. 确保所选游戏正在运行；扫描过程中可按**鼠标右键**终止
6. 《原神》结果仍输出为 `GOODv3.json`；《星穹铁道》输出目录可在其页面中设置

程序内置完整的公开 GIlore `1.2.0` 参考快照，用于校验《星穹铁道》的库存与成就 ID。若明确选择自定义参考目录，它必须是完整且校验和正确的 GIlore 数据包；无效覆盖会安全拒绝，不会静默回退。正常使用无需另行下载参考数据。

### 扫描目标

以下命令行参数继续用于《原神》，其行为和 GOOD v3 输出保持不变。默认扫描全部（角色 + 武器 + 圣遗物），也可以指定：

```shell
GOODScanner.exe                    # 扫描全部
GOODScanner.exe --characters       # 仅扫描角色
GOODScanner.exe --weapons          # 仅扫描武器
GOODScanner.exe --artifacts        # 仅扫描圣遗物
GOODScanner.exe --characters --weapons  # 组合扫描
```

## 注意事项

- 需要**管理员权限**（用于模拟键鼠输入）
- 《原神》扫描仅支持**简体中文**客户端；《星穹铁道》识别中英文界面文字
- 推荐 **16:9** 分辨率（1920×1080、2560×1440 等）
- 扫描过程中请勿操作鼠标
- 默认 4 星以下圣遗物不扫描（可通过 `--artifact-min-rarity` 调整）

## 命令行参数

### 通用选项

| 参数 | 说明 |
|------|------|
| `-v, --verbose` | 显示详细扫描信息 |
| `--continue-on-failure` | 单项失败时继续扫描 |
| `--log-progress` | 逐项显示扫描进度 |
| `--output-dir <DIR>` | 输出目录（默认当前目录） |
| `--ocr-backend <NAME>` | 覆盖 OCR 后端（ppocrv4 或 ppocrv5） |
| `--dump-images` | 保存 OCR 区域截图到 `debug_images/` |

### 扫描器配置

| 参数 | 说明 |
|------|------|
| `--weapon-min-rarity <N>` | 最低武器稀有度（默认 3） |
| `--artifact-min-rarity <N>` | 最低圣遗物稀有度（默认 4） |
| `--char-max-count <N>` | 最大角色数（0 = 不限） |
| `--weapon-max-count <N>` | 最大武器数（0 = 不限） |
| `--artifact-max-count <N>` | 最大圣遗物数（0 = 不限） |
| `--weapon-skip-delay` | 跳过武器面板等待（更快但锁定检测可能不准） |
| `--artifact-skip-delay` | 跳过圣遗物面板等待（更快但锁定/星标检测可能不准） |
| `--artifact-substat-ocr <NAME>` | 副词条 OCR 后端（默认 ppocrv4） |

### 配置文件

时序参数和角色名通过 `data/good_config.json` 配置，无需命令行参数：

```json
{
  "traveler_name": "",
  "wanderer_name": "",
  "manekin_name": "",
  "manekina_name": "",
  "char_tab_delay": 400,
  "char_open_delay": 1200,
  "weapon_grid_delay": 60,
  "weapon_scroll_delay": 200,
  "artifact_grid_delay": 60,
  "artifact_scroll_delay": 200
}
```

## 从源码构建

```shell
# 需要 stable Rust 工具链
rustup default stable

# 确保安装 Git LFS
git lfs pull

# 构建两款游戏共用的普通扫描版
cargo build --locked --release -p good_tools_app --bin GOODScanner

# 构建包含两款游戏成就抓包的抓包版
cargo build --locked --release -p good_tools_app --features capture --bin GOODCapture

# 产物仍为以下两个既有路径：
# target/release/GOODScanner.exe
# target/release/GOODCapture.exe
```

夹具与回放测试只能证明解析、导出及模拟交互契约，不能证明当前《星穹铁道》客户端上的实时扫描、抓包或管理操作已经通过实机验证。

## 致谢

- [wormtql/yas](https://github.com/wormtql/yas) — 原始项目，提供核心 OCR 扫描框架
- [1803233552/yas](https://github.com/1803233552/yas) — fork 版本，本项目基于此分支开发
- [Andrewthe13th/Inventory_Kamera](https://github.com/Andrewthe13th/Inventory_Kamera) — GOOD 格式扫描器的参考实现

## 反馈

- [GitHub Issues](https://github.com/Anyrainel/GOODScanner/issues)
