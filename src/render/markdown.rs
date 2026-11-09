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

use super::{
    markdown_pipeline::gfm_options,
    raw_html::{InlineHtmlState, collect_inline_html, html_to_text},
};
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
            text: collect_single_line_text(node),
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
            body: normalize_children(node),
        }),
        NodeValue::Alert(alert) => Some(BlockKind::Callout {
            kind: map_alert_kind(alert.alert_type),
            title: alert.title.clone().map(StyledText::from_plain),
            body: normalize_children(node),
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

fn normalize_children<'a>(node: &'a AstNode<'a>) -> Vec<BlockKind> {
    node.children().filter_map(normalize_block).collect()
}

fn normalize_quote<'a>(node: &'a AstNode<'a>) -> BlockKind {
    let mut blocks = normalize_children(node);
    if let Some(BlockKind::Paragraph { text }) = blocks.first()
        && let Some((kind, body)) = parse_callout(&text.plain())
    {
        if body.is_empty() {
            blocks.remove(0);
        } else {
            blocks[0] = BlockKind::Paragraph { text: body };
        }
        return BlockKind::Callout {
            kind,
            title: None,
            body: blocks,
        };
    }

    BlockKind::BlockQuote { blocks }
}

fn normalize_list<'a>(node: &'a AstNode<'a>, list: &NodeList) -> BlockKind {
    let items = node
        .children()
        .map(normalize_list_item)
        .filter(|blocks| !blocks.is_empty())
        .collect();

    BlockKind::List {
        ordered: list.list_type == comrak::nodes::ListType::Ordered,
        tight: list.tight,
        items,
    }
}

/// A task item's checkbox is kept as `[x] ` text at the start of the item,
/// where the views turn it into a checkbox.
fn normalize_list_item<'a>(item: &'a AstNode<'a>) -> Vec<BlockKind> {
    let mut blocks = normalize_children(item);
    let NodeValue::TaskItem(task) = &item.data.borrow().value else {
        return blocks;
    };
    let marker = task
        .symbol
        .map_or("[ ] ".to_string(), |value| format!("[{value}] "));
    match blocks.first_mut() {
        Some(BlockKind::Paragraph { text }) => {
            let mut segments = vec![InlineSegment {
                text: marker,
                style: InlineStyle::default(),
            }];
            segments.append(&mut text.segments);
            text.segments = normalize_segments(segments);
        }
        _ => blocks.insert(
            0,
            BlockKind::Paragraph {
                text: StyledText::from_plain(marker.trim_end().to_string()),
            },
        ),
    }
    blocks
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
                .map(collect_single_line_text)
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.iter().all(StyledText::is_empty))
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
    collect_text_segments(
        node,
        InlineStyle::default(),
        &mut segments,
        &mut InlineHtmlState::default(),
    );
    StyledText {
        segments: normalize_segments(segments),
    }
}

/// Headings and table cells are printed on one line, so line breaks inside
/// them (such as `<br>`) become spaces.
fn collect_single_line_text<'a>(node: &'a AstNode<'a>) -> StyledText {
    let mut text = collect_styled_text(node);
    for segment in &mut text.segments {
        segment.text = segment.text.replace('\n', " ");
    }
    // Code spans keep their spaces, as the author wrote them on purpose.
    let mut leading_blank = 0;
    for segment in &mut text.segments {
        if segment.style.code {
            break;
        }
        segment.text = segment.text.trim_ascii_start().to_string();
        if !segment.text.is_empty() {
            break;
        }
        leading_blank += 1;
    }
    text.segments.drain(..leading_blank);
    while let Some(last) = text.segments.last_mut()
        && !last.style.code
    {
        last.text.truncate(last.text.trim_ascii_end().len());
        if !last.text.is_empty() {
            break;
        }
        text.segments.pop();
    }
    text
}

fn collect_text_segments<'a>(
    node: &'a AstNode<'a>,
    style: InlineStyle,
    parts: &mut Vec<InlineSegment>,
    html: &mut InlineHtmlState,
) {
    // Containers pass on only their Markdown style, so that an HTML style tag
    // closed inside one, as in `<b>a *b</b> c*`, ends where it is closed.
    let leaf_style = html.style(style);
    match &node.data.borrow().value {
        NodeValue::Text(text) => {
            let value = if html.follows_line_break(parts) {
                text.trim_start_matches(' ')
            } else {
                text
            };
            if !value.is_empty() {
                parts.push(InlineSegment {
                    text: value.to_string(),
                    style: leaf_style,
                });
            }
        }
        NodeValue::Code(code) => {
            let value = code.literal.clone();
            if !value.is_empty() {
                parts.push(InlineSegment {
                    text: value,
                    style: InlineStyle {
                        code: true,
                        ..leaf_style
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
                parts.push(InlineSegment {
                    text: value,
                    style: leaf_style,
                });
            }
        }
        NodeValue::HtmlInline(literal) => collect_inline_html(literal, style, parts, html),
        NodeValue::FootnoteReference(reference) => {
            parts.push(InlineSegment {
                text: format!("[^{}]", reference.name),
                style: leaf_style,
            });
        }
        NodeValue::Link(link) => {
            for child in node.children() {
                collect_text_segments(child, style, parts, html);
            }
            if !link.url.is_empty() {
                parts.push(InlineSegment {
                    text: format!(" <{}>", link.url),
                    style: html.style(style),
                });
            }
        }
        NodeValue::Strong => {
            let style = InlineStyle {
                bold: true,
                ..style
            };
            for child in node.children() {
                collect_text_segments(child, style, parts, html);
            }
        }
        NodeValue::Emph => {
            let style = InlineStyle {
                italic: true,
                ..style
            };
            for child in node.children() {
                collect_text_segments(child, style, parts, html);
            }
        }
        NodeValue::LineBreak | NodeValue::SoftBreak => {
            if html.follows_block_break(parts) {
                return;
            }
            parts.push(InlineSegment {
                text: "\n".to_string(),
                style: leaf_style,
            });
        }
        NodeValue::Image(_) => {
            for child in node.children() {
                collect_text_segments(child, style, parts, html);
            }
        }
        _ => {
            for child in node.children() {
                collect_text_segments(child, style, parts, html);
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
    let Some(converted) = html_to_text(literal) else {
        let source = literal.trim_end_matches(['\r', '\n']);
        return (!source.trim().is_empty()).then(|| BlockKind::Paragraph {
            text: StyledText::from_plain(source.to_string()),
        });
    };
    if let Some(image) = converted.lone_image {
        return Some(image);
    }
    let text = StyledText {
        segments: converted.segments,
    };
    (!text.is_empty()).then_some(BlockKind::Paragraph { text })
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
