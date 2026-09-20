use hsr_scanner::data_cache::{load_from_url, CaptureDataDocument};
use std::{env, fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() == 2 {
        let document: CaptureDataDocument = serde_json::from_slice(&fs::read(&args[1])?)?;
        let cache = document.validate()?;
        println!(
            "Validated hosted data: revision={}, character1508={}",
            cache.revision(),
            cache.character(1508).is_some()
        );
    } else if args.len() == 3 {
        let cache = load_from_url(Path::new(&args[2]), &args[1], true)?;
        println!(
            "Downloaded and cached hosted data: revision={}, character1508={}",
            cache.revision(),
            cache.character(1508).is_some()
        );
    } else {
        return Err("usage: verify_hosted_reference <JSON file> OR <URL> <cache directory>".into());
    }
    Ok(())
}
