//! Finding where a raw HTML tag ends.

/// The first `len` bytes cannot start a tag anywhere, so callers keep them as
/// text without scanning them again.
pub(super) struct NotATag {
    pub(super) len: usize,
}

/// Byte offsets of the last `"` and `'` in the input. A quote opened there
/// has nothing to close it, so it is read as an ordinary character; this
/// keeps one unbalanced quote from swallowing the rest of the block.
#[derive(Clone, Copy)]
pub(super) struct QuoteEnds {
    double: Option<usize>,
    single: Option<usize>,
}

impl QuoteEnds {
    pub(super) fn of(html: &str) -> Self {
        Self {
            double: html.rfind('"'),
            single: html.rfind('\''),
        }
    }

    pub(super) fn skip(self, bytes: usize) -> Self {
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
pub(super) fn html_tag_len(html: &str, quote_ends: QuoteEnds) -> Result<usize, NotATag> {
    let rest = html.strip_prefix('<').ok_or(NotATag { len: 1 })?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(NotATag { len: 1 });
    }
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(rest.len());

    let attrs_start = html.len() - rest.len() + name_len;
    let mut quote = None;
    let mut after_equals = false;
    for (offset, c) in html[attrs_start..].char_indices() {
        let at = attrs_start + offset;
        if let Some(open) = quote {
            if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '>' => return Ok(at + 1),
            '<' => return Err(NotATag { len: at }),
            '"' | '\'' if after_equals && quote_ends.can_close(c, at) => quote = Some(c),
            _ => {}
        }
        if !c.is_ascii_whitespace() {
            after_equals = c == '=';
        }
    }
    Err(NotATag { len: html.len() })
}
