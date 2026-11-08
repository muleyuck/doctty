//! Raw HTML written in Markdown: which tags doctty supports, and how each
//! view shows them. The plain-text normalizer and the rich view both depend
//! on this module and not on each other, so supporting a new tag changes
//! only this module.

mod scan;
mod tag;
mod text;

pub(crate) use self::{
    tag::restore_supported_tag,
    text::{InlineHtmlState, collect_inline_html, html_to_text, is_html_source_block},
};
