//! Plain-text output of raw HTML: the tags inside a paragraph, and whole
//! HTML blocks.

use super::{
    scan::{NotATag, QuoteEnds, html_tag_len},
    tag::{RawHtmlTag, TextStyle, classify_raw_html_tag},
};
use crate::core::document::{BlockKind, InlineSegment, InlineStyle};

/// Whether an HTML block is shown as its Markdown source rather than
/// rendered. Both views decide with this, so they always agree.
pub(crate) fn is_html_source_block(literal: &str) -> bool {
    html_to_text(literal).is_none()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BreakKind {
    /// `<br>`, which always starts a new line.
    LineBreak,
    /// A block tag, which starts a new line only when the text is not
    /// already at the start of one, so nesting adds no blank lines.
    Block,
}

/// How many of each style tag are open. A closing tag without an opening
/// one is ignored, as browsers do.
#[derive(Clone, Copy, Debug, Default)]
struct OpenStyles {
    bold: usize,
    italic: usize,
    code: usize,
}

impl OpenStyles {
    fn apply(&mut self, style: TextStyle, open: bool) {
        let depth = match style {
            TextStyle::Bold => &mut self.bold,
            TextStyle::Italic => &mut self.italic,
            TextStyle::Code => &mut self.code,
        };
        *depth = if open {
            *depth + 1
        } else {
            depth.saturating_sub(1)
        };
    }

    fn on(self, base: InlineStyle) -> InlineStyle {
        InlineStyle {
            bold: base.bold || self.bold > 0,
            italic: base.italic || self.italic > 0,
            code: base.code || self.code > 0,
        }
    }
}

/// Where a tag's plain text goes: the segments of a paragraph, or those of
/// a whole HTML block. `apply_tag` writes each tag's effect once for both.
trait PlainTextOut {
    fn push_str(&mut self, text: &str);
    fn push_space(&mut self);
    fn trim_end_spaces(&mut self);
    fn at_line_start(&self) -> bool;
    fn push_newline(&mut self, kind: BreakKind);
    fn href(&mut self) -> &mut Option<String>;
    fn open_styles(&mut self) -> &mut OpenStyles;
    fn image(&mut self, _source: &str, _alt: &str) {}
}

fn apply_tag(out: &mut impl PlainTextOut, kind: RawHtmlTag, source: &str) {
    match kind {
        RawHtmlTag::LineBreak => line_break(out, BreakKind::LineBreak),
        RawHtmlTag::Block => line_break(out, BreakKind::Block),
        RawHtmlTag::Image => {
            let alt = html_attr(source, "alt").unwrap_or_default();
            out.image(source, &alt);
            out.push_str(&alt);
        }
        RawHtmlTag::LinkStart => {
            *out.href() = html_attr(source, "href").filter(|href| !href.is_empty());
        }
        RawHtmlTag::LinkEnd => {
            if let Some(href) = out.href().take() {
                out.push_space();
                out.push_str(&format!("<{href}>"));
            }
        }
        RawHtmlTag::Style { style, open } => out.open_styles().apply(style, open),
        RawHtmlTag::Inline => {}
    }
}

fn line_break(out: &mut impl PlainTextOut, kind: BreakKind) {
    out.trim_end_spaces();
    if kind == BreakKind::LineBreak || !out.at_line_start() {
        out.push_newline(kind);
    }
}

#[derive(Default)]
pub(crate) struct InlineHtmlState {
    /// Index in the parts of the newline added by the last `<br>` or block
    /// tag.
    last_break: Option<(usize, BreakKind)>,
    href: Option<String>,
    styles: OpenStyles,
}

impl InlineHtmlState {
    /// `style` with the styles of the HTML tags open at this point, such as
    /// `<b>`. Markdown nesting and HTML tags can overlap, as in
    /// `<b>a *b</b> c*`, so each text node applies this itself.
    pub(crate) fn style(&self, style: InlineStyle) -> InlineStyle {
        self.styles.on(style)
    }

    /// Whether the last non-empty part is the newline from `<br>` or a block
    /// tag. A later tag can trim a part to empty, so empty parts are skipped.
    pub(crate) fn follows_line_break(&self, parts: &[InlineSegment]) -> bool {
        self.break_before(parts).is_some()
    }

    /// Whether the last non-empty part is a block tag's newline, which a
    /// Markdown line break does not stack with.
    pub(crate) fn follows_block_break(&self, parts: &[InlineSegment]) -> bool {
        self.break_before(parts) == Some(BreakKind::Block)
    }

    fn break_before(&self, parts: &[InlineSegment]) -> Option<BreakKind> {
        let last = parts.iter().rposition(|part| !part.text.is_empty())?;
        self.last_break
            .filter(|&(index, _)| index == last)
            .map(|(_, kind)| kind)
    }
}

struct InlineOut<'a> {
    parts: &'a mut Vec<InlineSegment>,
    state: &'a mut InlineHtmlState,
    style: InlineStyle,
}

impl PlainTextOut for InlineOut<'_> {
    fn push_str(&mut self, text: &str) {
        if !text.is_empty() {
            self.parts.push(InlineSegment {
                text: text.to_string(),
                style: self.state.style(self.style),
            });
        }
    }

    fn push_space(&mut self) {
        self.push_str(" ");
    }

    fn trim_end_spaces(&mut self) {
        if let Some(last) = self.parts.last_mut()
            && !last.style.code
        {
            last.text.truncate(last.text.trim_end_matches(' ').len());
        }
    }

    fn at_line_start(&self) -> bool {
        self.parts
            .iter()
            .rev()
            .find(|part| !part.text.is_empty())
            .is_none_or(|part| part.text.ends_with('\n'))
    }

    fn push_newline(&mut self, kind: BreakKind) {
        self.state.last_break = Some((self.parts.len(), kind));
        self.push_str("\n");
    }

    fn href(&mut self) -> &mut Option<String> {
        &mut self.state.href
    }

    fn open_styles(&mut self) -> &mut OpenStyles {
        &mut self.state.styles
    }
}

/// Comrak emits each inline tag as its own node, so only the tag itself is
/// handled here; the text between tags arrives as ordinary text nodes.
/// Unsupported tags and comments are kept as their source.
pub(crate) fn collect_inline_html(
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
            style: state.style(style),
        });
        return;
    };
    apply_tag(
        &mut InlineOut {
            parts,
            state,
            style,
        },
        kind,
        source,
    );
}

pub(crate) struct HtmlBlockText {
    pub(crate) segments: Vec<InlineSegment>,
    /// The block's only content, when that is one `<img>` with a `src`.
    pub(crate) lone_image: Option<BlockKind>,
}

#[derive(Default)]
struct BlockOut {
    /// Never holds an empty segment, so the last one ends the text.
    segments: Vec<InlineSegment>,
    images: Vec<Option<BlockKind>>,
    /// Whether the block has text of its own, outside its tags.
    has_text: bool,
    href: Option<String>,
    styles: OpenStyles,
}

impl BlockOut {
    fn last_char(&self) -> Option<char> {
        self.segments
            .last()
            .and_then(|last| last.text.chars().last())
    }

    /// Entities never contain `<` or whitespace, so each chunk between them
    /// decodes on its own; decoding chunks one by one keeps a reference split
    /// by a tag, such as `&am<b>p;`, from being joined.
    fn push_source_text(&mut self, chunk: &str) {
        if !chunk.is_empty() {
            self.push_str(&htmlize::unescape(chunk));
            self.has_text = true;
        }
    }

    fn finish(mut self) -> HtmlBlockText {
        let lone_image = match self.images.as_mut_slice() {
            [image @ Some(_)] if !self.has_text => image.take(),
            _ => None,
        };
        let mut segments = self.segments;
        // Draining once keeps this linear when many segments are blank.
        let blank = segments
            .iter()
            .take_while(|segment| segment.text.trim_start_matches([' ', '\n']).is_empty())
            .count();
        segments.drain(..blank);
        if let Some(first) = segments.first_mut() {
            first.text = first.text.trim_start_matches([' ', '\n']).to_string();
        }
        while let Some(last) = segments.last_mut() {
            last.text
                .truncate(last.text.trim_end_matches([' ', '\n']).len());
            if !last.text.is_empty() {
                break;
            }
            segments.pop();
        }
        HtmlBlockText {
            segments,
            lone_image,
        }
    }
}

impl PlainTextOut for BlockOut {
    fn push_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let style = self.styles.on(InlineStyle::default());
        match self.segments.last_mut() {
            Some(last) if last.style == style => last.text.push_str(text),
            _ => self.segments.push(InlineSegment {
                text: text.to_string(),
                style,
            }),
        }
    }

    fn push_space(&mut self) {
        if self.last_char().is_some_and(|c| c != ' ' && c != '\n') {
            self.push_str(" ");
        }
    }

    fn trim_end_spaces(&mut self) {
        while let Some(last) = self.segments.last_mut() {
            last.text.truncate(last.text.trim_end_matches(' ').len());
            if !last.text.is_empty() {
                break;
            }
            self.segments.pop();
        }
    }

    fn at_line_start(&self) -> bool {
        self.last_char().is_none_or(|c| c == '\n')
    }

    fn push_newline(&mut self, _kind: BreakKind) {
        self.push_str("\n");
    }

    fn href(&mut self) -> &mut Option<String> {
        &mut self.href
    }

    fn open_styles(&mut self) -> &mut OpenStyles {
        &mut self.styles
    }

    fn image(&mut self, source: &str, alt: &str) {
        self.images
            .push(html_attr(source, "src").map(|src| BlockKind::Image {
                src,
                alt: alt.to_string(),
                title: html_attr(source, "title"),
            }));
    }
}

/// Plain-text rendering of an HTML block, or `None` when the block must be
/// shown as its source: it holds a tag that doctty does not support, a
/// comment or another `<!`/`<?` construct, or no tag at all. Rendering only
/// part of a block would leave a mix the reader cannot make sense of.
pub(crate) fn html_to_text(html: &str) -> Option<HtmlBlockText> {
    let mut out = BlockOut::default();
    let mut has_supported_tag = false;
    let mut rest = html;
    // Input before this offset is known not to hold a tag.
    let mut text_until = 0;
    let quote_ends = QuoteEnds::of(html);

    // HTML collapses only ASCII whitespace; NBSP and U+3000 are content.
    while let Some(index) = rest.find(|c: char| c == '<' || c.is_ascii_whitespace()) {
        out.push_source_text(&rest[..index]);
        rest = &rest[index..];
        let offset = html.len() - rest.len();

        if let Some(after) = rest.strip_prefix(|c: char| c.is_ascii_whitespace()) {
            out.push_space();
            rest = after;
            continue;
        }
        if offset < text_until {
            out.push_source_text("<");
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
        apply_tag(&mut out, classify_raw_html_tag(source)?, source);
        has_supported_tag = true;
    }
    out.push_source_text(rest);

    has_supported_tag.then(|| out.finish())
}

/// Reads attribute `name` from the source of one start tag, following the
/// HTML attribute syntax: case-insensitive names, and values that are
/// double-quoted, single-quoted or unquoted.
///
/// This runs only on tags that `classify_raw_html_tag` accepted, so the
/// rich view's stricter reader has already checked the same attributes. It
/// stays separate because plain text needs values with their character
/// references decoded, while the rich view checks and writes them as
/// written. Plain text also reads `title`, which the rich view drops.
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
