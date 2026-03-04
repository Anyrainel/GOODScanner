use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{command, ArgMatches, Args, FromArgMatches};
use log::info;

use yas::export::{AssetEmitter, ExportAssets};
use yas::game_info::{GameInfo, GameInfoBuilder};
use yas::window_info::{load_window_info_repo, WindowInfoRepository};

use crate::artifact::GenshinArtifact;
use crate::character::GenshinCharacter;
use crate::export::artifact::good::GOODFormat;
use crate::scanner::{
    GenshinArtifactScanner, GenshinArtifactScannerConfig,
    GenshinWeaponScanner, GenshinWeaponScannerConfig, GenshinWeaponScanResult,
    GenshinCharacterScanner, GenshinCharacterScannerConfig, GenshinCharacterScanResult,
};
use crate::scanner_controller::repository_layout::GenshinRepositoryScannerLogicConfig;
use crate::weapon::GenshinWeapon;
use crate::weapon::weapon_name_to_good;
use crate::weapon::weapon::parse_level_and_ascension;
use crate::weapon::weapon::parse_refinement;

#[derive(Clone, clap::Args)]
pub struct CombinedScanConfig {
    /// Scan artifacts
    #[arg(long = "scan-artifacts", help = "扫描圣遗物")]
    pub scan_artifacts: bool,

    /// Scan weapons
    #[arg(long = "scan-weapons", help = "扫描武器")]
    pub scan_weapons: bool,

    /// Scan characters
    #[arg(long = "scan-characters", help = "扫描角色")]
    pub scan_characters: bool,

    /// Scan everything (artifacts + weapons + characters)
    #[arg(long = "scan-all", help = "扫描全部(圣遗物+武器+角色)")]
    pub scan_all: bool,

    /// Output directory
    #[arg(long = "output-dir", help = "输出目录", default_value = ".")]
    pub output_dir: String,
}

pub struct CombinedScannerApplication {
    arg_matches: ArgMatches,
}

impl CombinedScannerApplication {
    pub fn new(matches: ArgMatches) -> Self {
        CombinedScannerApplication {
            arg_matches: matches,
        }
    }

    pub fn build_command() -> clap::Command {
        let mut cmd = command!();
        cmd = <CombinedScanConfig as Args>::augment_args_for_update(cmd);
        cmd = <GenshinArtifactScannerConfig as Args>::augment_args_for_update(cmd);
        cmd = <GenshinWeaponScannerConfig as Args>::augment_args_for_update(cmd);
        cmd = <GenshinCharacterScannerConfig as Args>::augment_args_for_update(cmd);
        cmd = <GenshinRepositoryScannerLogicConfig as Args>::augment_args_for_update(cmd);
        cmd
    }

    fn get_window_info_repository() -> WindowInfoRepository {
        load_window_info_repo!(
            "../../window_info/windows1366x768.json",
            "../../window_info/windows1024x768.json",
            "../../window_info/windows1600x900.json",
            "../../window_info/windows1280x960.json",
            "../../window_info/windows1440x900.json",
            "../../window_info/windows2100x900.json",
            "../../window_info/windows2560x1440.json",
            "../../window_info/windows3440x1440.json",
        )
    }

    fn get_game_info() -> Result<GameInfo> {
        GameInfoBuilder::new()
            .add_local_window_name("原神")
            .add_local_window_name("Genshin Impact")
            .add_cloud_window_name("云·原神")
            .build()
    }

    /// Convert raw weapon scan results to GenshinWeapon structs
    fn convert_weapon_results(scan_results: &[GenshinWeaponScanResult]) -> Vec<GenshinWeapon> {
        scan_results.iter().filter_map(|r| {
            let good_name = weapon_name_to_good(&r.name).unwrap_or(&r.name);

            let (level, ascension, _ascended) = parse_level_and_ascension(&r.level)
                .unwrap_or((1, 0, false));

            let refinement = parse_refinement(&r.refinement).unwrap_or(1);

            let equip = if r.equip.is_empty() {
                None
            } else {
                Some(r.equip.clone())
            };

            Some(GenshinWeapon {
                name: good_name.to_string(),
                level,
                ascension,
                refinement,
                star: r.star,
                lock: r.lock,
                equip,
            })
        }).collect()
    }

    /// Convert raw character scan results to GenshinCharacter structs
    fn convert_character_results(scan_results: &[GenshinCharacterScanResult]) -> Vec<GenshinCharacter> {
        scan_results.iter().map(|r| {
            GenshinCharacter {
                name: r.name.clone(),
                element: r.element.clone(),
                level: r.level,
                ascension: r.ascension,
                constellation: r.constellation,
                talent_auto: r.talent_auto,
                talent_skill: r.talent_skill,
                talent_burst: r.talent_burst,
            }
        }).collect()
    }
}

impl CombinedScannerApplication {
    pub fn run(&self) -> Result<()> {
        let arg_matches = &self.arg_matches;
        let config = CombinedScanConfig::from_arg_matches(arg_matches)?;
        let window_info_repository = Self::get_window_info_repository();
        let game_info = Self::get_game_info()?;

        info!("window: {:?}", game_info.window);
        info!("ui: {:?}", game_info.ui);
        info!("cloud: {}", game_info.is_cloud);
        info!("resolution family: {:?}", game_info.resolution_family);

        #[cfg(target_os = "windows")]
        {
            if !yas::utils::is_admin() {
                return Err(anyhow!("请使用管理员运行"));
            }
        }

        // Determine what to scan
        let scan_artifacts = config.scan_artifacts || config.scan_all || (!config.scan_weapons && !config.scan_characters);
        let scan_weapons = config.scan_weapons || config.scan_all;
        let scan_characters = config.scan_characters || config.scan_all;

        let mut artifacts: Option<Vec<GenshinArtifact>> = None;
        let mut weapons: Option<Vec<GenshinWeapon>> = None;
        let mut characters: Option<Vec<GenshinCharacter>> = None;

        // Scan artifacts (user should already be on artifact tab)
        if scan_artifacts {
            info!("=== 开始扫描圣遗物 ===");
            let mut scanner = GenshinArtifactScanner::from_arg_matches(
                &window_info_repository,
                arg_matches,
                game_info.clone(),
            )?;
            let result = scanner.scan()?;
            let arts: Vec<GenshinArtifact> = result
                .iter()
                .flat_map(GenshinArtifact::try_from)
                .collect();
            info!("扫描到 {} 件圣遗物", arts.len());
            artifacts = Some(arts);
        }

        // Scan weapons
        if scan_weapons {
            info!("=== 开始扫描武器 ===");
            let weapon_config = GenshinWeaponScannerConfig::from_arg_matches(arg_matches)?;
            let controller_config = GenshinRepositoryScannerLogicConfig::from_arg_matches(arg_matches)?;
            let mut scanner = GenshinWeaponScanner::new(
                &window_info_repository,
                weapon_config,
                controller_config,
                game_info.clone(),
            )?;
            let result = scanner.scan()?;
            let weaps = Self::convert_weapon_results(&result);
            info!("扫描到 {} 把武器", weaps.len());
            weapons = Some(weaps);
        }

        // Scan characters
        if scan_characters {
            info!("=== 开始扫描角色 ===");
            let char_config = GenshinCharacterScannerConfig::from_arg_matches(arg_matches)?;
            let mut scanner = GenshinCharacterScanner::new(
                &window_info_repository,
                char_config,
                game_info.clone(),
            )?;
            let result = scanner.scan()?;
            let chars = Self::convert_character_results(&result);
            info!("扫描到 {} 个角色", chars.len());
            characters = Some(chars);
        }

        // Export as GOODv3
        let output_dir = PathBuf::from(&config.output_dir);
        let path = output_dir.join("good.json");

        let good = GOODFormat::new_v3(
            artifacts.as_deref(),
            characters.as_deref(),
            weapons.as_deref(),
        );

        let contents = serde_json::to_string(&good)?;
        let mut export_assets = ExportAssets::new();
        export_assets.add_asset(
            Some(String::from("GOOD")),
            path,
            contents.into_bytes(),
            Some(String::from("GOODv3格式(圣遗物+武器+角色)")),
        );

        let stats = export_assets.save();
        info!("保存结果：");
        for line in format!("{}", stats).lines() {
            info!("{}", line);
        }

        if let Some(ref arts) = artifacts {
            info!("圣遗物: {} 件", arts.len());
        }
        if let Some(ref weaps) = weapons {
            info!("武器: {} 把", weaps.len());
        }
        if let Some(ref chars) = characters {
            info!("角色: {} 个", chars.len());
        }

        Ok(())
    }
}
