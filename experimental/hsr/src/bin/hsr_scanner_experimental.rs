use std::{ffi::OsString, fs::OpenOptions, io::Write, path::PathBuf, process::ExitCode};

use clap::{error::ErrorKind, Parser, ValueEnum};
use hsr_scanner_experimental::{
    build_export, FixtureObservationSource, HsrError, JsonFileReferenceProvider, Language,
    ObservationSource, ReferenceCache,
};

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

#[derive(Debug, Parser)]
#[command(
    name = "HSRScannerExperimental",
    about = "从脱敏语义夹具生成只读 HSR 实验导出 / Generate a read-only experimental HSR export from a sanitized semantic fixture",
    long_about = "实验性且与正式 GOODScanner/GOODCapture 隔离。此程序不捕获实时流量、不控制游戏，也不更改游戏状态。\nExperimental and isolated from official GOODScanner/GOODCapture. This program does not capture live traffic, control the game, or change game state.",
    after_help = "隐私：输入不得包含 UID、账号、令牌、会话、Cookie、游戏 GUID 或原始数据包。\nPrivacy: input must not contain UIDs, accounts, tokens, sessions, cookies, game GUIDs, or raw packets."
)]
struct Cli {
    /// 语言：zh 或 en / Language: zh or en
    #[arg(long, value_enum, default_value = "zh")]
    lang: CliLanguage,

    /// 脱敏语义观测 JSON / Sanitized semantic observation JSON
    #[arg(long)]
    input: PathBuf,

    /// 规范化双语参考数据 JSON / Normalized bilingual reference-data JSON
    #[arg(long)]
    references: PathBuf,

    /// 新建导出文件（不会覆盖）/ New export file (never overwritten)
    #[arg(long)]
    output: PathBuf,
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
                "实验性 HSR 导出失败。\n\n{}",
                "Experimental HSR export failed.\n\n{}",
                error.localized_message(Language::active())
            );
            ExitCode::FAILURE
        },
    }
}

fn run(cli: Cli) -> Result<(), HsrError> {
    yas::log_info!(
        "正在验证脱敏 HSR 夹具和参考数据。",
        "Validating the sanitized HSR fixture and reference data."
    );

    let references =
        ReferenceCache::from_provider(&JsonFileReferenceProvider::new(cli.references))?;
    let observations = FixtureObservationSource::new(cli.input).load()?;
    let export = build_export(observations, &references)?;
    let output = serde_json::to_vec_pretty(&export).map_err(|error| {
        HsrError::write_failed(
            "HSR-EXPORT-JSON",
            format!("serialization failed; cause={error}"),
        )
    })?;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&cli.output)
        .map_err(|error| {
            HsrError::write_failed(
                "HSR-EXPORT-CREATE",
                format!("path={}; cause={error}", cli.output.display()),
            )
        })?;
    file.write_all(&output).map_err(|error| {
        HsrError::write_failed(
            "HSR-EXPORT-WRITE",
            format!("path={}; cause={error}", cli.output.display()),
        )
    })?;
    file.write_all(b"\n").map_err(|error| {
        HsrError::write_failed(
            "HSR-EXPORT-WRITE",
            format!("path={}; cause={error}", cli.output.display()),
        )
    })?;

    yas::log_info!(
        "实验导出完成：{} 个角色、{} 个光锥、{} 件遗器、{} 件位面饰品。输出：{}",
        "Experimental export complete: {} characters, {} Light Cones, {} Relics, and {} Planar Ornaments. Output: {}",
        export.characters.len(),
        export.light_cones.len(),
        export.relics.len(),
        export.planar_ornaments.len(),
        cli.output.display()
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
