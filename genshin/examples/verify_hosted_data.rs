//! Check the hosted catalogs through the same loaders used by the application.
//! Run from a disposable working directory to exercise cold-cache downloads.
use genshin_scanner::scanner::{
    achievement::AchievementCatalog,
    common::mappings::{MappingManager, NameOverrides},
};

fn main() -> anyhow::Result<()> {
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
        let capture = genshin_scanner::capture::data_cache::load_data_cache()?;
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
