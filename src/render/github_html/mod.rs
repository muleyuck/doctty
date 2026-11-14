use std::path::Path;

use anyhow::{Context, Result};
use comrak::{
    Arena, create_formatter, html::format_node_default, nodes::NodeValue, options::Plugins,
    parse_document,
};
use tracing::info_span;

use crate::{
    cli::Theme,
    core::config::MermaidMode,
    io::mermaid_cli::MermaidCliRenderer,
    render::{
        markdown::first_link, markdown_pipeline::gfm_options, raw_html::is_html_source_block,
    },
};

mod mermaid;
mod postprocess;
mod styles;
#[cfg(test)]
mod tests;

use self::{
    mermaid::replace_mermaid_code_blocks,
    postprocess::{
        decorate_code_blocks, inject_alert_icons, restore_supported_raw_html, retint_code_tokens,
        write_html_source_block,
    },
    styles::{directory_url, syntax_adapter, theme_styles},
};

create_formatter!(GithubHtmlFormatter, {
    NodeValue::HtmlBlock(ref block) => |context, node, entering| {
        if !is_html_source_block(&block.literal) {
            return format_node_default(context, node, entering);
        }
        if entering {
            context.cr()?;
            write_html_source_block(context, &block.literal)?;
            context.cr()?;
        }
    },
});

/// A rendered GitHub-style page and the first link of its source, taken from
/// the same parse.
pub struct GithubPage {
    pub html: String,
    pub first_link: Option<String>,
}

pub fn build_github_html(
    source: &str,
    base_dir: &Path,
    theme: Theme,
    mermaid_mode: MermaidMode,
) -> Result<String> {
    Ok(build_github_page(source, base_dir, theme, mermaid_mode)?.html)
}

pub fn build_github_page(
    source: &str,
    base_dir: &Path,
    theme: Theme,
    mermaid_mode: MermaidMode,
) -> Result<GithubPage> {
    let _span = info_span!("github_html.build").entered();
    let mut options = gfm_options();
    options.render.escape = true;
    options.render.r#unsafe = false;
    options.render.tasklist_classes = true;
    options.render.github_pre_lang = true;

    let arena = Arena::new();
    let root = parse_document(&arena, source, &options);
    // Before the Mermaid rewrite, so the AST is exactly what `gfm_options`
    // parses; the render-only options set above do not change parsing.
    let first_link = first_link(root);
    replace_mermaid_code_blocks(root, mermaid_mode, theme, &MermaidCliRenderer::from_env())?;

    let mut plugins = Plugins::default();
    plugins.render.codefence_syntax_highlighter = Some(syntax_adapter(theme));

    let mut rendered = String::new();
    GithubHtmlFormatter::format_document_with_plugins(root, &options, &mut rendered, &plugins)
        .context("formatting markdown to html failed")?;

    let body_html = restore_supported_raw_html(&decorate_code_blocks(&retint_code_tokens(
        &inject_alert_icons(&rendered),
        theme,
    )));
    let styles = theme_styles(theme);

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en" data-color-mode="{color_mode}" data-dark-theme="dark" data-light-theme="light">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <base href="{base_href}" />
  <style>{styles}</style>
</head>
<body>
  <div class="doctty-page">
    <article class="markdown-body entry-content container-lg">
      {body_html}
    </article>
  </div>
</body>
</html>"#,
        color_mode = theme.as_str(),
        base_href = directory_url(base_dir),
        styles = styles,
    );
    Ok(GithubPage { html, first_link })
}
