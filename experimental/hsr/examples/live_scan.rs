//! Attended live screenshot scan with GOODScanner-style OCR dumps.
//!
//! Caps item counts so dump review stays small. Writes `debug_images/` next to
//! the working directory. Does not change the in-game language.
//!
//! ```text
//! cargo run -p hsr_scanner --example live_scan
//! ```
//!
//! Optional environment:
//! - `HSR_SCAN_TARGETS` — comma list: `characters`, `light_cones`, `gear` (default all)
//! - `HSR_SCAN_MAX_ITEMS` — how many inventory entries to sample (default 3)
//! - `HSR_SCAN_MAX_CHARACTERS` — character sample cap (default 2)
//! - `HSR_SCAN_TRAILBLAZER_NAME` / `HSR_SCAN_TRAILBLAZER_GENDER` (`Stelle` or `Caelus`)

use hsr_scanner::{
    data_cache::load_data_cache,
    scanner::{HsrScanner, ScanConfig, ScanTargets},
    TrailblazerGender, TrailblazerIdentity,
};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if let Err(error) = ensure_ocr_runtime() {
        eprintln!("HSR-LIVE-SCAN-OCR {error}");
        std::process::exit(1);
    }

    let targets = scan_targets();
    let scan_item_limit = env_usize("HSR_SCAN_MAX_ITEMS", 3);
    let max_characters = env_usize("HSR_SCAN_MAX_CHARACTERS", 2);
    let dump_images = std::env::var("HSR_SCAN_DUMP")
        .ok()
        .map(|value| value != "0")
        .unwrap_or(true);
    yas::log_info!(
        "实时扫描目标：角色={} 光锥={} 遗器={}；抽样物品={}；抽样角色={}；转储=debug_images/",
        "Live scan targets: characters={} light_cones={} gear={}; item sample={}; character sample={}; dumps=debug_images/",
        targets.characters,
        targets.light_cones,
        targets.gear,
        scan_item_limit,
        max_characters
    );

    let references = match load_data_cache() {
        Ok(references) => references,
        Err(error) => {
            eprintln!("HSR-LIVE-SCAN-REF {error}");
            std::process::exit(1);
        },
    };

    let config = ScanConfig {
        targets,
        timings: hsr_scanner::scan_timing::ScanTimings {
            panel_timeout_ms: 3_000,
            ..Default::default()
        },
        scan_item_limit: (scan_item_limit > 0).then_some(scan_item_limit),
        max_characters,
        trailblazer: trailblazer(),
        dump_images,
        ..ScanConfig::default()
    };

    match HsrScanner::live(references.clone(), config).and_then(HsrScanner::scan) {
        Ok(result) => {
            yas::log_info!(
                "实时扫描结束：遗器 {} 件；覆盖率={:?}",
                "Live scan finished: {} gear items; coverage={:?}",
                result.gear_items.len(),
                result.coverage
            );
            match hsr_scanner::export_observations(
                &result.observations,
                &references,
                &result.export_details,
                None,
            ) {
                Ok(export) => {
                    let path = std::path::PathBuf::from(
                        std::env::var("HSR_SCAN_OUTPUT").unwrap_or_else(|_| {
                            format!(
                                "debug_capture/star_rail_scan_{}.json",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|duration| duration.as_nanos())
                                    .unwrap_or(0)
                            )
                        }),
                    );
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match hsr_scanner::write_json_create_new(&path, &export) {
                        Ok(()) => eprintln!("export={}", path.display()),
                        Err(error) => {
                            eprintln!("HSR-LIVE-SCAN-EXPORT {error}");
                            std::process::exit(1);
                        },
                    }
                },
                Err(error) => {
                    eprintln!("HSR-LIVE-SCAN-EXPORT {error}");
                    std::process::exit(1);
                },
            }
        },
        Err(error) => {
            eprintln!("HSR-LIVE-SCAN {error}");
            std::process::exit(1);
        },
    }
}

fn scan_targets() -> ScanTargets {
    let raw = std::env::var("HSR_SCAN_TARGETS").unwrap_or_default();
    if raw.trim().is_empty() {
        return ScanTargets::all();
    }
    let mut targets = ScanTargets {
        characters: false,
        light_cones: false,
        gear: false,
    };
    for token in raw.split(',') {
        match token.trim() {
            "characters" => targets.characters = true,
            "light_cones" => targets.light_cones = true,
            "gear" => targets.gear = true,
            other => {
                eprintln!("unknown HSR_SCAN_TARGETS token: {other}");
                std::process::exit(2);
            },
        }
    }
    targets
}

fn trailblazer() -> Option<TrailblazerIdentity> {
    let nickname = std::env::var("HSR_SCAN_TRAILBLAZER_NAME").ok()?;
    let gender = match std::env::var("HSR_SCAN_TRAILBLAZER_GENDER").as_deref() {
        Ok("Stelle") => TrailblazerGender::Stelle,
        Ok("Caelus") => TrailblazerGender::Caelus,
        other => {
            eprintln!("HSR_SCAN_TRAILBLAZER_GENDER must be Stelle or Caelus; got {other:?}");
            std::process::exit(2);
        },
    };
    Some(TrailblazerIdentity { nickname, gender })
}

fn ensure_ocr_runtime() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        genshin_scanner::cli::check_vcpp_runtime().map_err(|error| error.to_string())?;
        if !genshin_scanner::cli::check_onnxruntime() {
            genshin_scanner::cli::download_onnxruntime().map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
