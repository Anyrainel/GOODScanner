use std::{env, fs, path::PathBuf};

use hsr_scanner::{
    generate_embedded_gilore_reference, EMBEDDED_GILORE_COMMIT, EMBEDDED_GILORE_SOURCE_REVISION,
};
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args_os();
    let program = arguments
        .next()
        .unwrap_or_else(|| "generate_embedded_reference".into());
    let bundle_root = arguments.next().map(PathBuf::from);
    let output_path = arguments.next().map(PathBuf::from);
    let mode = arguments.next();
    if bundle_root.is_none()
        || output_path.is_none()
        || arguments.next().is_some()
        || mode.as_deref().is_some_and(|value| value != "--check")
    {
        return Err(format!(
            "usage: {} <GIlore bundle directory> <output JSON> [--check]",
            PathBuf::from(program).display()
        )
        .into());
    }

    let bundle_root = bundle_root.expect("checked above");
    let output_path = output_path.expect("checked above");
    let generated = generate_embedded_gilore_reference(&bundle_root)?;
    let sha256 = format!("{:x}", Sha256::digest(&generated));

    if mode.is_some() {
        let committed = fs::read(&output_path)?;
        if committed != generated {
            return Err(format!(
                "embedded reference is stale: path={}; generatedBytes={}; generatedSha256={sha256}",
                output_path.display(),
                generated.len()
            )
            .into());
        }
        println!(
            "verified path={} bytes={} sha256={sha256} giloreCommit={} sourceRevision={}",
            output_path.display(),
            generated.len(),
            EMBEDDED_GILORE_COMMIT,
            EMBEDDED_GILORE_SOURCE_REVISION
        );
    } else {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output_path, &generated)?;
        println!(
            "wrote path={} bytes={} sha256={sha256} giloreCommit={} sourceRevision={}",
            output_path.display(),
            generated.len(),
            EMBEDDED_GILORE_COMMIT,
            EMBEDDED_GILORE_SOURCE_REVISION
        );
    }

    Ok(())
}
