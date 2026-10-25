use std::{
    io::{self, IsTerminal, Read, Write},
    path::Path,
};

use anyhow::{Result, bail};
use tracing::info_span;

mod renderer_companion;

use crate::{
    cli::DocttyArgs,
    core::config::AppConfig,
    io::fs::FileSystemDocumentSource,
    render::{markdown::parse_document, text::render_plain_text},
    support::tracing::init_tracing,
};

pub fn run(args: DocttyArgs) -> Result<()> {
    init_tracing();

    let _startup_span = info_span!("app.run").entered();
    let config = AppConfig::try_from(args)?;
    if config.path == Path::new("-") {
        return run_from_stdin(config);
    }

    let source = FileSystemDocumentSource::new(config.path.clone());
    let content = {
        let _span = info_span!("startup.read_source").entered();
        source.read_to_string()?
    };
    let document = {
        let _span = info_span!("startup.parse_document").entered();
        parse_document(config.path.clone(), &content)?
    };

    if io::stdout().is_terminal() && io::stdin().is_terminal() {
        if !crate::ui::terminal::is_supported_terminal(
            std::env::var("TERM_PROGRAM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        ) {
            bail!("interactive mode requires Ghostty or Kitty");
        }
        return renderer_companion::run(
            &config.path,
            config.theme,
            config.watch,
            config.mermaid_mode,
        );
    }

    let rendered = {
        let _span = info_span!("startup.render_plain_text").entered();
        render_plain_text(&document, config.theme, config.mermaid_mode)
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(rendered.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

fn run_from_stdin(config: AppConfig) -> Result<()> {
    if io::stdin().is_terminal() {
        bail!("'-' requires piped stdin");
    }

    let mut content = String::new();
    {
        let _span = info_span!("startup.read_stdin").entered();
        io::stdin().read_to_string(&mut content)?;
    }

    let document = {
        let _span = info_span!("startup.parse_document").entered();
        let virtual_path = std::env::current_dir()?.join("stdin.md");
        parse_document(virtual_path, &content)?
    };

    let rendered = {
        let _span = info_span!("startup.render_plain_text").entered();
        render_plain_text(&document, config.theme, config.mermaid_mode)
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(rendered.as_bytes())?;
    stdout.flush()?;
    Ok(())
}
