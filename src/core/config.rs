use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::cli::{DocttyArgs, Theme};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppConfig {
    pub path: PathBuf,
    pub watch: bool,
    pub theme: Theme,
    pub mermaid_mode: MermaidMode,
}

impl TryFrom<DocttyArgs> for AppConfig {
    type Error = anyhow::Error;

    fn try_from(value: DocttyArgs) -> Result<Self> {
        let DocttyArgs { command, path, watch, theme, no_mermaid, .. } = value;

        if command.is_some() {
            bail!("view configuration cannot be built from a subcommand")
        }

        let Some(path) = path else { bail!("doctty requires a Markdown file path") };

        let mermaid_mode = if no_mermaid { MermaidMode::Disabled } else { MermaidMode::Enabled };

        Ok(Self { path, watch, theme: theme.resolve(), mermaid_mode })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MermaidMode {
    Enabled,
    Disabled,
}
