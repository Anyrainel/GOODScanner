//! Check the hosted catalogs through the same loaders used by the application.
//! Run from a disposable working directory to exercise cold-cache downloads.
use genshin_scanner::scanner::{
    achievement::AchievementCatalog,
    common::mappings::{MappingManager, NameOverrides},
};

fn main() -> anyhow::Result<()> {
    if let Some(path) = std::env::args().nth(1) {
        let data: genshin_scanner::game_data::ScannerData =
            serde_json::from_slice(&std::fs::read(path)?)?;
        data.validate()?;
        println!(
            "Validated shared scanner data: revision={}, artifacts={}",
            data.source_revision,
            data.capture.artifact_map.len()
        );
        return Ok(());
    }
    let mappings = MappingManager::new(&NameOverrides::default())?;
    println!(
        "Loaded mappings: characters={}, weapons={}, artifact sets={}",
        mappings.character_name_map.len(),
        mappings.weapon_name_map.len(),
        mappings.artifact_set_map.len(),
    );
    let achievements = AchievementCatalog::new()?;
    println!(
        "Loaded achievements: categories={}, IDs={}",
        achievements.categories.len(),
        achievements.len(),
    );
    #[cfg(feature = "capture")]
    {
        let capture = genshin_scanner::game_data::load()?.capture;
        println!(
            "Loaded capture data: artifacts={}, sets={}, characters={}, weapons={}",
            capture.artifact_map.len(),
            capture.set_map.len(),
            capture.character_map.len(),
            capture.weapon_map.len(),
        );
    }
    Ok(())
}
