use std::{fs, path::PathBuf};

#[test]
fn experimental_package_is_excluded_from_the_official_workspace() {
    let experimental = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = experimental.join("..").join("..");
    let root_manifest =
        fs::read_to_string(root.join("Cargo.toml")).expect("root Cargo.toml must be readable");
    let application_manifest = fs::read_to_string(root.join("application").join("Cargo.toml"))
        .expect("application Cargo.toml must be readable");
    let experimental_manifest = fs::read_to_string(experimental.join("Cargo.toml"))
        .expect("experimental Cargo.toml must be readable");

    assert!(root_manifest.contains("exclude = [\"experimental/hsr\"]"));
    assert!(!application_manifest.to_ascii_lowercase().contains("hsr"));
    assert!(experimental_manifest.contains("name = \"HSRScannerExperimental\""));
    assert!(experimental_manifest.contains("required-features = [\"experimental-hsr\"]"));
    assert!(experimental_manifest.contains("default = []"));
}
