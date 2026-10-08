//! Shared dump namespace and exclusive filesystem reservations for both games.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static RUNS: OnceLock<Mutex<HashMap<(String, String), PathBuf>>> = OnceLock::new();

pub fn component(name: &str) -> String {
    // Windows paths are case insensitive. Prefixing also avoids device names.
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "_-[]".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_matches('_');
    if name.is_empty() {
        return "unknown".into();
    }
    let upper = name.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit())
    {
        format!("_{name}")
    } else {
        name.into()
    }
}

pub fn start_run(base: &str, game: &str) {
    RUNS.get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .remove(&(base.into(), game.into()));
}

pub fn save_image_unique(dir: &Path, name: &str, image: &image::RgbImage) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let mut used = std::collections::HashSet::new();
    loop {
        let name = unique_name(&mut used, stem, "png");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(name))
        {
            Ok(file) => {
                return image
                    .write_to(
                        &mut std::io::BufWriter::new(file),
                        image::ImageOutputFormat::Png,
                    )
                    .map_err(std::io::Error::other)
            },
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Reserve a fresh directory atomically, including across concurrent processes.
pub fn reserve_dir(parent: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(parent).expect("cannot create debug image parent directory");
    let name = component(name);
    for attempt in 1.. {
        let dir = parent.join(if attempt == 1 {
            name.clone()
        } else {
            format!("{name}_{attempt}")
        });
        match std::fs::create_dir(&dir) {
            Ok(()) => return dir,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!(
                "cannot reserve debug image directory {}: {error}",
                dir.display()
            ),
        }
    }
    unreachable!()
}

pub fn run_dir(base: &str, game: &str) -> PathBuf {
    assert!(matches!(game, "genshin" | "hsr"));
    let mut runs = RUNS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    runs.entry((base.into(), game.into()))
        .or_insert_with(|| {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            reserve_dir(
                &Path::new(base).join(game),
                &format!("run_{stamp}_{}", std::process::id()),
            )
        })
        .clone()
}

pub fn category_dir(base: &str, game: &str, category: &str) -> PathBuf {
    let dir = run_dir(base, game).join(component(category));
    std::fs::create_dir_all(&dir).expect("cannot create debug image category directory");
    dir
}

pub fn item_dir(base: &str, game: &str, category: &str, index: usize) -> PathBuf {
    reserve_dir(&category_dir(base, game, category), &format!("{index:04}"))
}

/// Reserve a filename without overwriting another capture or metadata file.
pub fn unique_name(
    used: &mut std::collections::HashSet<String>,
    stem: &str,
    extension: &str,
) -> String {
    let stem = component(stem);
    for attempt in 1.. {
        let name = if attempt == 1 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}_{attempt}.{extension}")
        };
        if used.insert(name.to_lowercase()) {
            return name;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_games_and_parallel_repeated_indices_have_exclusive_directories() {
        let base = std::env::temp_dir()
            .join(format!("scanner_paths_test_{}", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let paths: Vec<_> = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..16)
                .map(|i| {
                    let base = &base;
                    scope.spawn(move || {
                        item_dir(
                            base,
                            if i % 2 == 0 { "genshin" } else { "hsr" },
                            "characters",
                            0,
                        )
                    })
                })
                .collect();
            jobs.into_iter().map(|job| job.join().unwrap()).collect()
        });
        assert_eq!(
            paths.iter().collect::<std::collections::HashSet<_>>().len(),
            16
        );
        for path in &paths {
            std::fs::write(path.join("sentinel"), "keep").unwrap();
        }
        start_run(&base, "genshin");
        let next = item_dir(&base, "genshin", "characters", 0);
        assert!(!paths.contains(&next));
        for path in paths {
            assert_eq!(
                std::fs::read_to_string(path.join("sentinel")).unwrap(),
                "keep"
            );
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn legacy_image_writes_reserve_case_insensitive_filenames() {
        let base = std::env::temp_dir().join(format!("scanner_legacy_test_{}", std::process::id()));
        let first = image::RgbImage::from_pixel(3, 2, image::Rgb([1, 2, 3]));
        let second = image::RgbImage::from_pixel(3, 2, image::Rgb([4, 5, 6]));
        save_image_unique(&base, "Field.png", &first).unwrap();
        save_image_unique(&base, "field.png", &second).unwrap();
        assert_eq!(
            image::open(base.join("Field.png")).unwrap().to_rgb8(),
            first
        );
        assert_eq!(
            image::open(base.join("field_2.png")).unwrap().to_rgb8(),
            second
        );
        assert_eq!(component("../CON"), "_CON");
        std::fs::remove_dir_all(base).unwrap();
    }
}
