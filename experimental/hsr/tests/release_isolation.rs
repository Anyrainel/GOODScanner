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

#[test]
fn windows_identity_and_elevation_manifest_are_experimental_only() {
    let experimental = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = experimental.join("..").join("..");
    let build_script = fs::read_to_string(experimental.join("build.rs"))
        .expect("experimental build.rs must be readable");
    let manifest = fs::read_to_string(
        experimental
            .join("assets")
            .join("hsr-experimental.manifest"),
    )
    .expect("experimental application manifest must be readable");
    let official_build_script = fs::read_to_string(root.join("application").join("build.rs"))
        .expect("official build.rs must be readable");
    let official_manifest = fs::read_to_string(root.join("assets").join("manifest.xml"))
        .expect("official application manifest must be readable");

    assert!(build_script.contains("GOODScanner HSR Experimental"));
    assert!(build_script.contains("HSRScannerExperimental.exe"));
    assert!(build_script.contains("hsr-experimental.manifest"));
    assert!(manifest.contains("GOODScanner.HSRScannerExperimental"));
    assert!(manifest
        .contains("<requestedExecutionLevel level=\"requireAdministrator\" uiAccess=\"false\" />"));

    assert!(!official_build_script.contains("HSRScannerExperimental"));
    assert!(!official_manifest.contains("HSRScannerExperimental"));
}
