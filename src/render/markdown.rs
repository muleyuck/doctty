use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
};

use anyhow::Result;
use comrak::{
    Arena,
    nodes::{AlertType, AstNode, NodeCodeBlock, NodeLink, NodeList, NodeValue},
    parse_document as parse_comrak_document,
};

use super::markdown_pipeline::gfm_options;
use crate::core::document::{
    Block, BlockId, BlockKind, CalloutKind, Document, DocumentMeta, InlineSegment, InlineStyle,
    SourceSpan, StyledText,
};

pub fn parse_document(path: PathBuf, source: &str) -> Result<Document> {
    let arena = Arena::new();
    let root = parse_comrak_document(&arena, source, &gfm_options());
    let mut blocks = Vec::new();

    for (index, node) in root.children().enumerate() {
        if let Some(kind) = normalize_block(node) {
            let source_hash = hash_block(&kind);
            blocks.push(Block {
                id: BlockId(index),
                kind,
                span: SourceSpan::default(),
                source_hash,
            });
        }
    }

    let title = blocks.iter().find_map(|block| match &block.kind {
        BlockKind::Heading { text, .. } => Some(text.plain()),
        _ => None,
    });
    let mut links = Vec::new();
    collect_links(root, &mut links);

    Ok(Document {
        path,
        revision: 0,
        blocks,
        meta: DocumentMeta {
            title,
            links,
            source_len: source.len(),
        },
    })
}

fn normalize_block<'a>(node: &'a AstNode<'a>) -> Option<BlockKind> {
    match &node.data.borrow().value {
        NodeValue::Heading(heading) => Some(BlockKind::Heading {
            level: heading.level,
            text: collect_styled_text(node),
        }),
        NodeValue::Paragraph => normalize_paragraph(node),
        NodeValue::BlockQuote => Some(normalize_quote(node)),
        NodeValue::List(list) => Some(normalize_list(node, list)),
        NodeValue::CodeBlock(code) => Some(normalize_code_block(code)),
        NodeValue::HtmlBlock(block) => normalize_html_block(&block.literal),
        NodeValue::ThematicBreak => Some(BlockKind::Rule),
        NodeValue::Table(_) => Some(normalize_table(node)),
        NodeValue::FootnoteDefinition(definition) => Some(BlockKind::Footnote {
            label: definition.name.clone(),
            body: collect_styled_text(node),
        }),
        NodeValue::Alert(alert) => Some(BlockKind::Callout {
            kind: map_alert_kind(alert.alert_type),
            title: alert.title.clone().map(StyledText::from_plain),
            body: collect_styled_text(node),
        }),
        _ => None,
    }
}

fn normalize_paragraph<'a>(node: &'a AstNode<'a>) -> Option<BlockKind> {
    if let Some(image) = extract_image(node) {
        return Some(image);
    }

    let text = collect_styled_text(node);
    (!text.is_empty()).then_some(BlockKind::Paragraph { text })
}

fn normalize_quote<'a>(node: &'a AstNode<'a>) -> BlockKind {
    let text = collect_styled_text(node);
    if let Some((kind, body)) = parse_callout(&text.plain()) {
        return BlockKind::Callout {
            kind,
            title: None,
            body,
        };
    }

    BlockKind::BlockQuote { text }
}

fn normalize_list<'a>(node: &'a AstNode<'a>, list: &NodeList) -> BlockKind {
    let items = node
        .children()
        .map(collect_styled_text)
        .filter(|text| !text.is_empty())
        .collect();

    BlockKind::List {
        ordered: list.list_type == comrak::nodes::ListType::Ordered,
        items,
    }
}

fn normalize_code_block(code: &NodeCodeBlock) -> BlockKind {
    let language = code.info.trim().to_string();
    let literal = code.literal.clone();

    if language == "mermaid" {
        BlockKind::Mermaid { source: literal }
    } else {
        BlockKind::CodeFence {
            language: (!language.is_empty()).then_some(language),
            code: literal,
        }
    }
}

fn normalize_table<'a>(node: &'a AstNode<'a>) -> BlockKind {
    let rows = node
        .children()
        .map(|row| {
            row.children()
                .map(collect_styled_text)
                .filter(|cell| !cell.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
        .collect();

    BlockKind::Table { rows }
}

fn extract_image<'a>(node: &'a AstNode<'a>) -> Option<BlockKind> {
    let mut children = node.children();
    let image = children.next()?;
    if children.next().is_some() {
        return None;
    }

    match &image.data.borrow().value {
        NodeValue::Image(link) => Some(image_block(link, image)),
        _ => None,
    }
}

fn image_block<'a>(link: &NodeLink, image: &'a AstNode<'a>) -> BlockKind {
    BlockKind::Image {
        src: link.url.clone(),
        alt: collect_styled_text(image).plain(),
        title: (!link.title.is_empty()).then(|| link.title.clone()),
    }
}

fn parse_callout(text: &str) -> Option<(CalloutKind, StyledText)> {
    let trimmed = text.trim();
    let (tag, body) = trimmed.split_once(']')?;
    let kind = match tag {
        "[!NOTE" => CalloutKind::Note,
        "[!TIP" => CalloutKind::Tip,
        "[!IMPORTANT" => CalloutKind::Important,
        "[!WARNING" => CalloutKind::Warning,
        "[!CAUTION" => CalloutKind::Caution,
        _ => return None,
    };

    Some((kind, StyledText::from_plain(body.trim().to_string())))
}

fn collect_styled_text<'a>(node: &'a AstNode<'a>) -> StyledText {
    let mut segments = Vec::new();
    collect_text_segments(node, InlineStyle::default(), &mut segments);
    StyledText {
        segments: normalize_segments(segments),
    }
}

fn collect_text_segments<'a>(
    node: &'a AstNode<'a>,
    style: InlineStyle,
    parts: &mut Vec<InlineSegment>,
) {
    match &node.data.borrow().value {
        NodeValue::Text(text) => {
            let mut value = text.to_string();
            if parts.last().is_some_and(|part| part.text.ends_with('\n')) {
                value = value.trim_start_matches(' ').to_string();
            }
            if !value.is_empty() {
                parts.push(InlineSegment { text: value, style });
            }
        }
        NodeValue::Code(code) => {
            let value = code.literal.clone();
            if !value.is_empty() {
                parts.push(InlineSegment {
                    text: value,
                    style: InlineStyle {
                        code: true,
                        ..style
                    },
                });
            }
        }
        NodeValue::Math(math) => {
            let value = if math.display_math {
                format!("$${}$$", math.literal)
            } else if math.dollar_math {
                format!("${}$", math.literal)
            } else {
                format!("$`{}`$", math.literal)
            };
            if !value.is_empty() {
                parts.push(InlineSegment { text: value, style });
            }
        }
        NodeValue::HtmlInline(literal) => {
            let text = inline_html_text(literal);
            if text == "\n"
                && let Some(last) = parts.last_mut()
            {
                last.text.truncate(last.text.trim_end_matches(' ').len());
            }
            if !text.is_empty() {
                parts.push(InlineSegment { text, style });
            }
        }
        NodeValue::TaskItem(task) => {
            let marker = task
                .symbol
                .map_or("[ ] ".to_string(), |value| format!("[{value}] "));
            parts.push(InlineSegment {
                text: marker,
                style,
            });
            for child in node.children() {
                collect_text_segments(child, style, parts);
            }
        }
        NodeValue::FootnoteReference(reference) => {
            parts.push(InlineSegment {
                text: format!("[^{}]", reference.name),
                style,
            });
        }
        NodeValue::Link(link) => {
            for child in node.children() {
                collect_text_segments(child, style, parts);
            }
            if !link.url.is_empty() {
                parts.push(InlineSegment {
                    text: format!(" <{}>", link.url),
                    style,
                });
            }
        }
        NodeValue::Strong => {
            let style = InlineStyle {
                bold: true,
                ..style
            };
            for child in node.children() {
                collect_text_segments(child, style, parts);
            }
        }
        NodeValue::Emph => {
            let style = InlineStyle {
                italic: true,
                ..style
            };
            for child in node.children() {
                collect_text_segments(child, style, parts);
            }
        }
        NodeValue::LineBreak | NodeValue::SoftBreak => {
            parts.push(InlineSegment {
                text: "\n".to_string(),
                style,
            });
        }
        NodeValue::Image(_) => {
            for child in node.children() {
                collect_text_segments(child, style, parts);
            }
        }
        _ => {
            for child in node.children() {
                collect_text_segments(child, style, parts);
            }
        }
    }
}

fn hash_block(kind: &BlockKind) -> u64 {
    let mut hasher = DefaultHasher::new();
    kind.hash(&mut hasher);
    hasher.finish()
}

fn normalize_segments(parts: Vec<InlineSegment>) -> Vec<InlineSegment> {
    let mut normalized: Vec<InlineSegment> = Vec::new();

    for part in parts {
        if part.text.is_empty() {
            continue;
        }
        if let Some(previous) = normalized.last_mut()
            && previous.style == part.style
        {
            previous.text.push_str(&part.text);
            continue;
        }
        normalized.push(part);
    }

    normalized
}

fn normalize_html_block(literal: &str) -> Option<BlockKind> {
    if let Some(image) = extract_html_img(literal) {
        return Some(image);
    }

    let text = StyledText::from_plain(html_to_text(literal));
    (!text.is_empty()).then_some(BlockKind::Paragraph { text })
}

/// Comrak emits each inline tag as its own node, so only the tag itself is
/// handled here; the text between tags arrives as ordinary text nodes.
fn inline_html_text(literal: &str) -> String {
    match parse_html_tag(literal) {
        Some(tag) if tag.name == "br" => "\n".to_string(),
        Some(tag) if tag.name == "img" && !tag.closing => extract_html_attr(literal, "alt")
            .map(|alt| htmlize::unescape_attribute(alt).into_owned())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

struct HtmlTag {
    name: String,
    closing: bool,
    len: usize,
}

/// Parses the tag at the start of `html`. Comments, declarations and a
/// `<` that does not open a tag yield `None`.
fn parse_html_tag(html: &str) -> Option<HtmlTag> {
    let rest = html.strip_prefix('<')?;
    let (closing, rest) = match rest.strip_prefix('/') {
        Some(after) => (true, after),
        None => (false, rest),
    };
    if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(rest.len());
    let name = rest[..name_len].to_ascii_lowercase();

    let mut quote = None;
    for (offset, c) in rest[name_len..].char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => {
                let len = html.len() - rest.len() + name_len + offset + 1;
                return Some(HtmlTag { name, closing, len });
            }
            (None, _) => {}
        }
    }
    None
}

/// Plain-text rendering of an HTML block: tags are removed, their text is
/// kept, and block-level elements and `<br>` start a new line.
fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(index) = rest.find(|c: char| c == '<' || c.is_whitespace()) {
        text.push_str(&rest[..index]);
        rest = &rest[index..];

        if let Some(after) = rest.strip_prefix(char::is_whitespace) {
            push_html_space(&mut text);
            rest = after;
        } else if let Some(after_open) = rest.strip_prefix("<!--") {
            rest = after_open
                .find("-->")
                .map_or("", |end| &after_open[end + "-->".len()..]);
        } else if let Some(tag) = parse_html_tag(rest) {
            rest = &rest[tag.len..];
            if !tag.closing && matches!(tag.name.as_str(), "script" | "style") {
                let close = format!("</{}", tag.name);
                rest = rest
                    .to_ascii_lowercase()
                    .find(&close)
                    .and_then(|end| rest[end..].find('>').map(|gt| &rest[end + gt + 1..]))
                    .unwrap_or("");
            } else if is_html_line_break(&tag.name) {
                text.push('\n');
            } else if matches!(tag.name.as_str(), "td" | "th") {
                push_html_space(&mut text);
            }
        } else {
            text.push('<');
            rest = &rest[1..];
        }
    }
    text.push_str(rest);

    htmlize::unescape(text)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn push_html_space(text: &mut String) {
    if !text.ends_with([' ', '\n']) {
        text.push(' ');
    }
}

fn is_html_line_break(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "br"
            | "caption"
            | "center"
            | "dd"
            | "details"
            | "div"
            | "dl"
            | "dt"
            | "figcaption"
            | "figure"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "summary"
            | "table"
            | "tbody"
            | "tfoot"
            | "thead"
            | "tr"
            | "ul"
    )
}

fn extract_html_img(html: &str) -> Option<BlockKind> {
    let trimmed = html.trim();
    let img_start = trimmed.find("<img")?;
    let after_img = trimmed[img_start + 4..].trim();
    let img_end = after_img.find("/>").or_else(|| after_img.find('>'))?;
    let img_content = after_img[..img_end].trim();

    let src = extract_html_attr(img_content, "src")?;
    let alt = extract_html_attr(img_content, "alt").unwrap_or_default();
    let title = extract_html_attr(img_content, "title");

    Some(BlockKind::Image { src, alt, title })
}

fn extract_html_attr(html: &str, name: &str) -> Option<String> {
    let needle = format!(r#"{name}=""#);
    let start = html.find(&needle)? + needle.len();
    let end = html[start..].find('"')?;
    Some(html[start..start + end].to_string())
}

fn collect_links<'a>(node: &'a AstNode<'a>, links: &mut Vec<String>) {
    if let NodeValue::Link(link) = &node.data.borrow().value
        && !links.contains(&link.url)
    {
        links.push(link.url.clone());
    }

    for child in node.children() {
        collect_links(child, links);
    }
}

fn map_alert_kind(kind: AlertType) -> CalloutKind {
    match kind {
        AlertType::Note => CalloutKind::Note,
        AlertType::Tip => CalloutKind::Tip,
        AlertType::Important => CalloutKind::Important,
        AlertType::Warning => CalloutKind::Warning,
        AlertType::Caution => CalloutKind::Caution,
    }
}
