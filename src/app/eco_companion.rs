//! Opt-in Eco bridge for the *bundled* persistent hybrid viewer.
//!
//! The full Eco renderer is currently an experimental, separate Rust
//! executable. Never invoke Cargo/Swift compilers on an end-user machine.

use std::path::Path;

use anyhow::{Result, bail};

use crate::cli::Theme;

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
        directory.join("webkit_viewport_poc"),
        directory.join("doctty-webkit-swift"),
    )
}

pub(super) fn run(document: &Path, theme: Theme) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().context("unable to resolve doctty executable")?;
        let (viewer, swift_helper): (PathBuf, PathBuf) = companion_paths(&exe);
        if !viewer.is_file() || !swift_helper.is_file() {
            bail!(
                "experimental Eco mode requires the prebuilt macOS bundle:                  missing webkit_viewport_poc or doctty-webkit-swift beside {}",
                exe.display()
            );
        }

        let status = Command::new(&viewer)
            .arg("--hybrid-image")
            .arg(document)
            .env("DOCTTY_WEBKIT_HELPER_BIN", &swift_helper)
            .env("DOCTTY_WEBKIT_THEME", theme.as_str())
            .status()
            .context("failed to start bundled Eco viewer")?;

        if !status.success() {
            bail!("bundled Eco viewer exited with {status}");
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (document, theme);
        bail!("experimental Eco mode requires macOS and the prebuilt Eco bundle");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companion_names_are_relative_to_the_doctty_executable() {
        let (viewer, helper) = companion_paths(Path::new("/some location/bin/doctty"));
        assert_eq!(viewer, Path::new("/some location/bin/webkit_viewport_poc"));
        assert_eq!(helper, Path::new("/some location/bin/doctty-webkit-swift"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn experimental_eco_requires_macos() {
        assert!(
            run(Path::new("README.md"), Theme::Light)
                .is_err_and(|error| error.to_string().contains("requires macOS"))
        );
    }
}
