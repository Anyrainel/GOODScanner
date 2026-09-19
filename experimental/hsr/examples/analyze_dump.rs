//! Replay dumped decrypted HSR commands through the production decoder
//! and write one HSR-Scanner v4 JSON next to the dump.
use std::{env, fs, path::PathBuf};

use hsr_scanner::{
    load_embedded_gilore_reference,
    packet_capture::{CaptureTargets, HsrPacketDecoder},
};

fn main() {
    let root = PathBuf::from(
        env::args()
            .nth(1)
            .expect("usage: analyze_dump <session dir>"),
    );
    let mut files: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("hsr_") && name.ends_with(".bin"))
        })
        .collect();
    files.sort();
    eprintln!(
        "replaying {} decrypted commands from {}",
        files.len(),
        root.display()
    );

    let references = load_embedded_gilore_reference().unwrap();
    let mut decoder = HsrPacketDecoder::new(references.clone(), CaptureTargets::default()).unwrap();
    for path in files {
        let bytes = fs::read(&path).unwrap();
        if let Err(error) = decoder.receive_command(&bytes) {
            eprintln!(
                "{} DECODE-ERR {}",
                path.file_name().unwrap().to_string_lossy(),
                error
            );
        }
        let state = decoder.state();
        if state.has_characters
            || state.has_light_cones
            || state.has_relics
            || state.has_achievements
        {
            eprintln!(
                "{} chars={}/{} lc={}/{} relics={}/{} ach={}/{} complete={}",
                path.file_name().unwrap().to_string_lossy(),
                state.has_characters,
                state.character_count,
                state.has_light_cones,
                state.light_cone_count,
                state.has_relics,
                state.relic_count,
                state.has_achievements,
                state.achievement_count,
                state.complete
            );
        }
    }
    let state = decoder.state();
    eprintln!(
        "FINAL chars={}/{} lc={}/{} relics={}/{} ach={}/{} complete={}",
        state.has_characters,
        state.character_count,
        state.has_light_cones,
        state.light_cone_count,
        state.has_relics,
        state.relic_count,
        state.has_achievements,
        state.achievement_count,
        state.complete
    );
    if !state.complete {
        std::process::exit(2);
    }
    let path = state.write_interop_exports(&root, &references, "").unwrap();
    eprintln!("export={}", path.display());
}
