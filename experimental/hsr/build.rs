fn main() {
    println!("cargo:rerun-if-changed=assets/hsr-experimental.manifest");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winres::WindowsResource::new();

    // Pure Rust test executables do not control the game. Keep their debug
    // runs unelevated while making elevation mandatory for every release.
    let test_as_invoker = std::env::var("PROFILE").as_deref() == Ok("debug")
        && std::env::var_os("CARGO_FEATURE_TEST_AS_INVOKER").is_some();
    if !test_as_invoker {
        resource.set_manifest_file("assets/hsr-experimental.manifest");
    }

    let version = env!("CARGO_PKG_VERSION");
    resource.set("ProductName", "GOODScanner HSR Experimental");
    resource.set(
        "FileDescription",
        "Experimental Honkai: Star Rail Capture, OCR Scanner, and Manager",
    );
    resource.set("InternalName", "HSRScannerExperimental");
    resource.set("OriginalFilename", "HSRScannerExperimental.exe");
    resource.set("ProductVersion", version);
    resource.set("FileVersion", version);
    resource.set("LegalCopyright", "GPL-2.0-or-later");

    resource
        .compile()
        .expect("compile experimental HSR Windows resources");
}
