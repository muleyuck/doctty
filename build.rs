// Build the macOS WebKit helper once at source-build time, never at runtime.
#[cfg(target_os = "macos")]
fn main() {
    use std::{env, path::PathBuf, process::Command};

    let source = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest"))
        .join("native/macos/webkit_viewport.swift");
    println!("cargo:rerun-if-changed={}", source.display());
    let dest = PathBuf::from(env::var("OUT_DIR").expect("build directory"))
        .join("doctty-webkit-swift");
    let status = Command::new("swiftc")
        .args(["-swift-version", "5", "-O"])
        .arg(&source)
        .arg("-o")
        .arg(&dest)
        .status()
        .expect("macOS source install requires Swift compiler (swiftc)");
    assert!(status.success(), "Swift helper build failed");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
