//! Live Star Rail capture with packet dumps always enabled.
//!
//! Start this before launching the game, then log in. Dumps land under
//! `debug_capture/hsr/` (override with `HSR_DUMP_DIR`).
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use hsr_scanner::{
    data_cache::load_data_cache,
    packet_capture::{CaptureTargets, HsrCaptureCommand, HsrCaptureMonitor, HsrCaptureState},
};

fn main() {
    let dump_root = PathBuf::from(
        std::env::var("HSR_DUMP_DIR").unwrap_or_else(|_| "debug_capture/hsr".to_owned()),
    );
    eprintln!("dump_root={}", dump_root.display());

    // Blocking HTTP cannot run inside the capture runtime.
    let references = match load_data_cache() {
        Ok(references) => references,
        Err(error) => {
            eprintln!("HSR-LIVE-REF {error}");
            std::process::exit(1);
        },
    };
    eprintln!(
        "references revision={} achievements={}",
        references.revision(),
        references.achievement_count()
    );

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("HSR-LIVE-RUNTIME {error}");
            std::process::exit(1);
        },
    };
    std::process::exit(runtime.block_on(run_capture(dump_root, references)));
}

async fn run_capture(dump_root: PathBuf, references: hsr_scanner::ReferenceCache) -> i32 {
    let state = Arc::new(Mutex::new(HsrCaptureState::default()));
    let monitor = match HsrCaptureMonitor::new(
        state.clone(),
        references.clone(),
        CaptureTargets::default(),
    ) {
        Ok(monitor) => monitor.with_packet_dump(dump_root.clone()),
        Err(error) => {
            eprintln!("HSR-LIVE-MONITOR {error}");
            return 1;
        },
    };

    let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
    if command_tx.send(HsrCaptureCommand::StartCapture).is_err() {
        eprintln!("HSR-LIVE-START could not queue StartCapture");
        return 1;
    }

    let status = {
        let state = state.clone();
        let dump_root = dump_root.clone();
        tokio::spawn(async move {
            let mut saw_capturing = false;
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let snapshot = state.lock().unwrap().clone();
                if snapshot.capturing && !saw_capturing {
                    eprintln!("CAPTURING");
                    if let Ok(entries) = std::fs::read_dir(&dump_root) {
                        for entry in entries.flatten() {
                            eprintln!("dump_session={}", entry.path().display());
                        }
                    }
                }
                saw_capturing |= snapshot.capturing;
                eprintln!(
                    "status packets={} commands={} chars={}/{} lc={}/{} relics={}/{} ach={}/{} complete={} capturing={} transport={} error={}",
                    snapshot.packet_count,
                    snapshot.command_count,
                    snapshot.has_characters,
                    snapshot.character_count,
                    snapshot.has_light_cones,
                    snapshot.light_cone_count,
                    snapshot.has_relics,
                    snapshot.relic_count,
                    snapshot.has_achievements,
                    snapshot.achievement_count,
                    snapshot.complete,
                    snapshot.capturing,
                    snapshot.last_transport_error.as_deref().unwrap_or("none"),
                    snapshot
                        .error
                        .as_ref()
                        .map(|error| error.to_string().replace('\n', " | "))
                        .unwrap_or_else(|| "none".to_owned()),
                );
                if snapshot.complete
                    || snapshot.error.is_some()
                    || (saw_capturing && !snapshot.capturing)
                {
                    break;
                }
            }
            drop(command_tx);
        })
    };

    monitor.run(command_rx).await;
    let _ = status.await;
    let snapshot = state.lock().unwrap().clone();
    if snapshot.complete {
        match write_completed_export(&dump_root, &snapshot, &references) {
            Ok(path) => {
                eprintln!(
                    "COMPLETE characters={} lightCones={} relics={} achievements={}",
                    snapshot.character_count,
                    snapshot.light_cone_count,
                    snapshot.relic_count,
                    snapshot.achievement_count
                );
                eprintln!("export={}", path.display());
                0
            },
            Err(error) => {
                eprintln!("HSR-LIVE-EXPORT {error}");
                1
            },
        }
    } else {
        eprintln!("INCOMPLETE");
        2
    }
}

fn write_completed_export(
    dump_root: &PathBuf,
    snapshot: &HsrCaptureState,
    references: &hsr_scanner::ReferenceCache,
) -> hsr_scanner::HsrResult<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            hsr_scanner::HsrError::write_failed("HSR-LIVE-EXPORT-STAMP", error.to_string())
        })?
        .as_nanos();
    snapshot.write_interop_exports(dump_root, references, &format!("_{stamp}"))
}
