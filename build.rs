// Build the macOS WebKit helper once at source-build time, never at runtime.
#[cfg(target_os = "macos")]
fn main() {
    use std::{env, path::PathBuf, process::Command};

    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|error| panic!("missing Cargo manifest directory: {error}"));
    let source = PathBuf::from(manifest_dir).join("native/macos/webkit_viewport.swift");
    println!("cargo:rerun-if-changed={}", source.display());
    let output_dir = env::var("OUT_DIR")
        .unwrap_or_else(|error| panic!("missing Cargo build directory: {error}"));
    let dest = PathBuf::from(output_dir).join("doctty-webkit-swift");
    let status = Command::new("swiftc")
        .args(["-swift-version", "5", "-O"])
        .arg(&source)
        .arg("-o")
        .arg(&dest)
        .status()
        .unwrap_or_else(|error| panic!("macOS source install requires swiftc: {error}"));
    assert!(status.success(), "Swift helper build failed");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
