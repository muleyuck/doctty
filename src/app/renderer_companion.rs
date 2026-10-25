//! Version-matched, compiled renderer companion for macOS interactive sessions.
//!
//! The same compiled renderer is the default for every macOS install.
//! Source builds embed the compiled Swift helper; ZIPs ship it alongside doctty.

use std::path::Path;

use anyhow::{Result, bail};

use crate::{cli::Theme, core::config::MermaidMode};

#[cfg(target_os = "macos")]
use anyhow::Context;
#[cfg(any(target_os = "macos", test))]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(any(target_os = "macos", test))]
fn companion_paths(main_executable: &Path) -> (PathBuf, PathBuf) {
    let directory = main_executable.parent().unwrap_or_else(|| Path::new("."));
    (
        directory.join("doctty-renderer"),
        directory.join("doctty-webkit-swift"),
    )
}

#[cfg(any(target_os = "macos", test))]
fn require_companions(main_executable: &Path) -> Result<(PathBuf, PathBuf)> {
    let (viewer, swift_helper) = companion_paths(main_executable);
    if !viewer.is_file() {
        bail!(
            "missing doctty-renderer next to {}. Install the complete doctty              package with cargo install --path . --locked or download the ZIP",
            main_executable.display()
        );
    }
    let swift_helper = if swift_helper.is_file() {
        swift_helper
    } else {
        source_install_swift_helper()?
    };
    Ok((viewer, swift_helper))
}

/// Source installs embed the Swift executable built by build.rs. Extracting it
/// into a private cache requires no Swift interpreter or compiler at runtime.
#[cfg(target_os = "macos")]
fn source_install_swift_helper() -> Result<PathBuf> {
    use std::{fs, io::Write, os::unix::fs::PermissionsExt};

    const SWIFT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/doctty-webkit-swift"));
    let home = std::env::var_os("HOME").context("HOME is needed for the WebKit helper cache")?;
    let cache = PathBuf::from(home)
        .join("Library/Caches/doctty/native")
        .join(env!("CARGO_PKG_VERSION"));
    fs::create_dir_all(&cache).with_context(|| format!("create {}", cache.display()))?;
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700))?;
    let destination = cache.join("doctty-webkit-swift");
    // Verify actual bytes rather than only trusting a filename/version marker.
    if fs::read(&destination).ok().as_deref() == Some(SWIFT) {
        return Ok(destination);
    }

    let mut temp = tempfile::NamedTempFile::new_in(&cache)?;
    temp.write_all(SWIFT)?;
    temp.as_file().set_permissions(fs::Permissions::from_mode(0o700))?;
    temp.persist(&destination)
        .with_context(|| format!("install {}", destination.display()))?;
    Ok(destination)
}

#[cfg(all(test, not(target_os = "macos")))]
fn source_install_swift_helper() -> Result<PathBuf> {
    bail!("Swift/WebKit helper requires macOS")
}

pub(super) fn run(document: &Path, theme: Theme, watch: bool, mermaid: MermaidMode) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().context("unable to resolve doctty executable")?;
        // Fail clearly instead of falling back silently to interpreted Swift.
        let (viewer, swift_helper) = require_companions(&exe)?;
        let mut command = Command::new(&viewer);
        command
            .arg("--hybrid-image")
            .arg(document)
            .env("DOCTTY_WEBKIT_HELPER_BIN", &swift_helper)
            .env("DOCTTY_WEBKIT_THEME", theme.as_str())
            .env(
                "DOCTTY_WEBKIT_MERMAID",
                match mermaid {
                    MermaidMode::Enabled => "enabled",
                    MermaidMode::Disabled => "disabled",
                },
            );
        if watch {
            command.arg("--watch");
        }

        let status = command
            .status()
            .context("failed to start doctty renderer")?;
        if !status.success() {
            bail!("doctty renderer exited with {status}");
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (document, theme, watch, mermaid);
        bail!("the compiled renderer viewer requires macOS and the prebuilt renderer bundle");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companions_must_reside_beside_the_cli_even_in_paths_with_spaces() -> Result<()> {
        let root = tempfile::tempdir()?;
        let bin = root.path().join("application with spaces");
        std::fs::create_dir_all(&bin)?;
        let exe = bin.join("doctty");
        let (viewer, helper) = companion_paths(&exe);
        assert_eq!(viewer, bin.join("doctty-renderer"));
        assert_eq!(helper, bin.join("doctty-webkit-swift"));
        assert!(!viewer.exists());
        std::fs::write(&viewer, b"test placeholder")?;
        std::fs::write(&helper, b"test placeholder")?;
        assert_eq!(require_companions(&exe)?, (viewer, helper));
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn compiled_eco_requires_macos() {
        assert!(
            run(
                Path::new("README.md"),
                Theme::Light,
                false,
                MermaidMode::Disabled
            )
            .is_err_and(|error| error.to_string().contains("requires macOS"))
        );
    }
}
