use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Clone, Debug, Eq, Parser, PartialEq)]
#[command(
    name = "doctty",
    about = "High-fidelity Markdown viewer for Ghostty and Kitty",
    arg_required_else_help = true
)]
pub struct DocttyArgs {
    /// Markdown file to open.
    pub path: Option<PathBuf>,

    /// Reload the document when the file changes.
    #[arg(long)]
    pub watch: bool,

    /// Use the experimental bundled low-CPU Eco WebKit viewer (macOS).
    /// Requires a separately packaged companion executable.
    #[arg(long, hide = true)]
    pub experimental_eco: bool,

    /// Color theme used by the viewer.
    #[arg(long, value_enum, default_value_t = Theme::System)]
    pub theme: Theme,

    /// Disable Mermaid rendering and show placeholders instead.
    #[arg(long = "no-mermaid")]
    pub no_mermaid: bool,

    /// Print the current version.
    #[arg(short = 'V', long = "version", global = true)]
    pub version: bool,
}

impl DocttyArgs {
    #[must_use]
    pub fn parse_from<I, T>(itr: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::parse_from(itr)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    #[must_use]
    pub fn resolve(self) -> Self {
        match self {
            Self::System => match dark_light::detect() {
                dark_light::Mode::Dark => Self::Dark,
                _ => Self::Light,
            },
            theme => theme,
        }
    }
}
