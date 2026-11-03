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
            let follows_br = parts
                .len()
                .checked_sub(1)
                .is_some_and(|last| html.br_index == Some(last));
            let value = if follows_br {
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

fn normalize_html_block(literal: &str) -> Option<BlockKind> {
    if let Some(code) = html_pre_block(literal) {
        return Some(BlockKind::CodeFence {
            language: None,
            code,
        });
    }

    let converted = html_to_text(literal);
    if let ([image], 1, false) = (
        converted.images.as_slice(),
        converted.image_count,
        converted.has_text,
    ) {
        return Some(image.clone());
    }
    if converted.text == "---" {
        return Some(BlockKind::Rule);
    }
    let text = StyledText::from_plain(converted.text);
    (!text.is_empty()).then_some(BlockKind::Paragraph { text })
}

/// State carried between inline HTML nodes of one paragraph.
#[derive(Default)]
struct InlineHtmlState {
    /// Index in the collected parts of the newline that the last `<br>` added.
    br_index: Option<usize>,
    /// `href` of the open `<a>`, printed after its text like a Markdown link.
    href: Option<String>,
}

/// Comrak emits each inline tag as its own node, so only the tag itself is
/// handled here; the text between tags arrives as ordinary text nodes.
fn collect_inline_html(
    literal: &str,
    style: InlineStyle,
    parts: &mut Vec<InlineSegment>,
    state: &mut InlineHtmlState,
) {
    let Ok(tag) = parse_html_tag(literal, QuoteEnds::of(literal)) else {
        return;
    };
    let source = &literal[..tag.len];
    let text = match (tag.name.as_str(), tag.closing) {
        ("br", _) => {
            if let Some(last) = parts.last_mut()
                && !last.style.code
            {
                last.text.truncate(last.text.trim_end_matches(' ').len());
            }
            state.br_index = Some(parts.len());
            "\n".to_string()
        }
        ("img", false) => html_attr(source, "alt").unwrap_or_default(),
        ("a", false) => {
            state.href = html_attr(source, "href").filter(|href| !href.is_empty());
            return;
        }
        ("a", true) => match state.href.take() {
            Some(href) => format!(" <{href}>"),
            None => return,
        },
        _ => return,
    };
    if !text.is_empty() {
        parts.push(InlineSegment { text, style });
    }
}

struct HtmlTag {
    name: String,
    closing: bool,
    len: usize,
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

/// Parses the tag at the start of `html`. Comments and declarations are not
/// tags.
///
/// Unlike a browser, an unquoted `<` ends the attempt, and callers skip the
/// whole failed span: rescanning it from each stray `<` made stripping
/// quadratic.
fn parse_html_tag(html: &str, quote_ends: QuoteEnds) -> Result<HtmlTag, NotATag> {
    let rest = html.strip_prefix('<').ok_or(NotATag { len: 1 })?;
    let (closing, rest) = match rest.strip_prefix('/') {
        Some(after) => (true, after),
        None => (false, rest),
    };
    if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(NotATag { len: 1 });
    }
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(rest.len());
    let name = rest[..name_len].to_ascii_lowercase();

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
                let len = html.len() - rest.len() + name_len + offset + 1;
                return Ok(HtmlTag { name, closing, len });
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

/// Finds the first `</name ...>` in `html`, returning where it starts and
/// ends. It is found without parsing what lies before it, as the contents
/// of `<script>`, `<style>` and `<pre>` are read up to it.
fn find_closing_tag(html: &str, name: &str) -> Option<(usize, usize)> {
    let mut search_from = 0;
    while let Some(found) = html[search_from..].find("</") {
        let start = search_from + found;
        let name_start = start + "</".len();
        let after_name = name_start + name.len();
        let closes = html
            .get(name_start..after_name)
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
            && html[after_name..]
                .chars()
                .next()
                .is_none_or(|c| c.is_ascii_whitespace() || matches!(c, '/' | '>'));
        if closes {
            let end = html[after_name..]
                .find('>')
                .map_or(html.len(), |gt| after_name + gt + 1);
            return Some((start, end));
        }
        search_from = name_start;
    }
    None
}

/// Returns the input after the element that `<name>` opened, or `""` when it
/// is never closed.
fn skip_raw_text_element<'a>(html: &'a str, name: &str) -> &'a str {
    find_closing_tag(html, name).map_or("", |(_, end)| &html[end..])
}

struct HtmlBlockText {
    text: String,
    /// Images that have a `src`.
    images: Vec<BlockKind>,
    /// All `<img>` tags, with or without `src`.
    image_count: usize,
    /// Whether anything other than images was printed.
    has_text: bool,
}

/// Plain-text rendering of an HTML block. Only tags common in READMEs get
/// special output: block-level elements and `<br>` start a new line, list
/// items get a marker, `<a href>` prints its URL like a Markdown link,
/// `<img>` prints its alt text and `<hr>` prints `---`. Other tags are
/// removed and their text is kept.
fn html_to_text(html: &str) -> HtmlBlockText {
    let mut text = String::with_capacity(html.len());
    let mut images = Vec::new();
    let mut image_count = 0;
    let mut has_text = false;
    let mut href: Option<String> = None;
    // `None` for `<ul>`, the next item number for `<ol>`.
    let mut lists: Vec<Option<i64>> = Vec::new();
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
        if let Some(after_open) = rest.strip_prefix("<!--") {
            rest = after_open
                .find("-->")
                .map_or("", |end| &after_open[end + "-->".len()..]);
            continue;
        }
        let tag = match parse_html_tag(rest, quote_ends.skip(offset)) {
            Ok(tag) => tag,
            Err(NotATag { len }) => {
                text_until = offset + len;
                continue;
            }
        };
        let source = &rest[..tag.len];
        rest = &rest[tag.len..];
        match (tag.name.as_str(), tag.closing) {
            ("script" | "style", false) => rest = skip_raw_text_element(rest, &tag.name),
            ("img", false) => {
                image_count += 1;
                let alt = html_attr(source, "alt").unwrap_or_default();
                if let Some(src) = html_attr(source, "src") {
                    images.push(BlockKind::Image {
                        src,
                        alt: alt.clone(),
                        title: html_attr(source, "title"),
                    });
                }
                text.push_str(&alt);
            }
            ("a", false) => href = html_attr(source, "href").filter(|href| !href.is_empty()),
            ("a", true) => {
                if let Some(href) = href.take() {
                    push_html_space(&mut text);
                    text.push_str(&format!("<{href}>"));
                }
            }
            ("ul", false) => {
                lists.push(None);
                text.push('\n');
            }
            ("ol", false) => {
                let start =
                    html_attr(source, "start").and_then(|start| start.trim().parse::<i64>().ok());
                lists.push(Some(start.unwrap_or(1)));
                text.push('\n');
            }
            ("ul" | "ol", true) => {
                lists.pop();
                text.push('\n');
            }
            ("li", false) => {
                let marker = match lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}. ");
                        *number = number.saturating_add(1);
                        marker
                    }
                    _ => "- ".to_string(),
                };
                text.push('\n');
                text.push_str(&marker);
                has_text = true;
            }
            ("hr", _) => {
                text.push_str("\n---\n");
                has_text = true;
            }
            ("td" | "th", _) => push_html_space(&mut text),
            (name, _) if is_html_line_break(name) => text.push('\n'),
            _ => {}
        }
    }
    push_html_text(&mut text, &mut has_text, rest);

    let text = text
        .lines()
        .map(str::trim_ascii)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    HtmlBlockText {
        text,
        images,
        image_count,
        has_text,
    }
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

/// Returns the text of a block that is a single `<pre>` element, keeping its
/// whitespace. Tags inside it are removed.
fn html_pre_block(html: &str) -> Option<String> {
    let html = html.trim_ascii();
    let open = parse_html_tag(html, QuoteEnds::of(html)).ok()?;
    if open.name != "pre" || open.closing {
        return None;
    }
    let body = &html[open.len..];
    let (close_at, close_end) = find_closing_tag(body, "pre")?;
    if close_end != body.len() {
        return None;
    }
    // HTML drops the line break right after `<pre>`.
    let body = &body[..close_at];
    let body = body
        .strip_prefix("\r\n")
        .or_else(|| body.strip_prefix('\n'))
        .unwrap_or(body);

    let mut code = String::with_capacity(body.len());
    let mut rest = body;
    let quote_ends = QuoteEnds::of(body);
    while let Some(index) = rest.find('<') {
        code.push_str(&htmlize::unescape(&rest[..index]));
        rest = &rest[index..];
        if let Some(after_open) = rest.strip_prefix("<!--") {
            rest = after_open
                .find("-->")
                .map_or("", |end| &after_open[end + "-->".len()..]);
            continue;
        }
        let offset = body.len() - rest.len();
        match parse_html_tag(rest, quote_ends.skip(offset)) {
            Ok(tag) => {
                let source = &rest[..tag.len];
                rest = &rest[tag.len..];
                match (tag.name.as_str(), tag.closing) {
                    ("br", _) => code.push('\n'),
                    ("img", false) => code.push_str(&html_attr(source, "alt").unwrap_or_default()),
                    ("script" | "style", false) => rest = skip_raw_text_element(rest, &tag.name),
                    _ => {}
                }
            }
            Err(NotATag { len }) => {
                code.push_str(&htmlize::unescape(&rest[..len]));
                rest = &rest[len..];
            }
        }
    }
    code.push_str(&htmlize::unescape(rest));
    let code = code.trim_end_matches(['\r', '\n']).to_string();
    (!code.trim_ascii().is_empty()).then_some(code)
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
