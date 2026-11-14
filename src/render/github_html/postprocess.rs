use crate::{cli::Theme, render::raw_html::restore_supported_tag};

pub(super) fn retint_code_tokens(html: &str, theme: Theme) -> String {
    match theme {
        Theme::Light | Theme::System => html.to_string(),
        Theme::Dark => {
            let color_map: &[(&str, &str)] = &[
                ("#b48ead", "#ff7b72"),
                ("#8fa1b3", "#79c0ff"),
                ("#a3be8c", "#a5d6ff"),
                ("#d08770", "#ffa657"),
                ("#96b5b4", "#7ee787"),
                ("#65737e", "#8b949e"),
            ];
            retint_style_colors(html, color_map)
        }
    }
}

/// Replace hex colors only inside `style="color:#..."` attributes so that
/// user-authored text containing the same hex values is not corrupted.
fn retint_style_colors(html: &str, color_map: &[(&str, &str)]) -> String {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    let needle = "style=\"color:";

    while let Some(pos) = rest.find(needle) {
        // Copy everything before this style attribute verbatim.
        output.push_str(&rest[..pos]);
        let after_prefix = &rest[pos + needle.len()..];
        // Find the closing quote of the style attribute value.
        let close = after_prefix.find('"').unwrap_or(after_prefix.len());
        let color_value = &after_prefix[..close];

        output.push_str(needle);
        let mut replaced = color_value.to_string();
        for &(from, to) in color_map {
            replaced = replaced.replace(from, to);
        }
        output.push_str(&replaced);

        rest = &after_prefix[close..];
    }

    output.push_str(rest);
    output
}

pub(super) fn decorate_code_blocks(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(pre_start) = rest.find("<pre") {
        output.push_str(&rest[..pre_start]);
        let pre_segment = &rest[pre_start..];
        let Some(pre_open_end) = pre_segment.find('>') else {
            output.push_str(pre_segment);
            return output;
        };
        let pre_open_tag = &pre_segment[..=pre_open_end];
        let Some(pre_close) = pre_segment.find("</pre>") else {
            output.push_str(pre_segment);
            return output;
        };
        let pre_body = &pre_segment[pre_open_end + 1..pre_close];
        let lang = extract_attr(pre_open_tag, "lang").map(sanitize_language_token);
        let decorated_pre_body = decorate_code_tag(pre_body, lang.as_deref());

        output.push_str(r#"<div class="highlight"#);
        if let Some(language) = &lang {
            output.push(' ');
            output.push_str("highlight-source-");
            output.push_str(language);
        }
        output.push_str(r#" notranslate position-relative overflow-auto" dir="auto">"#);
        output.push_str(pre_open_tag);
        output.push_str(&decorated_pre_body);
        output.push_str("</pre></div>");

        rest = &pre_segment[pre_close + "</pre>".len()..];
    }

    output.push_str(rest);
    output
}

fn decorate_code_tag(pre_body: &str, language: Option<&str>) -> String {
    let Some(code_start) = pre_body.find("<code") else {
        return pre_body.to_string();
    };
    let code_segment = &pre_body[code_start..];
    let Some(code_open_end) = code_segment.find('>') else {
        return pre_body.to_string();
    };

    let code_open_tag = &code_segment[..=code_open_end];
    let mut classes = extract_attr(code_open_tag, "class")
        .map(|value| {
            value
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if let Some(language) = language {
        classes.push(format!("language-{language}"));
    }
    classes.push("notranslate".to_string());
    classes.sort();
    classes.dedup();

    let mut decorated_open = String::from("<code");
    if !classes.is_empty() {
        decorated_open.push_str(r#" class=""#);
        decorated_open.push_str(&classes.join(" "));
        decorated_open.push('"');
    }
    if let Some(language) = language {
        decorated_open.push_str(r#" data-lang=""#);
        decorated_open.push_str(language);
        decorated_open.push('"');
    }
    decorated_open.push('>');

    let mut output = String::with_capacity(pre_body.len() + decorated_open.len());
    output.push_str(&pre_body[..code_start]);
    output.push_str(&decorated_open);
    output.push_str(&code_segment[code_open_end + 1..]);
    output
}

fn extract_attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!(r#"{name}=""#);
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

fn sanitize_language_token(language: String) -> String {
    language
        .chars()
        .filter(|char| char.is_ascii_alphanumeric() || matches!(char, '-' | '_'))
        .collect()
}

pub(super) fn inject_alert_icons(html: &str) -> String {
    [
        (
            "Note",
            r#"<svg class="octicon octicon-info mr-2" viewBox="0 0 16 16" version="1.1" width="16" height="16" aria-hidden="true"><path d="M0 8a8 8 0 1 1 16 0A8 8 0 0 1 0 8Zm8-6.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13ZM6.5 7.75A.75.75 0 0 1 7.25 7h1a.75.75 0 0 1 .75.75v2.75h.25a.75.75 0 0 1 0 1.5h-2a.75.75 0 0 1 0-1.5h.25v-2h-.25a.75.75 0 0 1-.75-.75ZM8 6a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z"></path></svg>"#,
        ),
        (
            "Tip",
            r#"<svg class="octicon octicon-light-bulb mr-2" viewBox="0 0 16 16" version="1.1" width="16" height="16" aria-hidden="true"><path d="M8 1.5a4.75 4.75 0 0 0-2.633 8.703c.603.398.883.91.883 1.547V12h3.5v-.25c0-.638.28-1.15.883-1.547A4.75 4.75 0 0 0 8 1.5ZM5.75 13.25a.75.75 0 0 1 .75-.75h3a.75.75 0 0 1 0 1.5h-3a.75.75 0 0 1-.75-.75ZM6.5 15a.75.75 0 0 1 0-1.5h2a.75.75 0 0 1 0 1.5h-2Z"></path></svg>"#,
        ),
        (
            "Important",
            r#"<svg class="octicon octicon-report mr-2" viewBox="0 0 16 16" version="1.1" width="16" height="16" aria-hidden="true"><path d="M1.75 2A1.75 1.75 0 0 0 0 3.75v8.5C0 13.216.784 14 1.75 14h12.5A1.75 1.75 0 0 0 16 12.25v-8.5A1.75 1.75 0 0 0 14.25 2H1.75ZM8 4.75a.75.75 0 0 1 .75.75v3.25a.75.75 0 0 1-1.5 0V5.5A.75.75 0 0 1 8 4.75Zm0 6a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z"></path></svg>"#,
        ),
        (
            "Warning",
            r#"<svg class="octicon octicon-alert mr-2" viewBox="0 0 16 16" version="1.1" width="16" height="16" aria-hidden="true"><path d="M6.457 1.047c.659-1.17 2.427-1.17 3.086 0l5.482 9.737c.648 1.152-.185 2.591-1.543 2.591H2.518c-1.358 0-2.191-1.439-1.543-2.59L6.457 1.047ZM8 5.25a.75.75 0 0 0-.75.75v2.25a.75.75 0 0 0 1.5 0V6A.75.75 0 0 0 8 5.25Zm0 5.25a1 1 0 1 0 0-2 1 1 0 0 0 0 2Z"></path></svg>"#,
        ),
        (
            "Caution",
            r#"<svg class="octicon octicon-stop mr-2" viewBox="0 0 16 16" version="1.1" width="16" height="16" aria-hidden="true"><path d="M4.47.22A.749.749 0 0 1 5 0h6c.2 0 .39.08.53.22l4.25 4.25c.14.14.22.33.22.53v6a.749.749 0 0 1-.22.53l-4.25 4.25a.749.749 0 0 1-.53.22H5a.749.749 0 0 1-.53-.22L.22 11.53A.749.749 0 0 1 0 11V5c0-.2.08-.39.22-.53L4.47.22Zm.84 1.28L1.5 5.31v5.38l3.81 3.81h5.38l3.81-3.81V5.31L10.69 1.5H5.31ZM8 4c.535 0 .954.462.9.995l-.35 3.507a.552.552 0 0 1-1.1 0l-.35-3.507A.905.905 0 0 1 8 4Zm.002 7a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z"></path></svg>"#,
        ),
    ]
    .into_iter()
    .fold(html.to_string(), |acc, (title, icon)| {
        acc.replace(
            &format!(r#"<p class="markdown-alert-title">{title}</p>"#),
            &format!(r#"<p class="markdown-alert-title">{icon}{title}</p>"#),
        )
    })
}

/// Class of the element that holds an HTML block shown as its source.
pub(super) const HTML_SOURCE_CLASS: &str = "doctty-html-source";

/// `restore_supported_raw_html` restores a tag only from its escaped `<`,
/// and decodes an entity only after an escaped `&`.
const ESCAPED_LT: &str = "&lt;";
const ESCAPED_AMP: &str = "&amp;";

/// Writes an HTML block that doctty shows as its source. Characters are
/// escaped as numeric references rather than [`ESCAPED_LT`] and
/// [`ESCAPED_AMP`], so `restore_supported_raw_html` restores no tag inside
/// and entities stay as written.
pub(super) fn write_html_source_block(
    output: &mut dyn std::fmt::Write,
    literal: &str,
) -> std::fmt::Result {
    write!(output, r#"<div class="{HTML_SOURCE_CLASS}">"#)?;
    write_escaped_text(output, literal.trim_end_matches(['\r', '\n']))?;
    output.write_str("</div>")
}

/// Escapes with numeric references, which `restore_supported_raw_html` leaves alone, so no
/// tag in `text` is turned back into markup.
pub(super) fn write_escaped_text(output: &mut dyn std::fmt::Write, text: &str) -> std::fmt::Result {
    for c in text.chars() {
        match c {
            '&' => output.write_str("&#38;")?,
            '<' => output.write_str("&#60;")?,
            '>' => output.write_str("&#62;")?,
            '"' => output.write_str("&#34;")?,
            '\0' => output.write_char('\u{fffd}')?,
            c => output.write_char(c)?,
        }
    }
    Ok(())
}

pub(super) fn restore_supported_raw_html(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    let mut inside_code_or_pre = false;
    let mut next = NextMatches::default();

    loop {
        let cursor = html.len() - rest.len();
        if inside_code_or_pre {
            // Look for the closing </code> or </pre> tag before doing anything else.
            if let Some(close_pos) = next.code_pre_close(html, cursor) {
                let end = close_pos.0 + close_pos.1;
                output.push_str(&rest[..end]);
                rest = &rest[end..];
                inside_code_or_pre = false;
                continue;
            }
            // No closing tag found — rest of document is inside code/pre.
            output.push_str(rest);
            return output;
        }

        // Find the next interesting position: either an escaped tag or a <code>/<pre> open.
        let escaped_pos = next.escaped_lt.at_or_after(html, cursor);
        let code_open = next.code_pre_open(html, cursor);

        match (escaped_pos, code_open) {
            (None, None) => {
                output.push_str(&decode_double_escaped_entities(rest));
                return output;
            }
            // A <code>/<pre> open comes before (or at) the next escaped tag.
            (_, Some((co_pos, co_len))) if escaped_pos.is_none_or(|ep| co_pos <= ep) => {
                let end = co_pos + co_len;
                output.push_str(&decode_double_escaped_entities(&rest[..end]));
                rest = &rest[end..];
                inside_code_or_pre = true;
            }
            // An escaped tag comes first — try to restore it.
            (Some(start), _) => {
                output.push_str(&decode_double_escaped_entities(&rest[..start]));
                let tag_segment = &rest[start..];
                let Some(end) = tag_segment.find("&gt;") else {
                    output.push_str(tag_segment);
                    return output;
                };
                let escaped_tag = &tag_segment[..end + "&gt;".len()];
                if let Some((_, restored)) = restore_supported_tag(escaped_tag) {
                    output.push_str(&restored);
                } else {
                    output.push_str(escaped_tag);
                }
                rest = &tag_segment[end + "&gt;".len()..];
            }
            _ => unreachable!(),
        }
    }
}

fn decode_double_escaped_entities(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some(offset) = text[cursor..].find(ESCAPED_AMP) {
        let marker = cursor + offset;
        output.push_str(&text[cursor..marker]);

        let entity_start = marker + ESCAPED_AMP.len();
        let remaining = &text[entity_start..];
        if let Some(entity_len) = escaped_entity_len(remaining) {
            output.push('&');
            output.push_str(&remaining[..entity_len]);
            cursor = entity_start + entity_len;
        } else {
            output.push_str(ESCAPED_AMP);
            cursor = entity_start;
        }
    }

    output.push_str(&text[cursor..]);
    output
}

fn escaped_entity_len(text: &str) -> Option<usize> {
    let candidate = text.strip_prefix('#').map_or(text, |_| &text[1..]);
    let is_numeric = text.starts_with('#');
    let is_hex = text.starts_with("#x") || text.starts_with("#X");

    if is_hex {
        let digits = text
            .strip_prefix("#x")
            .or_else(|| text.strip_prefix("#X"))?;
        let end = digits.find(';')?;
        if end == 0 || !digits[..end].chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        return Some(2 + end + 1);
    }

    if is_numeric {
        let end = candidate.find(';')?;
        if end == 0 || !candidate[..end].chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        return Some(1 + end + 1);
    }

    let end = text.find(';')?;
    let name = &text[..end];
    if name.is_empty() || !name.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    Some(end + 1)
}

/// The next occurrence of a needle at or after a position that only moves forward.
/// Searching again only once the scan has passed the remembered match keeps
/// `restore_supported_raw_html` linear in the length of Comrak's HTML.
struct NextMatch {
    needle: &'static str,
    /// Absolute position. `None` until searched; `Some(None)` once the needle
    /// does not occur after the last searched position.
    found: Option<Option<usize>>,
}

impl NextMatch {
    const fn new(needle: &'static str) -> Self {
        Self {
            needle,
            found: None,
        }
    }

    /// Returns the position relative to `from`, the start of the unscanned rest.
    fn at_or_after(&mut self, html: &str, from: usize) -> Option<usize> {
        if let Some(found) = self.found
            && found.is_none_or(|pos| pos >= from)
        {
            return found.map(|pos| pos - from);
        }
        let found = html[from..].find(self.needle).map(|offset| from + offset);
        self.found = Some(found);
        found.map(|pos| pos - from)
    }
}

/// The next `<code` or `<pre` opening tag, remembering its length so a long
/// tag ahead of the scan is not measured again on every step.
struct NextOpenTag {
    matcher: NextMatch,
    /// Absolute position of the measured match and its tag length, if it is a tag.
    measured: Option<(usize, Option<usize>)>,
}

impl NextOpenTag {
    const fn new(needle: &'static str) -> Self {
        Self {
            matcher: NextMatch::new(needle),
            measured: None,
        }
    }

    /// Returns the position relative to `from` and the byte length of the next
    /// match, or `None` when that match is not followed by a tag end.
    fn at_or_after(&mut self, html: &str, from: usize) -> Option<(usize, usize)> {
        let pos = from + self.matcher.at_or_after(html, from)?;
        let tag_len = match self.measured {
            Some((at, tag_len)) if at == pos => tag_len,
            _ => {
                let tag_len = self.tag_len(html, pos);
                self.measured = Some((pos, tag_len));
                tag_len
            }
        };
        tag_len.map(|tag_len| (pos - from, tag_len))
    }

    fn tag_len(&self, html: &str, pos: usize) -> Option<usize> {
        let after = pos + self.matcher.needle.len();
        if after < html.len() && !matches!(html.as_bytes()[after], b'>' | b' ' | b'\t' | b'\n') {
            return None;
        }
        html[pos..].find('>').map(|close| close + 1)
    }
}

struct NextMatches {
    escaped_lt: NextMatch,
    code_open: NextOpenTag,
    pre_open: NextOpenTag,
    code_close: NextMatch,
    pre_close: NextMatch,
}

impl Default for NextMatches {
    fn default() -> Self {
        Self {
            escaped_lt: NextMatch::new(ESCAPED_LT),
            code_open: NextOpenTag::new("<code"),
            pre_open: NextOpenTag::new("<pre"),
            code_close: NextMatch::new("</code>"),
            pre_close: NextMatch::new("</pre>"),
        }
    }
}

impl NextMatches {
    /// Find the start position and byte-length of the next `<code` or `<pre` opening tag.
    fn code_pre_open(&mut self, html: &str, from: usize) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize)> = None;
        for tag in [&mut self.code_open, &mut self.pre_open] {
            if let Some((pos, tag_len)) = tag.at_or_after(html, from)
                && best.is_none_or(|(bp, _)| pos < bp)
            {
                best = Some((pos, tag_len));
            }
        }
        best
    }

    /// Find the end position (start, byte-length) of the next `</code>` or `</pre>` closing tag.
    fn code_pre_close(&mut self, html: &str, from: usize) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize)> = None;
        for matcher in [&mut self.code_close, &mut self.pre_close] {
            if let Some(pos) = matcher.at_or_after(html, from)
                && best.is_none_or(|(bp, _)| pos < bp)
            {
                best = Some((pos, matcher.needle.len()));
            }
        }
        best
    }
}
