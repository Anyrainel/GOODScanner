//! Star Rail commands in both shipped applications, using the GUI scan path.
use std::{fs::File, path::PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use hsr_scanner::device::{HsrDevice, WindowsHsrDevice};
use yas::cancel::CancelToken;

use crate::{
    config::{ApplicationConfigStore, StarRailCaptureMethod, StarRailSettings},
    gui::{
        star_rail_worker,
        state::{Lang, UiText},
        worker,
    },
};

#[derive(Parser)]
#[command(name = "star-rail", about = "Star Rail scanning / 星穹铁道扫描")]
struct Cli {
    /// Save progress and complete errors to this file / 保存完整日志
    #[arg(long, global = true)]
    log_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan using saved GUI settings and optional overrides / 使用界面设置扫描
    Scan(ScanArgs),
    /// Verify live screenshot dimensions without navigating / 验证实机截图尺寸
    Check {
        #[arg(long, value_enum)]
        capture_method: Option<Capture>,
        /// Enable HDR capture / 启用 HDR 截图
        #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
        hdr_mode: Option<bool>,
        /// Test the scanner's top-bar drag / 测试角色栏拖动
        #[arg(long)]
        drag_character_bar: bool,
        /// Save the captured client for troubleshooting / 保存实机截图
        #[arg(long)]
        save_frame: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Capture {
    Auto,
    Bitblt,
    Wgc,
}

impl Capture {
    fn setting(self) -> StarRailCaptureMethod {
        match self {
            Self::Auto => StarRailCaptureMethod::Auto,
            Self::Bitblt => StarRailCaptureMethod::BitBlt,
            Self::Wgc => StarRailCaptureMethod::Wgc,
        }
    }
}

#[derive(Parser)]
struct ScanArgs {
    /// Select targets explicitly; otherwise use saved targets / 选择扫描目标
    #[arg(long)]
    characters: bool,
    #[arg(long)]
    light_cones: bool,
    #[arg(long)]
    relics: bool,
    #[arg(long, value_enum)]
    capture_method: Option<Capture>,
    /// Enable HDR capture; pass false to override saved HDR / 启用 HDR 截图
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    hdr_mode: Option<bool>,
    /// Maximum Characters (0 = all) / 最大角色扫描数（0 = 全部）
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=500))]
    max_characters: Option<u32>,
    /// Sample each inventory category; coverage remains incomplete / 背包抽样上限
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=10000))]
    sample_items: Option<u32>,
    #[arg(long)]
    output_dir: Option<PathBuf>,
    #[arg(long)]
    dump_images: bool,
}

impl ScanArgs {
    fn apply(&self, settings: &mut StarRailSettings) {
        if self.characters || self.light_cones || self.relics {
            settings.scan_characters = self.characters;
            settings.scan_light_cones = self.light_cones;
            settings.scan_relics_and_ornaments = self.relics;
        }
        if let Some(hdr_mode) = self.hdr_mode {
            settings.set_hdr_mode(hdr_mode);
        }
        if let Some(method) = self.capture_method {
            settings.capture_method = method.setting();
        }
        if let Some(limit) = self.max_characters {
            settings.max_characters = limit as usize;
        }
        if let Some(path) = &self.output_dir {
            settings.output_dir = path.to_string_lossy().into_owned();
        }
        settings.dump_images |= self.dump_images;
    }
}

pub fn run() -> i32 {
    let cli = match Cli::try_parse_from(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            return code;
        },
    };
    let config = genshin_scanner::cli::load_config_or_default();
    yas::lang::set_lang(&config.lang);
    let lang = if yas::lang::is_en() {
        Lang::En
    } else {
        Lang::Zh
    };
    let mut logger = env_logger::Builder::new();
    logger
        .filter_level(log::LevelFilter::Info)
        .format(|buf, record| {
            use std::io::Write;
            writeln!(
                buf,
                "{}",
                yas::lang::localize_log_message(record.target(), &record.args().to_string())
            )
        });
    if let Some(path) = &cli.log_file {
        match File::create(path) {
            Ok(file) => {
                logger.target(env_logger::Target::Pipe(Box::new(file)));
            },
            Err(error) => {
                eprintln!("Cannot create log {}: {error}", path.display());
                return 1;
            },
        }
    }
    if let Err(error) = logger.try_init() {
        eprintln!("Cannot initialize logging: {error}");
        return 1;
    }
    let result = worker::run_cli_with_safety_net(
        UiText::new("星穹铁道操作失败。", "The Star Rail operation failed."),
        move || {
            #[cfg(target_os = "windows")]
            yas::utils::ensure_admin()?;
            match cli.command {
                Command::Scan(args) => {
                    let (store, warning) = ApplicationConfigStore::for_running_executable();
                    if let Some(error) = warning {
                        return Err(error);
                    }
                    let mut settings = store.config.star_rail;
                    args.apply(&mut settings);
                    star_rail_worker::run_scan(
                        &settings,
                        CancelToken::new(),
                        args.sample_items.map(|n| n as usize),
                    )
                    .map_err(|error| anyhow::anyhow!(error.copy_text(lang)))
                },
                Command::Check {
                    capture_method,
                    hdr_mode,
                    drag_character_bar,
                    save_frame,
                } => {
                    let _lease = hsr_scanner::manager::HsrControllerLease::try_acquire()?;
                    let (store, warning) = ApplicationConfigStore::for_running_executable();
                    if let Some(error) = warning {
                        return Err(error);
                    }
                    let mut settings = store.config.star_rail;
                    if let Some(hdr_mode) = hdr_mode {
                        settings.set_hdr_mode(hdr_mode);
                    }
                    if let Some(method) = capture_method {
                        settings.capture_method = method.setting();
                    }
                    let method = settings.capture_method.to_yas().unwrap_or_else(|| {
                        yas::capture::CaptureMethod::for_hdr_mode(settings.hdr_mode)
                    });
                    let mut device = WindowsHsrDevice::locate(method, settings.hdr_mode)?;
                    log::info!("Selected HSR window: {:?}", device.identity());
                    device.focus_and_verify()?;
                    device.wait(std::time::Duration::from_millis(
                        settings.timings.input_settle_ms,
                    ))?;
                    let mut frame = device.capture_client()?;
                    if drag_character_bar {
                        if let Some(path) = &save_frame {
                            frame.save(path.with_extension("before.png"))?;
                        }
                        frame = hsr_scanner::scanner::drag_character_page(
                            &mut device,
                            true,
                            &settings.timings,
                        )?;
                    }
                    if let Some(path) = save_frame {
                        frame.save(path)?;
                    }
                    let (width, height) = frame.dimensions();
                    Ok(UiText::new(
                        format!("截图验证成功：{width} × {height}。"),
                        format!("Live screenshot verified: {width} × {height}."),
                    ))
                },
            }
        },
    );
    match result {
        Ok(message) => {
            log::info!(target: yas::lang::LOCALIZED_LOG_TARGET, "{}", message.text(lang));
            0
        },
        Err(error) => {
            log::error!(target: yas::lang::LOCALIZED_LOG_TARGET, "{}", error.copy_text(lang));
            1
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_hdr_and_automatic_capture_can_override_saved_settings() {
        for (value, expected) in [("true", true), ("false", false)] {
            let cli = Cli::try_parse_from([
                "star-rail",
                "scan",
                "--hdr-mode",
                value,
                "--capture-method",
                "auto",
            ])
            .unwrap();
            let Command::Scan(args) = cli.command else {
                panic!("expected scan")
            };
            let mut settings = StarRailSettings {
                hdr_mode: !expected,
                capture_method: StarRailCaptureMethod::BitBlt,
                ..Default::default()
            };
            args.apply(&mut settings);
            assert_eq!(settings.hdr_mode, expected);
            assert_eq!(settings.capture_method, StarRailCaptureMethod::Auto);
        }
        let cli = Cli::try_parse_from(["star-rail", "check", "--hdr-mode"]).unwrap();
        let Command::Check {
            hdr_mode,
            capture_method,
            ..
        } = cli.command
        else {
            panic!("expected check")
        };
        assert_eq!(hdr_mode, Some(true));
        assert!(capture_method.is_none());
        let cli = Cli::try_parse_from(["star-rail", "scan", "--hdr-mode"]).unwrap();
        let Command::Scan(args) = cli.command else {
            panic!("expected scan")
        };
        let mut settings = StarRailSettings {
            capture_method: StarRailCaptureMethod::BitBlt,
            ..Default::default()
        };
        args.apply(&mut settings);
        assert!(settings.hdr_mode);
        assert_eq!(settings.capture_method, StarRailCaptureMethod::Auto);
    }

    #[test]
    fn retired_capture_backend_is_not_offered_by_the_cli() {
        for command in ["scan", "check"] {
            assert!(
                Cli::try_parse_from(["star-rail", command, "--capture-method", "printwindow"])
                    .is_err()
            );
        }
    }

    #[test]
    fn explicit_inventory_target_disables_character_dependency() {
        let cli = Cli::try_parse_from([
            "star-rail",
            "scan",
            "--relics",
            "--capture-method",
            "wgc",
            "--sample-items",
            "3",
        ])
        .unwrap();
        let Command::Scan(args) = cli.command else {
            panic!("expected scan")
        };
        let mut settings = StarRailSettings::default();
        args.apply(&mut settings);
        assert!(!settings.scan_characters);
        assert!(!settings.scan_light_cones);
        assert!(settings.scan_relics_and_ornaments);
        assert_eq!(settings.capture_method, StarRailCaptureMethod::Wgc);
        assert_eq!(args.sample_items, Some(3));
    }

    #[test]
    fn cli_rejects_zero_sample_but_allows_unlimited_characters() {
        assert!(Cli::try_parse_from(["star-rail", "scan", "--sample-items", "0"]).is_err());
        assert!(Cli::try_parse_from(["star-rail", "scan", "--max-characters", "0"]).is_ok());
    }
}
