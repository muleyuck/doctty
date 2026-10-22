//! Version-matched, CI-compiled Eco companion for macOS interactive sessions.
//!
//! No Swift interpreter, compiler, Cargo or runtime source build is used on
//! the default path. The old renderer remains explicitly opt-in for developers.

use std::path::Path;

use anyhow::{Result, bail};

use crate::{cli::Theme, core::config::MermaidMode};

#[cfg(target_os = "macos")]
use anyhow::Context;
#[cfg(any(target_os = "macos", test))]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;

/// Select Eco by default for interactive macOS; leave non-macOS routing
/// unchanged. The headless/pipe branch does not call this backend.
pub(super) const fn use_eco(explicit: bool, legacy: bool) -> bool {
    explicit || (cfg!(target_os = "macos") && !legacy)
}

#[cfg(any(target_os = "macos", test))]
fn companion_paths(main_executable: &Path) -> (PathBuf, PathBuf) {
    let directory = main_executable.parent().unwrap_or_else(|| Path::new("."));
    (
        directory.join("webkit_viewport_poc"),
        directory.join("doctty-webkit-swift"),
    )
}

#[cfg(any(target_os = "macos", test))]
fn require_companions(main_executable: &Path) -> Result<(PathBuf, PathBuf)> {
    let (viewer, swift_helper) = companion_paths(main_executable);
    if !viewer.is_file() || !swift_helper.is_file() {
        bail!(
            "the prebuilt macOS Eco bundle is incomplete: expected \
             webkit_viewport_poc and doctty-webkit-swift beside {}. \
             Download the complete CI-built macOS archive and keep its \
             executables together; for source development only, use \
             --legacy-renderer (requires Swift tooling)",
            main_executable.display()
        );
    }
    Ok((viewer, swift_helper))
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
            .context("failed to start bundled Eco viewer")?;
        if !status.success() {
            bail!("bundled Eco viewer exited with {status}");
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (document, theme, watch, mermaid);
        bail!("the compiled Eco viewer requires macOS and the prebuilt Eco bundle");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_is_compiled_eco_only_on_macos() {
        assert_eq!(use_eco(false, false), cfg!(target_os = "macos"));
        assert!(!use_eco(false, true));
        assert!(use_eco(true, false));
    }

    #[test]
    fn companions_must_reside_beside_the_cli_even_in_paths_with_spaces() -> Result<()> {
        let root = tempfile::tempdir()?;
        let bin = root.path().join("application with spaces");
        std::fs::create_dir_all(&bin)?;
        let exe = bin.join("doctty");
        let (viewer, helper) = companion_paths(&exe);
        assert_eq!(viewer, bin.join("webkit_viewport_poc"));
        assert_eq!(helper, bin.join("doctty-webkit-swift"));
        assert!(require_companions(&exe).is_err());
        std::fs::write(&viewer, b"test placeholder")?;
        assert!(require_companions(&exe).is_err());
        std::fs::write(&helper, b"test placeholder")?;
        assert_eq!(require_companions(&exe)?, (viewer, helper));
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn compiled_eco_requires_macos() {
        assert!(run(Path::new("README.md"), Theme::Light, false, MermaidMode::Disabled)
            .is_err_and(|error| error.to_string().contains("requires macOS")));
    }
}
