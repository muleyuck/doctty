use crate::{
    cli::Theme,
    core::{
        config::MermaidMode,
        document::{BlockKind, CalloutKind, Document, InlineStyle, StyledText},
    },
    io::{
        image_decoder::{self, ImageDecoder, LoadedImage},
        mermaid_cli::MermaidCliRenderer,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedDocument {
    pub lines: Vec<RenderedLine>,
    pub graphics: Vec<RenderedGraphic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedLine {
    pub plain_text: String,
    pub display_text: String,
    pub spans: Vec<RenderedInlineSegment>,
    pub kind: RenderedLineKind,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RenderedInlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

impl RenderedInlineStyle {
    pub const PLAIN: Self = Self {
        bold: false,
        italic: false,
        code: false,
    };
    pub const BOLD: Self = Self {
        bold: true,
        italic: false,
        code: false,
    };
    pub const ITALIC: Self = Self {
        bold: false,
        italic: true,
        code: false,
    };
    pub const CODE: Self = Self {
        bold: false,
        italic: false,
        code: true,
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedInlineSegment {
    pub text: String,
    pub style: RenderedInlineStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderedLineKind {
    Blank,
    Heading {
        level: u8,
    },
    Paragraph,
    List,
    Quote,
    Callout {
        kind: CalloutKind,
    },
    Code {
        language: Option<String>,
        is_fence_delimiter: bool,
    },
    Table,
    Rule,
    Meta,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedGraphic {
    pub line_index: usize,
    pub width_cells: u16,
    pub height_cells: u16,
    pub natural_width_px: u32,
    pub natural_height_px: u32,
    pub content: RenderedGraphicContent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderedGraphicContent {
    Png(Vec<u8>),
    Mermaid {
        source: String,
        png_bytes: Option<Vec<u8>>,
        failed: bool,
    },
}

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

#[must_use]
pub fn render_document(
    document: &Document,
    _theme: Theme,
    mermaid_mode: MermaidMode,
    width_cells: u16,
    cell_aspect_ratio: f32,
) -> RenderedDocument {
    let context = DisplayContext {
        document,
        mermaid_mode,
        cell_aspect_ratio,
        image_decoder: ImageDecoder::new(),
    };
    let mut rendered = RenderedDocument {
        lines: Vec::new(),
        graphics: Vec::new(),
    };
    let wrap_width = usize::from(width_cells.max(20));

    for block in &document.blocks {
        context.push_block(&mut rendered, &block.kind, wrap_width, width_cells);
        rendered.lines.push(blank_line());
    }

    rendered
}

struct DisplayContext<'a> {
    document: &'a Document,
    mermaid_mode: MermaidMode,
    cell_aspect_ratio: f32,
    image_decoder: ImageDecoder,
}

impl DisplayContext<'_> {
    /// Renders `blocks` narrower by the prefix width, then puts `first`
    /// before the first line and `rest` before the others. Paragraph lines
    /// take `kind`, so they are styled as part of their container. Rows an
    /// image is drawn over and code lines get no prefix: the terminal draws
    /// a code frame from the line's own text, and an empty closing delimiter
    /// is what draws its bottom border.
    fn push_nested(
        &self,
        out: &mut RenderedDocument,
        blocks: &[BlockKind],
        separated: bool,
        (first, rest): (&str, &str),
        kind: &RenderedLineKind,
        (wrap_width, width_cells): (usize, u16),
    ) {
        let prefix_width = first.chars().count().max(rest.chars().count());
        let mut nested = RenderedDocument {
            lines: Vec::new(),
            graphics: Vec::new(),
        };
        for (index, block) in blocks.iter().enumerate() {
            if separated && index > 0 {
                nested.lines.push(blank_line());
            }
            self.push_block(
                &mut nested,
                block,
                wrap_width.saturating_sub(prefix_width).max(1),
                width_cells
                    .saturating_sub(u16::try_from(prefix_width).unwrap_or(u16::MAX))
                    .max(1),
            );
        }
        if nested.lines.is_empty() {
            nested.lines.push(blank_line());
        }

        // A code line takes no prefix, so a list marker before code gets a
        // line of its own.
        let marker = first.trim_end();
        if !marker.is_empty() && matches!(nested.lines[0].kind, RenderedLineKind::Code { .. }) {
            out.lines.push(rendered_line(
                StyledText::from_plain(marker.to_string()),
                kind.clone(),
            ));
        }

        let offset = out.lines.len();
        let is_graphic_row = |row: usize| {
            nested.graphics.iter().any(|graphic| {
                (graphic.line_index..graphic.line_index + usize::from(graphic.height_cells))
                    .contains(&row)
            })
        };
        for (index, line) in nested.lines.iter().enumerate() {
            if is_graphic_row(index) || matches!(line.kind, RenderedLineKind::Code { .. }) {
                out.lines.push(line.clone());
                continue;
            }
            let prefix = if index == 0 { first } else { rest };
            let prefix = if line.plain_text.is_empty() {
                prefix.trim_end()
            } else {
                prefix
            };
            // A blank line inside a list stays blank; inside a quote or a
            // callout it keeps the container's bar.
            let line_kind = match line.kind {
                RenderedLineKind::Paragraph => kind.clone(),
                RenderedLineKind::Blank if *kind != RenderedLineKind::List => kind.clone(),
                ref other => other.clone(),
            };
            out.lines.push(prefix_line(line, prefix, line_kind));
        }
        out.graphics
            .extend(nested.graphics.into_iter().map(|graphic| RenderedGraphic {
                line_index: graphic.line_index + offset,
                ..graphic
            }));
    }

    fn push_block(
        &self,
        out: &mut RenderedDocument,
        kind: &BlockKind,
        wrap_width: usize,
        width_cells: u16,
    ) {
        let lines = &mut out.lines;
        match kind {
            BlockKind::Heading { level, text } => {
                push_wrapped_display_line(
                    lines,
                    display_inline_text(text),
                    RenderedLineKind::Heading { level: *level },
                    wrap_width,
                );
            }
            BlockKind::Paragraph { text } => {
                push_wrapped_display_line(
                    lines,
                    display_inline_text(text),
                    RenderedLineKind::Paragraph,
                    wrap_width,
                );
            }
            BlockKind::List {
                ordered,
                tight,
                items,
            } => {
                for (index, item) in items.iter().enumerate() {
                    if !tight && index > 0 {
                        out.lines.push(blank_line());
                    }
                    let marker = if *ordered {
                        format!("{}. ", index + 1)
                    } else {
                        "• ".to_string()
                    };
                    let indent = " ".repeat(marker.chars().count());
                    let item = with_checkbox(item);
                    self.push_nested(
                        out,
                        &item,
                        !tight,
                        (&marker, &indent),
                        &RenderedLineKind::List,
                        (wrap_width, width_cells),
                    );
                }
            }
            BlockKind::BlockQuote { blocks } => {
                self.push_nested(
                    out,
                    blocks,
                    true,
                    ("│ ", "│ "),
                    &RenderedLineKind::Quote,
                    (wrap_width, width_cells),
                );
            }
            BlockKind::Callout { kind, title, body } => {
                let heading = title
                    .clone()
                    .unwrap_or_else(|| StyledText::from_plain(kind.label().to_string()));
                push_wrapped_prefixed_line(
                    lines,
                    String::new(),
                    "  ".to_string(),
                    heading,
                    RenderedLineKind::Callout { kind: *kind },
                    wrap_width,
                );
                // Each block starts at the bar and only its wrapped lines are
                // indented, as a single-paragraph body always was.
                let kind = RenderedLineKind::Callout { kind: *kind };
                for (index, block) in body.iter().enumerate() {
                    if index > 0 {
                        out.lines.push(RenderedLine {
                            kind: kind.clone(),
                            ..blank_line()
                        });
                    }
                    self.push_nested(
                        out,
                        std::slice::from_ref(block),
                        false,
                        ("", "  "),
                        &kind,
                        (wrap_width, width_cells),
                    );
                }
            }
            BlockKind::CodeFence { language, code } => {
                let language = language.as_deref().map(normalize_language_token);
                let fence_label = language.clone().unwrap_or_else(|| "code".to_string());
                lines.push(RenderedLine {
                    plain_text: fence_label.clone(),
                    display_text: fence_label.clone(),
                    spans: spans_from_plain(fence_label),
                    kind: RenderedLineKind::Code {
                        language: language.clone(),
                        is_fence_delimiter: true,
                    },
                });
                for code_line in code.lines() {
                    lines.push(RenderedLine {
                        plain_text: code_line.to_owned(),
                        display_text: code_line.to_owned(),
                        spans: spans_from_plain(code_line.to_owned()),
                        kind: RenderedLineKind::Code {
                            language: language.clone(),
                            is_fence_delimiter: false,
                        },
                    });
                }
                lines.push(RenderedLine {
                    plain_text: String::new(),
                    display_text: String::new(),
                    spans: Vec::new(),
                    kind: RenderedLineKind::Code {
                        language,
                        is_fence_delimiter: true,
                    },
                });
            }
            BlockKind::Table { rows } => {
                for line in render_table_display(rows, wrap_width) {
                    lines.push(RenderedLine {
                        plain_text: line.clone(),
                        display_text: line.clone(),
                        spans: spans_from_plain(line.clone()),
                        kind: RenderedLineKind::Table,
                    });
                }
            }
            BlockKind::Image { alt, src, .. } => {
                let label = if alt.is_empty() { src } else { alt };
                match self
                    .image_decoder
                    .load_from_document(&self.document.path, src)
                {
                    Ok(image) => {
                        let display =
                            format!("Image: {} ({}x{})", label, image.width, image.height);
                        push_wrapped_display_line(
                            &mut out.lines,
                            StyledText::from_plain(display),
                            RenderedLineKind::Meta,
                            wrap_width,
                        );
                        push_graphic(
                            &mut out.lines,
                            &mut out.graphics,
                            width_cells,
                            self.cell_aspect_ratio,
                            &image,
                        );
                    }
                    Err(_) => {
                        push_wrapped_display_line(
                            lines,
                            StyledText::from_plain(format!("Image unavailable: {label}")),
                            RenderedLineKind::Meta,
                            wrap_width,
                        );
                    }
                }
            }
            BlockKind::Mermaid { source } => match self.mermaid_mode {
                MermaidMode::Disabled => {
                    push_wrapped_display_line(
                        lines,
                        StyledText::from_plain("Mermaid disabled".to_string()),
                        RenderedLineKind::Meta,
                        wrap_width,
                    );
                }
                MermaidMode::Enabled => {
                    push_wrapped_display_line(
                        &mut out.lines,
                        StyledText::from_plain("Mermaid diagram".to_string()),
                        RenderedLineKind::Meta,
                        wrap_width,
                    );
                    push_graphic_placeholder(
                        &mut out.lines,
                        &mut out.graphics,
                        width_cells,
                        scaled_graphic_height(width_cells, self.cell_aspect_ratio, 960, 540),
                        960,
                        540,
                        RenderedGraphicContent::Mermaid {
                            source: source.clone(),
                            png_bytes: None,
                            failed: false,
                        },
                    );
                }
            },
            BlockKind::Rule => lines.push(RenderedLine {
                plain_text: "─".repeat(wrap_width),
                display_text: "─".repeat(wrap_width),
                spans: spans_from_plain("─".repeat(wrap_width)),
                kind: RenderedLineKind::Rule,
            }),
            BlockKind::Footnote { label, body } => {
                let label = format!("[{label}] ");
                let indent = " ".repeat(label.chars().count());
                self.push_nested(
                    out,
                    body,
                    true,
                    (&label, &indent),
                    &RenderedLineKind::Meta,
                    (wrap_width, width_cells),
                );
            }
        }
    }
}

fn blank_line() -> RenderedLine {
    RenderedLine {
        plain_text: String::new(),
        display_text: String::new(),
        spans: Vec::new(),
        kind: RenderedLineKind::Blank,
    }
}

fn prefix_line(line: &RenderedLine, prefix: &str, kind: RenderedLineKind) -> RenderedLine {
    let mut spans = spans_from_plain(prefix.to_string());
    spans.extend(line.spans.iter().cloned());
    RenderedLine {
        plain_text: format!("{prefix}{}", line.plain_text),
        display_text: format!("{prefix}{}", line.display_text),
        spans,
        kind,
    }
}

/// Shows a task item's `[x] ` or `[ ] ` text as a checkbox.
fn with_checkbox(item: &[BlockKind]) -> Vec<BlockKind> {
    let mut item = item.to_vec();
    if let Some(BlockKind::Paragraph { text }) = item.first_mut() {
        *text = checkbox_text(text);
    }
    item
}

fn push_wrapped_display_line(
    lines: &mut Vec<RenderedLine>,
    text: StyledText,
    kind: RenderedLineKind,
    width: usize,
) {
    for segment in wrap_styled_text(&text, width) {
        lines.push(rendered_line(segment, kind.clone()));
    }
}

fn push_wrapped_prefixed_line(
    lines: &mut Vec<RenderedLine>,
    prefix: String,
    continuation_prefix: String,
    text: StyledText,
    kind: RenderedLineKind,
    width: usize,
) {
    let body_width = width
        .saturating_sub(
            prefix
                .chars()
                .count()
                .max(continuation_prefix.chars().count()),
        )
        .max(1);
    let segments = wrap_styled_text(&text, body_width);

    for (index, segment) in segments.into_iter().enumerate() {
        let current_prefix = if index == 0 {
            &prefix
        } else {
            &continuation_prefix
        };
        let prefixed = prepend_plain(segment, current_prefix);
        lines.push(rendered_line(prefixed, kind.clone()));
    }
}

fn render_table_display(rows: &[Vec<StyledText>], width: usize) -> Vec<String> {
    if rows.is_empty() {
        return vec!["(empty table)".to_string()];
    }

    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return vec!["(empty table)".to_string()];
    }

    let mut widths = vec![3_usize; columns];
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(display_inline_text(cell).plain().chars().count());
        }
    }

    while table_width(&widths) > width {
        let Some((index, cell_width)) = widths
            .iter_mut()
            .enumerate()
            .max_by_key(|(_, cell_width)| **cell_width)
        else {
            break;
        };
        if *cell_width <= 4 {
            break;
        }
        let _ = index;
        *cell_width -= 1;
    }

    let mut lines = Vec::new();
    lines.push(table_border('┌', '┬', '┐', &widths));
    lines.push(table_row(&rows[0], &widths));
    if rows.len() > 1 {
        lines.push(table_border('├', '┼', '┤', &widths));
        for row in rows.iter().skip(1) {
            lines.push(table_row(row, &widths));
        }
    }
    lines.push(table_border('└', '┴', '┘', &widths));
    lines
}

fn table_width(widths: &[usize]) -> usize {
    widths.iter().sum::<usize>() + widths.len() * 3 + 1
}

fn table_border(left: char, middle: char, right: char, widths: &[usize]) -> String {
    let mut line = String::new();
    line.push(left);
    for (index, width) in widths.iter().enumerate() {
        line.push_str(&"─".repeat(*width + 2));
        line.push(if index + 1 == widths.len() {
            right
        } else {
            middle
        });
    }
    line
}

fn table_row(row: &[StyledText], widths: &[usize]) -> String {
    let mut line = String::from("│");
    for (index, width) in widths.iter().enumerate() {
        let cell = row.get(index).cloned().unwrap_or_default();
        let display = truncate_to_width(&display_inline_text(&cell).plain(), *width);
        line.push(' ');
        line.push_str(&display);
        line.push_str(&" ".repeat(width.saturating_sub(display.chars().count()) + 1));
        line.push('│');
    }
    line
}

/// An item that starts with a block other than a paragraph, such as an
/// image, has a paragraph holding only `[x]` or `[ ]`.
fn checkbox_text(text: &StyledText) -> StyledText {
    if let [segment] = text.segments.as_slice()
        && segment.style == InlineStyle::default()
    {
        match segment.text.as_str() {
            "[x]" => return StyledText::from_plain("☑"),
            "[ ]" => return StyledText::from_plain("☐"),
            _ => {}
        }
    }
    if let Some(rest) = strip_leading_plain_prefix(text, "[x] ") {
        return prepend_plain(rest, "☑ ");
    }
    if let Some(rest) = strip_leading_plain_prefix(text, "[ ] ") {
        return prepend_plain(rest, "☐ ");
    }
    text.clone()
}

fn display_inline_text(text: &StyledText) -> StyledText {
    StyledText {
        segments: text
            .segments
            .iter()
            .filter_map(|segment| {
                let mut segment = segment.clone();
                if segment.style == InlineStyle::default() {
                    segment.text = strip_link_destinations(&segment.text);
                }
                (!segment.text.is_empty()).then_some(segment)
            })
            .collect(),
    }
}

fn truncate_to_width(text: &str, width: usize) -> String {
    text.chars().take(width.max(1)).collect()
}

fn push_graphic(
    lines: &mut Vec<RenderedLine>,
    graphics: &mut Vec<RenderedGraphic>,
    width_cells: u16,
    cell_aspect_ratio: f32,
    image: &LoadedImage,
) {
    let content_width = width_cells.saturating_sub(2).max(1);
    let height_cells =
        scaled_graphic_height(content_width, cell_aspect_ratio, image.width, image.height);
    push_graphic_placeholder(
        lines,
        graphics,
        content_width,
        height_cells,
        image.width,
        image.height,
        RenderedGraphicContent::Png(image.png_bytes.clone()),
    );
}

fn push_graphic_placeholder(
    lines: &mut Vec<RenderedLine>,
    graphics: &mut Vec<RenderedGraphic>,
    width_cells: u16,
    height_cells: u16,
    natural_width_px: u32,
    natural_height_px: u32,
    content: RenderedGraphicContent,
) {
    let line_index = lines.len();
    for _ in 0..height_cells {
        lines.push(RenderedLine {
            plain_text: String::new(),
            display_text: String::new(),
            spans: Vec::new(),
            kind: RenderedLineKind::Blank,
        });
    }
    graphics.push(RenderedGraphic {
        line_index,
        width_cells,
        height_cells,
        natural_width_px,
        natural_height_px,
        content,
    });
}

#[must_use]
pub(crate) fn scaled_graphic_height(
    width_cells: u16,
    cell_aspect_ratio: f32,
    width: u32,
    height: u32,
) -> u16 {
    let aspect = height as f32 / width.max(1) as f32;
    let cell_aspect_ratio = if cell_aspect_ratio.is_finite() && cell_aspect_ratio > 0.0 {
        cell_aspect_ratio
    } else {
        0.5
    };
    let height_cells = (width_cells as f32 * aspect * cell_aspect_ratio).ceil() as u16;
    height_cells.clamp(1, 18)
}

fn normalize_language_token(language: &str) -> String {
    language.trim().to_ascii_lowercase()
}

fn rendered_line(text: StyledText, kind: RenderedLineKind) -> RenderedLine {
    let plain_text = text.plain();
    let spans = rendered_segments(&text);
    RenderedLine {
        plain_text: plain_text.clone(),
        display_text: plain_text,
        spans,
        kind,
    }
}

fn rendered_segments(text: &StyledText) -> Vec<RenderedInlineSegment> {
    text.segments
        .iter()
        .filter(|segment| !segment.text.is_empty())
        .map(|segment| RenderedInlineSegment {
            text: segment.text.clone(),
            style: rendered_style(segment.style),
        })
        .collect()
}

fn rendered_style(style: InlineStyle) -> RenderedInlineStyle {
    RenderedInlineStyle {
        bold: style.bold,
        italic: style.italic,
        code: style.code,
    }
}

fn spans_from_plain(text: String) -> Vec<RenderedInlineSegment> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![RenderedInlineSegment {
            text,
            style: RenderedInlineStyle::PLAIN,
        }]
    }
}

fn prepend_plain(text: StyledText, prefix: &str) -> StyledText {
    if prefix.is_empty() {
        return text;
    }

    let mut segments = Vec::with_capacity(text.segments.len() + 1);
    segments.push(crate::core::document::InlineSegment {
        text: prefix.to_string(),
        style: InlineStyle::default(),
    });
    segments.extend(text.segments);
    StyledText { segments }
}

fn strip_leading_plain_prefix(text: &StyledText, prefix: &str) -> Option<StyledText> {
    let mut segments = text.segments.clone();
    let first = segments.first_mut()?;
    if first.style != InlineStyle::default() || !first.text.starts_with(prefix) {
        return None;
    }
    first.text = first.text[prefix.len()..].to_string();
    if first.text.is_empty() {
        segments.remove(0);
    }
    Some(StyledText { segments })
}

fn wrap_styled_text(text: &StyledText, width: usize) -> Vec<StyledText> {
    if text.is_empty() {
        return vec![StyledText::default()];
    }

    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current: Vec<crate::core::document::InlineSegment> = Vec::new();
    let mut current_width = 0usize;
    let mut needs_space = false;

    let push_current = |lines: &mut Vec<StyledText>,
                        current: &mut Vec<crate::core::document::InlineSegment>,
                        current_width: &mut usize,
                        needs_space: &mut bool| {
        if current.is_empty() {
            lines.push(StyledText::default());
        } else {
            lines.push(StyledText {
                segments: std::mem::take(current),
            });
        }
        *current_width = 0;
        *needs_space = false;
    };

    for segment in &text.segments {
        if segment.text.is_empty() {
            continue;
        }

        if segment.style.code {
            let parts = segment.text.split('\n').collect::<Vec<_>>();
            for (index, part) in parts.iter().enumerate() {
                if index > 0 {
                    push_current(
                        &mut lines,
                        &mut current,
                        &mut current_width,
                        &mut needs_space,
                    );
                }
                if part.is_empty() {
                    continue;
                }
                let part_width = part.chars().count();
                if needs_space && current_width + 1 + part_width > width && !current.is_empty() {
                    push_current(
                        &mut lines,
                        &mut current,
                        &mut current_width,
                        &mut needs_space,
                    );
                }
                if needs_space {
                    current.push(crate::core::document::InlineSegment {
                        text: " ".to_string(),
                        style: InlineStyle::default(),
                    });
                    current_width += 1;
                }
                current.push(crate::core::document::InlineSegment {
                    text: (*part).to_string(),
                    style: segment.style,
                });
                current_width += part_width;
                needs_space = true;
            }
            continue;
        }

        let logical_lines = segment.text.split('\n').collect::<Vec<_>>();
        for (line_index, logical_line) in logical_lines.iter().enumerate() {
            for word in logical_line.split_whitespace() {
                let word_width = word.chars().count();
                let separator_width = usize::from(needs_space && !is_attached_punctuation(word));
                if needs_space
                    && current_width + separator_width + word_width > width
                    && !current.is_empty()
                {
                    push_current(
                        &mut lines,
                        &mut current,
                        &mut current_width,
                        &mut needs_space,
                    );
                }

                if needs_space && !is_attached_punctuation(word) {
                    current.push(crate::core::document::InlineSegment {
                        text: " ".to_string(),
                        style: InlineStyle::default(),
                    });
                    current_width += 1;
                }

                let chunk = truncate_to_width(word, width);
                current.push(crate::core::document::InlineSegment {
                    text: chunk.clone(),
                    style: segment.style,
                });
                current_width += chunk.chars().count();
                needs_space = true;
            }

            if line_index + 1 != logical_lines.len() {
                push_current(
                    &mut lines,
                    &mut current,
                    &mut current_width,
                    &mut needs_space,
                );
            }
        }
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(StyledText { segments: current });
    }

    lines
}

fn strip_link_destinations(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find(" <") {
        output.push_str(&rest[..start]);
        let candidate = &rest[start + 2..];
        let Some(end_offset) = candidate.find('>') else {
            output.push_str(&rest[start..]);
            return output;
        };
        if !candidate[..end_offset].contains("://") {
            output.push_str(&rest[start..start + 2 + end_offset + 1]);
        }
        rest = &candidate[end_offset + 1..];
    }

    output.push_str(rest);
    output
}

fn is_attached_punctuation(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|ch| matches!(ch, ',' | '.' | ';' | ':' | '!' | '?' | ')' | ']' | '}'))
}
