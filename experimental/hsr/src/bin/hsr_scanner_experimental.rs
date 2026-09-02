use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{error::ErrorKind, Args, Parser, Subcommand, ValueEnum};
use hsr_scanner_experimental::{
    build_export,
    capture::{import_reliquary_archive_file, PinnedReliquaryArchiver},
    manager::{
        apply_manager_envelope, build_manager_plan, validate_manager_envelope_reference,
        AppendOnlyJsonJournalStore, ApplyAuthorization, HsrControllerLease,
        ManagerInstructionsEnvelope, MutationScope,
    },
    scanner::{HsrScanner, ScanConfig, ScanTargets},
    FixtureObservationSource, GiloreBundleReferenceProvider, HsrError, Language, LocalizedText,
    ObservationSource, ReferenceCache,
};
use yas::capture::CaptureMethod;

const MAX_MANAGER_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliLanguage {
    #[value(name = "zh")]
    Zh,
    #[value(name = "en")]
    En,
}

impl CliLanguage {
    fn language(self) -> Language {
        match self {
            Self::Zh => Language::ZhCn,
            Self::En => Language::En,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliCaptureMethod {
    #[value(name = "wgc")]
    Wgc,
    #[value(name = "bitblt")]
    BitBlt,
    #[value(name = "print-window")]
    PrintWindow,
}

impl CliCaptureMethod {
    fn method(self) -> CaptureMethod {
        match self {
            Self::Wgc => CaptureMethod::Wgc,
            Self::BitBlt => CaptureMethod::BitBlt,
            Self::PrintWindow => CaptureMethod::PrintWindow,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "HSRScannerExperimental",
    version,
    about = "隔离的 HSR 抓包、截图 OCR 与安全管理实验 / Isolated HSR capture, screenshot OCR, and attended-manager experiment",
    long_about = "实验性且与正式 GOODScanner/GOODCapture 完全隔离。支持双语截图 OCR、用户提供并校验哈希的 Reliquary Archiver 抓包导入，以及仅预览后凭摘要确认的可逆状态管理。\nExperimental and fully isolated from official GOODScanner/GOODCapture. Supports bilingual screenshot OCR, capture import through a user-supplied hash-pinned Reliquary Archiver, and reversible state management only after a digest-bound preview.",
    after_help = "安全边界：不接受账号 UID、Cookie、令牌、原始数据包或服务器物品 ID；不会下载辅助程序；不会分解、删除或装备物品；实时变更需要单独运行 apply 并提供预览摘要和逐类授权。\nSafety boundary: account UIDs, cookies, tokens, raw packets, and server item IDs are never exported; no helper is downloaded; nothing is salvaged, deleted, or equipped; live changes require a separate apply run with the preview digest and per-action authorization."
)]
struct Cli {
    /// 语言：zh 或 en / Language: zh or en
    #[arg(long, value_enum, default_value = "zh", global = true)]
    lang: CliLanguage,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 从脱敏语义夹具生成 v2 导出 / Export v2 from a sanitized semantic fixture
    FixtureExport(FixtureExportArgs),
    /// 通过截图、控制器导航和 OCR 扫描客户端 / Scan through screenshots, navigation, and OCR
    Scan(ScanArgs),
    /// 运行已固定哈希的 Reliquary Archiver 并导入结果 / Run a hash-pinned Reliquary Archiver and import it
    Capture(CaptureArgs),
    /// 离线导入 Reliquary/Fribbels v4 归档 / Import a Reliquary/Fribbels v4 archive offline
    ImportCapture(ImportCaptureArgs),
    /// 预览或应用 GGStarRail 管理指令 / Preview or apply GGStarRail manager instructions
    Manager(ManagerArgs),
}

#[derive(Debug, Args)]
struct ReferenceArgs {
    /// GIlore 生成的 ggstarrail-reference 目录 / GIlore ggstarrail-reference bundle directory
    #[arg(long)]
    reference_bundle: PathBuf,
}

#[derive(Debug, Args)]
struct FixtureExportArgs {
    #[command(flatten)]
    reference: ReferenceArgs,
    /// 脱敏语义观测 JSON / Sanitized semantic observation JSON
    #[arg(long)]
    input: PathBuf,
    /// 新建导出文件（不会覆盖）/ New export file (never overwritten)
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct LiveScanOptions {
    /// Windows Graphics Capture（推荐）、BitBlt 或 PrintWindow / Capture backend
    #[arg(long, value_enum, default_value = "wgc")]
    capture_method: CliCaptureMethod,
    /// 导航操作后的等待毫秒数 / Milliseconds to wait after navigation input
    #[arg(long, default_value_t = 250, value_parser = clap::value_parser!(u64).range(50..=5_000))]
    navigation_delay_ms: u64,
    /// 稳定面板等待上限毫秒数 / Stable-panel timeout in milliseconds
    #[arg(long, default_value_t = 900, value_parser = clap::value_parser!(u64).range(250..=10_000))]
    panel_timeout_ms: u64,
    /// 背包物品安全上限 / Safety limit for inventory items
    #[arg(long, default_value_t = 2_000, value_parser = clap::value_parser!(u64).range(1..=10_000))]
    max_inventory_items: u64,
}

impl LiveScanOptions {
    fn scanner_config(&self, targets: ScanTargets) -> ScanConfig {
        ScanConfig {
            targets,
            capture_method: self.capture_method.method(),
            navigation_delay: Duration::from_millis(self.navigation_delay_ms),
            panel_timeout: Duration::from_millis(self.panel_timeout_ms),
            max_inventory_items: self.max_inventory_items as usize,
            ..ScanConfig::default()
        }
    }
}

#[derive(Debug, Args)]
struct ScanArgs {
    #[command(flatten)]
    reference: ReferenceArgs,
    #[command(flatten)]
    live: LiveScanOptions,
    /// 扫描角色 / Scan Characters
    #[arg(long)]
    characters: bool,
    /// 扫描光锥 / Scan Light Cones
    #[arg(long)]
    light_cones: bool,
    /// 扫描隧洞遗器和位面饰品 / Scan Cavern Relics and Planar Ornaments
    #[arg(long)]
    relics: bool,
    /// 扫描全部四类（未指定类别时也是默认值）/ Scan all four categories (also the default)
    #[arg(long)]
    all: bool,
    /// 已知角色总数；提供后才能证明角色覆盖完整 / Expected Character count required to claim complete Character coverage
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=500))]
    expected_characters: Option<u64>,
    /// 角色扫描安全上限 / Character scan safety limit
    #[arg(long, default_value_t = 200, value_parser = clap::value_parser!(u64).range(1..=500))]
    max_characters: u64,
    /// 下一个角色按键（默认 E，实机校准前不声称已验证）/ Next-Character key (default E; not claimed calibrated before live proof)
    #[arg(long, default_value_t = 'e')]
    next_character_key: char,
    /// 新建导出文件（不会覆盖）/ New export file (never overwritten)
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct CaptureArgs {
    #[command(flatten)]
    reference: ReferenceArgs,
    /// 用户提供的 Reliquary Archiver v0.18.0 可执行文件 / User-supplied Reliquary Archiver v0.18.0 executable
    #[arg(long)]
    archiver: PathBuf,
    /// 上述文件经用户核准的 SHA-256 / User-approved SHA-256 of that exact executable
    #[arg(long)]
    archiver_sha256: String,
    /// 抓包等待秒数 / Capture timeout in seconds
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=600))]
    timeout_seconds: u64,
    /// 新建导出文件（不会覆盖）/ New export file (never overwritten)
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct ImportCaptureArgs {
    #[command(flatten)]
    reference: ReferenceArgs,
    /// 已存在的 Reliquary/Fribbels v4 JSON / Existing Reliquary/Fribbels v4 JSON
    #[arg(long)]
    input: PathBuf,
    /// 新建导出文件（不会覆盖）/ New export file (never overwritten)
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct ManagerArgs {
    #[command(subcommand)]
    action: ManagerCommand,
}

#[derive(Debug, Subcommand)]
enum ManagerCommand {
    /// 完整重扫并显示精确预览；不变更游戏 / Rescan all gear and show an exact preview; never mutate
    Preview(ManagerPreviewArgs),
    /// 重新生成预览并按摘要和范围授权应用 / Rebuild preview and apply digest/scope-authorized changes
    Apply(ManagerApplyArgs),
}

#[derive(Debug, Args)]
struct ManagerPreviewArgs {
    #[command(flatten)]
    reference: ReferenceArgs,
    #[command(flatten)]
    live: LiveScanOptions,
    /// GGStarRail 版本化管理指令 JSON / Versioned GGStarRail manager-instructions JSON
    #[arg(long)]
    instructions: PathBuf,
}

#[derive(Debug, Args)]
struct ManagerApplyArgs {
    #[command(flatten)]
    reference: ReferenceArgs,
    #[command(flatten)]
    live: LiveScanOptions,
    /// 与 preview 相同的 GGStarRail 指令文件 / Same instruction file used for preview
    #[arg(long)]
    instructions: PathBuf,
    /// preview 显示的完整 SHA-256 确认摘要 / Full SHA-256 confirmation digest printed by preview
    #[arg(long)]
    confirm_digest: String,
    /// 持久化追加式恢复日志 / Durable append-only recovery journal
    #[arg(long)]
    journal: PathBuf,
    /// 明确确认本次是实时状态变更 / Explicitly confirm this is a live state mutation
    #[arg(long)]
    confirm_live_mutation: bool,
    /// 授权锁定 / Authorize locking
    #[arg(long)]
    allow_lock: bool,
    /// 授权解锁 / Authorize unlocking
    #[arg(long)]
    allow_unlock: bool,
    /// 授权标记弃置（不会分解）/ Authorize marking discard (never salvage)
    #[arg(long)]
    allow_mark_discard: bool,
    /// 授权取消弃置标记 / Authorize unmarking discard
    #[arg(long)]
    allow_unmark_discard: bool,
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let requested_language = preparse_language(&args);
    yas::lang::set_lang(requested_language.code());
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            let _ = error.print();
            return ExitCode::SUCCESS;
        },
        Err(error) => {
            let heading = match requested_language {
                Language::ZhCn => "命令行参数无效。",
                Language::En => "The command-line arguments are invalid.",
            };
            eprintln!("{heading}\n");
            let _ = error.print();
            return ExitCode::from(2);
        },
    };
    yas::lang::set_lang(cli.lang.language().code());
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format_timestamp(None)
        .init();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            yas::log_error!(
                "实验性 HSR 操作失败。\n\n{}",
                "Experimental HSR operation failed.\n\n{}",
                error.localized_message(Language::active())
            );
            ExitCode::FAILURE
        },
    }
}

fn run(cli: Cli) -> Result<(), HsrError> {
    match cli.command {
        Command::FixtureExport(args) => run_fixture_export(args),
        Command::Scan(args) => run_scan(args),
        Command::Capture(args) => run_capture(args),
        Command::ImportCapture(args) => run_capture_import(args),
        Command::Manager(args) => run_manager(args.action),
    }
}

fn run_fixture_export(args: FixtureExportArgs) -> Result<(), HsrError> {
    let references = load_references(&args.reference)?;
    let observations = FixtureObservationSource::new(args.input).load()?;
    write_export(&args.output, &build_export(observations, &references)?)
}

fn run_scan(args: ScanArgs) -> Result<(), HsrError> {
    let _controller_lease = HsrControllerLease::try_acquire()?;
    let references = load_live_references(&args.reference)?;
    let default_all = !args.characters && !args.light_cones && !args.relics;
    let targets = ScanTargets {
        characters: args.all || default_all || args.characters,
        light_cones: args.all || default_all || args.light_cones,
        gear: args.all || default_all || args.relics,
    };
    let mut config = args.live.scanner_config(targets);
    config.expected_characters = args.expected_characters.map(|count| count as usize);
    config.max_characters = args.max_characters as usize;
    config.next_character_key = args.next_character_key;
    let result = HsrScanner::live(references.clone(), config)?.scan()?;
    write_export(
        &args.output,
        &build_export(result.observations, &references)?,
    )
}

fn run_capture(args: CaptureArgs) -> Result<(), HsrError> {
    let references = load_references(&args.reference)?;
    yas::log_info!(
        "只读抓包即将开始。请让 HSR 客户端停在“点击开始”前；辅助输出位于私有临时目录，若无法清理会报告路径，原始数据包不会导出。",
        "Read-only capture is about to start. Leave the HSR client before Click to Start; helper output stays in a private temporary directory whose cleanup failure is reported, and raw packets are never exported."
    );
    let archiver = PinnedReliquaryArchiver::new(args.archiver, args.archiver_sha256)?
        .with_timeout_seconds(args.timeout_seconds)?;
    let imported = archiver.capture(&references)?;
    write_export(
        &args.output,
        &build_export(imported.into_observations(), &references)?,
    )
}

fn run_capture_import(args: ImportCaptureArgs) -> Result<(), HsrError> {
    let references = load_references(&args.reference)?;
    let imported = import_reliquary_archive_file(args.input, &references)?;
    write_export(
        &args.output,
        &build_export(imported.into_observations(), &references)?,
    )
}

fn run_manager(command: ManagerCommand) -> Result<(), HsrError> {
    match command {
        ManagerCommand::Preview(args) => {
            let _controller_lease = HsrControllerLease::try_acquire()?;
            let references = load_live_references(&args.reference)?;
            let envelope = load_manager_instructions(&args.instructions)?;
            validate_manager_envelope_reference(&envelope, &references)?;
            let config = args.live.scanner_config(manager_targets());
            let mut scanner = HsrScanner::live(references, config)?;
            let inventory = scanner.scan_manager_inventory()?;
            let plan = build_manager_plan(&envelope, &inventory)?;
            print_exact_plan(&plan)
        },
        ManagerCommand::Apply(args) => {
            if !args.confirm_live_mutation {
                return Err(HsrError::new(
                    "HSR_MANAGER_EXPLICIT_CONFIRMATION_REQUIRED",
                    LocalizedText::new(
                        "缺少单独的实时变更确认；不会执行任何游戏操作。",
                        "The separate live-mutation confirmation is missing; no game action was performed.",
                    ),
                    "manager apply requires --confirm-live-mutation after reviewing manager preview",
                ));
            }
            let controller_lease = HsrControllerLease::try_acquire()?;
            let references = load_live_references(&args.reference)?;
            let envelope = load_manager_instructions(&args.instructions)?;
            validate_manager_envelope_reference(&envelope, &references)?;
            let config = args.live.scanner_config(manager_targets());
            let authorized_scopes = scopes(&args);
            let authorization = ApplyAuthorization::new(args.confirm_digest, authorized_scopes);
            let mut journal = AppendOnlyJsonJournalStore::new(args.journal)
                .try_acquire_apply_lease_with_controller(controller_lease)?;
            let mut scanner = HsrScanner::live(references, config)?;
            let report = apply_manager_envelope(
                &envelope,
                &authorization,
                &mut scanner,
                &mut journal,
                |scanner| scanner.scan_manager_inventory(),
                print_exact_plan,
            )?;
            println!("{}", report.render(Language::active()));
            Ok(())
        },
    }
}

fn manager_targets() -> ScanTargets {
    ScanTargets {
        characters: false,
        light_cones: false,
        gear: true,
    }
}

fn scopes(args: &ManagerApplyArgs) -> BTreeSet<MutationScope> {
    let mut scopes = BTreeSet::new();
    if args.allow_lock {
        scopes.insert(MutationScope::Lock);
    }
    if args.allow_unlock {
        scopes.insert(MutationScope::Unlock);
    }
    if args.allow_mark_discard {
        scopes.insert(MutationScope::MarkDiscard);
    }
    if args.allow_unmark_discard {
        scopes.insert(MutationScope::UnmarkDiscard);
    }
    scopes
}

fn load_references(args: &ReferenceArgs) -> Result<ReferenceCache, HsrError> {
    ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(&args.reference_bundle))
}

fn load_live_references(args: &ReferenceArgs) -> Result<ReferenceCache, HsrError> {
    let references = load_references(args)?;
    references.validate_live_complete_profile()?;
    Ok(references)
}

fn load_manager_instructions(path: &Path) -> Result<ManagerInstructionsEnvelope, HsrError> {
    let metadata = fs::metadata(path).map_err(|error| manager_read_error(path, error))?;
    if !metadata.is_file() || metadata.len() > MAX_MANAGER_BYTES {
        return Err(HsrError::new(
            "HSR_MANAGER_READ",
            LocalizedText::new(
                "HSR 管理指令文件无效或过大。",
                "The HSR manager-instructions file is invalid or too large.",
            ),
            format!(
                "path={}; regularFile={}; bytes={}; maximum={MAX_MANAGER_BYTES}",
                path.display(),
                metadata.is_file(),
                metadata.len()
            ),
        ));
    }
    let input = fs::read_to_string(path).map_err(|error| manager_read_error(path, error))?;
    ManagerInstructionsEnvelope::parse_json(&input)
}

fn manager_read_error(path: &Path, error: std::io::Error) -> HsrError {
    HsrError::new(
        "HSR_MANAGER_READ",
        LocalizedText::new(
            "无法读取 HSR 管理指令文件。",
            "Could not read the HSR manager-instructions file.",
        ),
        format!("path={}; cause={error}", path.display()),
    )
}

fn print_exact_plan(plan: &hsr_scanner_experimental::manager::ManagerPlan) -> Result<(), HsrError> {
    println!("{}", plan.render(Language::active()));
    let exact = serde_json::to_string_pretty(plan).map_err(|error| {
        HsrError::new(
            "HSR_MANAGER_PREVIEW_SERIALIZE",
            LocalizedText::new(
                "无法生成精确的管理预览；不会执行任何游戏操作。",
                "Could not render the exact manager preview; no game action was performed.",
            ),
            format!("manager plan serialization failed; cause={error}"),
        )
    })?;
    let label = match Language::active() {
        Language::ZhCn => "精确预览 JSON（完整匹配字段、当前状态和目标变更）：",
        Language::En => {
            "Exact preview JSON (complete matcher, current state, and desired changes):"
        },
    };
    println!("\n{label}\n{exact}");
    Ok(())
}

fn write_export(
    path: &Path,
    export: &hsr_scanner_experimental::HsrInventoryExport,
) -> Result<(), HsrError> {
    let output = serde_json::to_vec_pretty(export).map_err(|error| {
        HsrError::write_failed(
            "HSR-EXPORT-JSON",
            format!("serialization failed; cause={error}"),
        )
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            HsrError::write_failed(
                "HSR-EXPORT-CREATE",
                format!("path={}; cause={error}", path.display()),
            )
        })?;
    file.write_all(&output)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            HsrError::write_failed(
                "HSR-EXPORT-WRITE",
                format!("path={}; cause={error}", path.display()),
            )
        })?;
    yas::log_info!(
        "实验导出完成：{} 个角色、{} 个光锥、{} 件隧洞遗器、{} 件位面饰品。输出：{}",
        "Experimental export complete: {} Characters, {} Light Cones, {} Cavern Relics, and {} Planar Ornaments. Output: {}",
        export.characters.len(),
        export.light_cones.len(),
        export.relics.len(),
        export.planar_ornaments.len(),
        path.display()
    );
    Ok(())
}

fn preparse_language(args: &[OsString]) -> Language {
    for (index, argument) in args.iter().enumerate() {
        let argument = argument.to_string_lossy();
        if let Some(value) = argument.strip_prefix("--lang=") {
            return if value.eq_ignore_ascii_case("en") {
                Language::En
            } else {
                Language::ZhCn
            };
        }
        if argument == "--lang" {
            if let Some(value) = args.get(index + 1) {
                return if value.to_string_lossy().eq_ignore_ascii_case("en") {
                    Language::En
                } else {
                    Language::ZhCn
                };
            }
        }
    }
    Language::ZhCn
}
