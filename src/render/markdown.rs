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
    github_html::{RawHtmlTag, classify_raw_html_tag},
    markdown_pipeline::gfm_options,
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
                    style,
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
        NodeValue::HtmlInline(literal) => collect_inline_html(literal, style, parts, html),
        NodeValue::TaskItem(task) => {
            let marker = task
                .symbol
                .map_or("[ ] ".to_string(), |value| format!("[{value}] "));
            parts.push(InlineSegment {
                text: marker,
                style,
            });
            for child in node.children() {
                collect_text_segments(child, style, parts, html);
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
                collect_text_segments(child, style, parts, html);
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
            if html.br_from_block_tag && html.follows_line_break(parts) {
                return;
            }
            parts.push(InlineSegment {
                text: "\n".to_string(),
                style,
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

/// Whether an HTML block is shown as its Markdown source rather than
/// rendered. Both views decide with this, so they always agree.
pub(crate) fn is_html_source_block(literal: &str) -> bool {
    html_to_text(literal).is_none()
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
    let text = StyledText::from_plain(converted.text);
    (!text.is_empty()).then_some(BlockKind::Paragraph { text })
}

#[derive(Default)]
struct InlineHtmlState {
    /// Index in the parts of the newline added by the last `<br>` or block
    /// tag.
    br_index: Option<usize>,
    /// A block tag's newline does not stack with a Markdown line break.
    br_from_block_tag: bool,
    href: Option<String>,
}

impl InlineHtmlState {
    /// Whether the last non-empty part is the newline from `<br>` or a block
    /// tag. A later tag can trim a part to empty, so empty parts are skipped.
    fn follows_line_break(&self, parts: &[InlineSegment]) -> bool {
        parts
            .iter()
            .rposition(|part| !part.text.is_empty())
            .is_some_and(|last| self.br_index == Some(last))
    }
}

/// Comrak emits each inline tag as its own node, so only the tag itself is
/// handled here; the text between tags arrives as ordinary text nodes.
/// Unsupported tags and comments are kept as their source.
fn collect_inline_html(
    literal: &str,
    style: InlineStyle,
    parts: &mut Vec<InlineSegment>,
    state: &mut InlineHtmlState,
) {
    let supported = html_tag_len(literal, QuoteEnds::of(literal))
        .ok()
        .and_then(|len| {
            let source = &literal[..len];
            classify_raw_html_tag(source).map(|kind| (kind, source))
        });
    let Some((kind, source)) = supported else {
        parts.push(InlineSegment {
            text: literal.to_string(),
            style,
        });
        return;
    };
    let text = match kind {
        RawHtmlTag::LineBreak | RawHtmlTag::Block => {
            if let Some(last) = parts.last_mut()
                && !last.style.code
            {
                last.text.truncate(last.text.trim_end_matches(' ').len());
            }
            let block_tag = kind == RawHtmlTag::Block;
            let at_line_start = parts
                .iter()
                .rev()
                .find(|part| !part.text.is_empty())
                .is_none_or(|part| part.text.ends_with('\n'));
            if block_tag && at_line_start {
                return;
            }
            state.br_index = Some(parts.len());
            state.br_from_block_tag = block_tag;
            "\n".to_string()
        }
        RawHtmlTag::Image => html_attr(source, "alt").unwrap_or_default(),
        RawHtmlTag::LinkStart => {
            state.href = html_attr(source, "href").filter(|href| !href.is_empty());
            return;
        }
        RawHtmlTag::LinkEnd => match state.href.take() {
            Some(href) => format!(" <{href}>"),
            None => return,
        },
        RawHtmlTag::Inline => return,
    };
    if !text.is_empty() {
        parts.push(InlineSegment { text, style });
    }
}

/// The first `len` bytes cannot start a tag anywhere, so callers keep them as
/// text without scanning them again.
struct NotATag {
    len: usize,
}

/// Byte offsets of the last `"` and `'` in the input. A quote opened there
/// has nothing to close it, so it is read as an ordinary character; this
/// keeps one unbalanced quote from swallowing the rest of the block.
#[derive(Clone, Copy)]
struct QuoteEnds {
    double: Option<usize>,
    single: Option<usize>,
}

impl QuoteEnds {
    fn of(html: &str) -> Self {
        Self {
            double: html.rfind('"'),
            single: html.rfind('\''),
        }
    }

    fn skip(self, bytes: usize) -> Self {
        Self {
            double: self.double.and_then(|at| at.checked_sub(bytes)),
            single: self.single.and_then(|at| at.checked_sub(bytes)),
        }
    }

    fn can_close(self, quote: char, at: usize) -> bool {
        let last = if quote == '"' {
            self.double
        } else {
            self.single
        };
        last.is_some_and(|last| at < last)
    }
}

/// Byte length of the tag at the start of `html`. Comments and
/// declarations are not tags.
///
/// Unlike a browser, an unquoted `<` ends the attempt, and callers skip the
/// whole failed span: rescanning it from each stray `<` made stripping
/// quadratic.
fn html_tag_len(html: &str, quote_ends: QuoteEnds) -> Result<usize, NotATag> {
    let rest = html.strip_prefix('<').ok_or(NotATag { len: 1 })?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(NotATag { len: 1 });
    }
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(rest.len());

    let mut quote = None;
    let mut after_equals = false;
    for (offset, c) in rest[name_len..].char_indices() {
        if let Some(open) = quote {
            if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '>' => {
                return Ok(html.len() - rest.len() + name_len + offset + 1);
            }
            '<' => {
                let len = html.len() - rest.len() + name_len + offset;
                return Err(NotATag { len });
            }
            '"' | '\''
                if after_equals
                    && quote_ends.can_close(c, html.len() - rest.len() + name_len + offset) =>
            {
                quote = Some(c)
            }
            _ => {}
        }
        if !c.is_ascii_whitespace() {
            after_equals = c == '=';
        }
    }
    Err(NotATag { len: html.len() })
}

struct HtmlBlockText {
    text: String,
    /// The block's only content, when that is one `<img>` with a `src`.
    lone_image: Option<BlockKind>,
}

/// Plain-text rendering of an HTML block, or `None` when the block must be
/// shown as its source: it holds a tag that doctty does not support, a
/// comment or another `<!`/`<?` construct, or no tag at all. Rendering only
/// part of a block would leave a mix the reader cannot make sense of.
fn html_to_text(html: &str) -> Option<HtmlBlockText> {
    let mut text = String::with_capacity(html.len());
    let mut images = Vec::new();
    let mut has_text = false;
    let mut has_supported_tag = false;
    let mut href: Option<String> = None;
    let mut rest = html;
    // Input before this offset is known not to hold a tag.
    let mut text_until = 0;
    let quote_ends = QuoteEnds::of(html);

    // HTML collapses only ASCII whitespace; NBSP and U+3000 are content.
    while let Some(index) = rest.find(|c: char| c == '<' || c.is_ascii_whitespace()) {
        push_html_text(&mut text, &mut has_text, &rest[..index]);
        rest = &rest[index..];
        let offset = html.len() - rest.len();

        if let Some(after) = rest.strip_prefix(|c: char| c.is_ascii_whitespace()) {
            push_html_space(&mut text);
            rest = after;
            continue;
        }
        if offset < text_until {
            push_html_text(&mut text, &mut has_text, "<");
            rest = &rest[1..];
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            return None;
        }
        let len = match html_tag_len(rest, quote_ends.skip(offset)) {
            Ok(len) => len,
            Err(NotATag { len }) => {
                text_until = offset + len;
                continue;
            }
        };
        let source = &rest[..len];
        rest = &rest[len..];
        let kind = classify_raw_html_tag(source)?;
        has_supported_tag = true;
        match kind {
            RawHtmlTag::Image => {
                let alt = html_attr(source, "alt").unwrap_or_default();
                images.push(html_attr(source, "src").map(|src| BlockKind::Image {
                    src,
                    alt: alt.clone(),
                    title: html_attr(source, "title"),
                }));
                text.push_str(&alt);
            }
            RawHtmlTag::LinkStart => {
                href = html_attr(source, "href").filter(|href| !href.is_empty());
            }
            RawHtmlTag::LinkEnd => {
                if let Some(href) = href.take() {
                    push_html_space(&mut text);
                    text.push_str(&format!("<{href}>"));
                }
            }
            RawHtmlTag::LineBreak => push_html_line_break(&mut text, true),
            RawHtmlTag::Block => push_html_line_break(&mut text, false),
            RawHtmlTag::Inline => {}
        }
    }
    push_html_text(&mut text, &mut has_text, rest);

    if !has_supported_tag {
        return None;
    }
    let lone_image = match images.as_mut_slice() {
        [image @ Some(_)] if !has_text => image.take(),
        _ => None,
    };
    let text = text.trim_matches([' ', '\n']).to_string();
    Some(HtmlBlockText { text, lone_image })
}

/// Entities never contain `<` or whitespace, so each chunk between them
/// decodes on its own; decoding chunks one by one keeps a reference split
/// by a tag, such as `&am<b>p;`, from being joined.
fn push_html_text(text: &mut String, has_text: &mut bool, chunk: &str) {
    if !chunk.is_empty() {
        text.push_str(&htmlize::unescape(chunk));
        *has_text = true;
    }
}

fn push_html_space(text: &mut String) {
    if !text.is_empty() && !text.ends_with([' ', '\n']) {
        text.push(' ');
    }
}

/// Starts a new line. `<br>` always does; block tags do only when the text
/// is not already at the start of a line, so nesting adds no blank lines.
fn push_html_line_break(text: &mut String, always: bool) {
    text.truncate(text.trim_end_matches(' ').len());
    if always || !(text.is_empty() || text.ends_with('\n')) {
        text.push('\n');
    }
}

/// Reads attribute `name` from the source of one start tag, following the
/// HTML attribute syntax: case-insensitive names, and values that are
/// double-quoted, single-quoted or unquoted.
fn html_attr(tag: &str, name: &str) -> Option<String> {
    let tag = tag.strip_prefix('<')?;
    let mut rest = tag.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '-');
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
        if rest.is_empty() || rest.starts_with('>') {
            return None;
        }
        let name_len = rest
            .find(|c: char| c.is_ascii_whitespace() || matches!(c, '=' | '>' | '/'))
            .unwrap_or(rest.len())
            .max(1);
        let attr_name = &rest[..name_len];
        rest = rest[name_len..].trim_ascii_start();
        let mut value = "";
        if let Some(after_equals) = rest.strip_prefix('=') {
            rest = after_equals.trim_ascii_start();
            let value_len = match rest.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let end = rest[1..].find(quote).map_or(rest.len(), |end| end + 1);
                    value = &rest[1..end];
                    (end + 1).min(rest.len())
                }
                _ => {
                    let end = rest
                        .find(|c: char| c.is_ascii_whitespace() || c == '>')
                        .unwrap_or(rest.len());
                    value = &rest[..end];
                    end
                }
            };
            rest = &rest[value_len..];
        }
        if attr_name.eq_ignore_ascii_case(name) {
            return Some(htmlize::unescape_attribute(value).into_owned());
        }
    }
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
