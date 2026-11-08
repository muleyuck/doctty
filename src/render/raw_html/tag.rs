//! Which raw HTML tags doctty supports, and the sanitized form the rich view
//! restores each one as.

/// What a supported raw HTML tag does. This is the one list of supported
/// tags: the rich view restores exactly these, and plain-text output matches
/// on the kind, so adding a tag makes both views handle it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RawHtmlTag {
    /// `<br>`.
    LineBreak,
    /// Start or end of a block element, such as `<div>` or `</p>`.
    Block,
    /// `<a href>`.
    LinkStart,
    /// `</a>`.
    LinkEnd,
    /// `<img>`.
    Image,
    /// `<sub>`, `<sup>`, `<picture>` and `<source>`, which only wrap text or
    /// images.
    Inline,
}

const BLOCK_TAGS: &[&str] = &[
    "div", "p", "h1", "h2", "h3", "h4", "h5", "h6", "details", "summary",
];

/// The kind of the raw tag `tag` (such as `<br>`) when the rich view renders
/// it, or `None` when both views show it as text.
pub(crate) fn classify_raw_html_tag(tag: &str) -> Option<RawHtmlTag> {
    // The rich view finds a tag from its `<` to the first `>`, so a tag with
    // `>` inside its attributes is never restored there.
    let inner = tag.strip_prefix('<')?.strip_suffix('>')?;
    if inner.contains('>') {
        return None;
    }
    let mut escaped = String::with_capacity(tag.len() + 16);
    escaped.push_str("&lt;");
    comrak::html::escape(&mut escaped, inner).ok()?;
    escaped.push_str("&gt;");
    restore_supported_tag(&escaped).map(|(kind, _)| kind)
}

pub(crate) fn restore_supported_tag(escaped_tag: &str) -> Option<(RawHtmlTag, String)> {
    let tag = escaped_tag
        .strip_prefix("&lt;")?
        .strip_suffix("&gt;")?
        .trim();
    let decoded = tag.replace("&quot;", "\"").replace("&amp;", "&");
    let decoded = decoded.trim();

    let simple = match decoded {
        "br" | "br/" | "br /" => Some((RawHtmlTag::LineBreak, "<br/>")),
        "sub" => Some((RawHtmlTag::Inline, "<sub>")),
        "/sub" => Some((RawHtmlTag::Inline, "</sub>")),
        "sup" => Some((RawHtmlTag::Inline, "<sup>")),
        "/sup" => Some((RawHtmlTag::Inline, "</sup>")),
        "/picture" => Some((RawHtmlTag::Inline, "</picture>")),
        "/a" => Some((RawHtmlTag::LinkEnd, "</a>")),
        _ => None,
    };
    if let Some((kind, html)) = simple {
        return Some((kind, html.to_string()));
    }

    let restored = [
        (
            RawHtmlTag::Block,
            restore_block_tag as fn(&str) -> Option<String>,
        ),
        (RawHtmlTag::Block, restore_block_tag_close),
        (RawHtmlTag::LinkStart, restore_anchor),
        (RawHtmlTag::Inline, restore_picture_tag),
        (RawHtmlTag::Inline, restore_source_tag),
        (RawHtmlTag::Image, restore_img),
    ];
    restored
        .into_iter()
        .find_map(|(kind, restore)| restore(decoded).map(|html| (kind, html)))
}

fn restore_block_tag(tag: &str) -> Option<String> {
    let (name, attrs) = split_tag_name(tag)?;
    if !BLOCK_TAGS.contains(&name) {
        return None;
    }

    if attrs.is_empty() {
        return Some(format!("<{name}>"));
    }

    let parsed = parse_attributes_and_flags(attrs)?;
    let align = parsed
        .attrs
        .iter()
        .find(|(attr_name, _)| attr_name == "align")
        .map(|(_, value)| value.as_str());

    let mut output = format!("<{name}");

    if let Some(value) = align {
        if !matches!(value, "left" | "center" | "right") {
            return None;
        }
        output.push_str(&format!(r#" align="{value}""#));
    }

    if name == "details" && parsed.flags.iter().any(|flag| flag == "open") {
        output.push_str(" open");
    }

    output.push('>');
    Some(output)
}

fn restore_block_tag_close(tag: &str) -> Option<String> {
    let name = tag.strip_prefix('/')?;
    BLOCK_TAGS.contains(&name).then(|| format!("</{name}>"))
}

fn restore_anchor(tag: &str) -> Option<String> {
    let attrs = strip_tag_name(tag, "a")?.trim();
    let attrs = parse_attributes_strict(attrs)?;
    let href = attrs
        .iter()
        .find(|(name, _)| name == "href")
        .map(|(_, value)| value.as_str())?;
    if !is_safe_href(href) {
        return None;
    }

    Some(format!(r#"<a href="{href}">"#))
}

fn restore_picture_tag(tag: &str) -> Option<String> {
    let (name, attrs) = split_tag_name(tag)?;
    if name != "picture" {
        return None;
    }
    if !attrs.is_empty() {
        return None;
    }
    Some("<picture>".to_string())
}

fn restore_source_tag(tag: &str) -> Option<String> {
    let body = strip_tag_name(tag, "source")?.trim();
    let body = body.strip_suffix('/').unwrap_or(body).trim();
    let attrs = parse_attributes_strict(body)?;

    let mut result = String::from("<source");
    for (name, value) in &attrs {
        match name.as_str() {
            "media" | "type" | "sizes" => result.push_str(&format!(r#" {name}="{value}""#)),
            "srcset" => {
                if !is_safe_srcset(value) {
                    return None;
                }
                result.push_str(&format!(r#" {name}="{value}""#));
            }
            _ => return None,
        }
    }
    result.push_str(" />");
    Some(result)
}

fn restore_img(tag: &str) -> Option<String> {
    let body = strip_tag_name(tag, "img")?.trim();
    let body = body.strip_suffix('/').unwrap_or(body).trim();

    let attrs = parse_attributes_strict(body)?;
    let src = attrs
        .iter()
        .find(|(name, _)| *name == "src")
        .map(|(_, value)| value.as_str())?;
    if !is_safe_src(src) {
        return None;
    }

    let mut result = format!(r#"<img src="{src}""#);
    for (name, value) in &attrs {
        match name.as_str() {
            "src" => {}
            "alt" | "width" | "height" => {
                result.push_str(&format!(r#" {name}="{value}""#));
            }
            _ => {}
        }
    }
    result.push_str(" />");

    Some(result)
}

/// The rest of `tag` after its name, when the name is exactly `name`, so
/// that `<img-foo>` or `<abbr>` is not read as `<img>` or `<a>`.
fn strip_tag_name<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    tag.strip_prefix(name)
        .filter(|rest| rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || c == '/'))
}

fn split_tag_name(tag: &str) -> Option<(&str, &str)> {
    let trimmed = tag.trim();
    if trimmed.is_empty() {
        return None;
    }

    match trimmed.split_once(char::is_whitespace) {
        Some((name, attrs)) => Some((name, attrs.trim())),
        None => Some((trimmed, "")),
    }
}

fn parse_attributes_strict(input: &str) -> Option<Vec<(String, String)>> {
    let input = input.trim();
    if input.is_empty() {
        return Some(Vec::new());
    }

    let mut attrs = Vec::new();
    let mut rest = input;

    while !rest.is_empty() {
        let (name, after_name) = match rest.split_once('=') {
            Some((name, after)) => (name.trim(), after.trim()),
            None => return None,
        };
        let after_quote = after_name.strip_prefix('"')?;
        let (quoted, remainder) = after_quote.split_once('"')?;
        rest = remainder.trim();
        if quoted.contains('<') || quoted.contains('>') {
            return None;
        }
        attrs.push((name.to_string(), quoted.to_string()));
    }

    Some(attrs)
}

struct ParsedTagAttributes {
    attrs: Vec<(String, String)>,
    flags: Vec<String>,
}

fn parse_attributes_and_flags(input: &str) -> Option<ParsedTagAttributes> {
    let input = input.trim();
    if input.is_empty() {
        return Some(ParsedTagAttributes {
            attrs: Vec::new(),
            flags: Vec::new(),
        });
    }

    let mut attrs = Vec::new();
    let mut flags = Vec::new();
    let mut rest = input;
    // Flags are consumed only before the first `=`, so its position never
    // changes; searching for it on every flag made long tags quadratic.
    let first_eq = input.find('=');

    while !rest.is_empty() {
        let offset = input.len() - rest.len();
        let eq_pos = first_eq.and_then(|eq| eq.checked_sub(offset));
        let space_pos = rest.find(char::is_whitespace);

        match (eq_pos, space_pos) {
            (Some(eq), Some(space)) if space < eq => {
                let flag = rest[..space].trim();
                if flag.is_empty() {
                    return None;
                }
                flags.push(flag.to_string());
                rest = rest[space..].trim();
            }
            (None, Some(space)) => {
                let flag = rest[..space].trim();
                if flag.is_empty() {
                    return None;
                }
                flags.push(flag.to_string());
                rest = rest[space..].trim();
            }
            (None, None) => {
                flags.push(rest.trim().to_string());
                rest = "";
            }
            _ => {
                let parsed = parse_attributes_strict(rest)?;
                attrs.extend(parsed);
                rest = "";
            }
        }
    }

    Some(ParsedTagAttributes { attrs, flags })
}

fn is_safe_href(href: &str) -> bool {
    href.starts_with("https://")
        || href.starts_with("http://")
        || href.starts_with("mailto:")
        || href.starts_with('/')
        || href.starts_with("./")
        || href.starts_with("../")
        || href.starts_with('#')
}

fn is_safe_src(src: &str) -> bool {
    if is_safe_href(src) {
        return true;
    }
    let lower = src.trim_start().to_ascii_lowercase();
    // Allow bare relative paths (e.g. "docs/screenshot.jpg", "image.png")
    // but reject anything that looks like a protocol other than http(s)
    !src.contains("://") && !lower.starts_with("javascript:") && !lower.starts_with("data:")
}

fn is_safe_srcset(srcset: &str) -> bool {
    for candidate in srcset.split(',') {
        let reference = candidate.split_whitespace().next().unwrap_or("");
        if reference.is_empty() || !is_safe_src(reference) {
            return false;
        }
    }

    true
}
