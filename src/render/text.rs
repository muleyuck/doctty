use crate::{
    cli::Theme,
    core::{
        config::MermaidMode,
        document::{BlockKind, Document, StyledText},
    },
    io::{
        image_decoder::{self, ImageDecoder},
        mermaid_cli::MermaidCliRenderer,
    },
};

#[must_use]
pub fn render_plain_text(document: &Document, theme: Theme, mermaid_mode: MermaidMode) -> String {
    let context = PlainTextContext {
        document,
        theme,
        mermaid_mode,
        image_decoder: ImageDecoder::new(),
        mermaid_renderer: MermaidCliRenderer::from_env(),
    };
    let mut lines = Vec::new();

    for block in &document.blocks {
        context.push_block(&mut lines, &block.kind);
        lines.push(String::new());
    }

    lines.join("\n")
}

struct PlainTextContext<'a> {
    document: &'a Document,
    theme: Theme,
    mermaid_mode: MermaidMode,
    image_decoder: ImageDecoder,
    mermaid_renderer: MermaidCliRenderer,
}

impl PlainTextContext<'_> {
    fn push_blocks(&self, lines: &mut Vec<String>, blocks: &[BlockKind], separated: bool) {
        for (index, block) in blocks.iter().enumerate() {
            if separated && index > 0 {
                lines.push(String::new());
            }
            self.push_block(lines, block);
        }
    }

    /// Renders `blocks` and prints the first line after `first` and the
    /// others after `rest`, like a list marker and its indentation.
    fn push_nested(
        &self,
        lines: &mut Vec<String>,
        blocks: &[BlockKind],
        separated: bool,
        first: &str,
        rest: &str,
    ) {
        let mut nested = Vec::new();
        self.push_blocks(&mut nested, blocks, separated);
        if nested.is_empty() {
            lines.push(first.trim_end().to_string());
        }
        for (index, line) in nested.into_iter().enumerate() {
            let prefix = if index == 0 { first } else { rest };
            lines.push(if line.is_empty() {
                prefix.trim_end().to_string()
            } else {
                format!("{prefix}{line}")
            });
        }
    }

    fn push_block(&self, lines: &mut Vec<String>, kind: &BlockKind) {
        match kind {
            BlockKind::Heading { level, text } => {
                lines.push(format!("{} {}", "#".repeat((*level).into()), text.plain()));
            }
            BlockKind::Paragraph { text } => {
                lines.extend(text.plain().lines().map(ToOwned::to_owned));
            }
            BlockKind::List {
                ordered,
                tight,
                items,
            } => {
                for (index, item) in items.iter().enumerate() {
                    if !tight && index > 0 {
                        lines.push(String::new());
                    }
                    let marker = if *ordered {
                        format!("{}. ", index + 1)
                    } else {
                        "- ".to_string()
                    };
                    let indent = " ".repeat(marker.len());
                    self.push_nested(lines, item, !tight, &marker, &indent);
                }
            }
            BlockKind::BlockQuote { blocks } => {
                self.push_nested(lines, blocks, true, "> ", "> ");
            }
            BlockKind::Callout { kind, body, .. } => {
                let label = format!("[{}] ", kind.label());
                let indent = " ".repeat(label.chars().count());
                self.push_nested(lines, body, true, &label, &indent);
            }
            BlockKind::CodeFence { language, code } => {
                lines.push(
                    language
                        .as_ref()
                        .map_or_else(|| "```".to_string(), |value| format!("```{value}")),
                );
                lines.extend(code.lines().map(ToOwned::to_owned));
                lines.push("```".to_string());
            }
            BlockKind::Table { rows } => {
                for row in rows {
                    lines.push(format!(
                        "| {} |",
                        row.iter()
                            .map(StyledText::plain)
                            .collect::<Vec<_>>()
                            .join(" | ")
                    ));
                }
            }
            BlockKind::Image { alt, src, .. } => {
                let label = if alt.is_empty() { src } else { alt };
                match self
                    .image_decoder
                    .load_from_document(&self.document.path, src)
                {
                    Ok(image) => lines.push(format!(
                        "[Image: {label} {}x{} -> {}]",
                        image.width,
                        image.height,
                        image.path.display()
                    )),
                    Err(_) => {
                        let image_path =
                            image_decoder::resolve_image_path(&self.document.path, src);
                        lines.push(format!(
                            "[Image missing: {label} -> {}]",
                            image_path.display()
                        ));
                    }
                }
            }
            BlockKind::Mermaid { source } => match self.mermaid_mode {
                MermaidMode::Disabled => lines.push("[Mermaid disabled]".to_string()),
                MermaidMode::Enabled => {
                    match self.mermaid_renderer.render_png(source, self.theme) {
                        Ok(png_bytes) => {
                            match self.image_decoder.dimensions_from_png_bytes(&png_bytes) {
                                Ok((width, height)) => {
                                    lines.push(format!("[Mermaid rendered: {width}x{height}]"));
                                }
                                Err(_) => lines.push(
                                    "[Mermaid unavailable: invalid renderer output]".to_string(),
                                ),
                            }
                        }
                        Err(_) => {
                            lines
                                .push("[Mermaid unavailable: renderer not configured]".to_string());
                        }
                    }
                }
            },
            BlockKind::Rule => lines.push("---".to_string()),
            BlockKind::Footnote { label, body } => {
                let label = format!("[^{label}] ");
                let indent = " ".repeat(label.chars().count());
                self.push_nested(lines, body, true, &label, &indent);
            }
        }
    }
}
