use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::core::config::MermaidMode;

use super::{
    mermaid::{decorate_mermaid_svg, render_mermaid_blocks_with},
    postprocess::{restore_supported_raw_html, write_html_source_block},
};

#[test]
fn decorate_mermaid_svg_adds_renderer_class() {
    let decorated = decorate_mermaid_svg(r#"<svg viewBox="0 0 10 10"><rect /></svg>"#);

    assert!(decorated.starts_with(r#"<svg class="doctty-mermaid-diagram""#));
    assert!(decorated.contains(r#"viewBox="0 0 10 10""#));
}

#[test]
fn render_mermaid_blocks_reuses_duplicate_sources() {
    let calls = Arc::new(AtomicUsize::new(0));
    let call_counter = Arc::clone(&calls);
    let sources = vec![
        "graph TD\n    A --> B\n".to_string(),
        "graph TD\n    A --> B\n".to_string(),
    ];

    let rendered = render_mermaid_blocks_with(&sources, MermaidMode::Enabled, move |content| {
        call_counter.fetch_add(1, Ordering::SeqCst);
        Ok(format!("<svg>{}</svg>", content.trim()))
    })
    .unwrap_or_else(|error| panic!("render should succeed: {error}"));

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(rendered.len(), 1);
    assert!(
        rendered
            .values()
            .next()
            .is_some_and(|html| html.contains("doctty-mermaid"))
    );
}

#[test]
fn failed_mermaid_render_shows_the_cli_error() {
    let sources = vec!["graph TD\n    A --> B\n".to_string()];

    let rendered = render_mermaid_blocks_with(&sources, MermaidMode::Enabled, |_| {
        Err(anyhow::anyhow!(
            "mermaid renderer exited with exit status: 1: error: unknown option '-w'\n\
             \n   \n<div>line 2</div>\nline 3\nline 4"
        ))
    })
    .unwrap_or_else(|error| panic!("a failed diagram should not fail the page: {error}"));
    let html = restore_supported_raw_html(&rendered[&sources[0]]);

    assert!(html.contains("Mermaid unavailable"));
    assert!(html.contains("error: unknown option '-w'"));
    assert!(html.contains("&#60;div&#62;line 2&#60;/div&#62;"));
    assert!(html.contains("line 3"));
    assert!(!html.contains("line 4"));
}

#[test]
fn restoring_a_written_source_block_leaves_it_unchanged() {
    let literal = "<div align=\"center\">\n<br> <a href=\"https://x.y\">a</a> &amp; &lt;b&gt; &#60; &copy;\n</div>\n";
    let mut written = String::new();
    write_html_source_block(&mut written, literal)
        .unwrap_or_else(|error| panic!("writing should succeed: {error}"));

    assert_eq!(restore_supported_raw_html(&written), written);
}

#[test]
fn restoring_tags_keeps_code_and_pre_contents_escaped() {
    let html = concat!(
        "<p>&lt;sub&gt;x&lt;/sub&gt; <codex>&lt;b&gt;y&lt;/b&gt;",
        "<code>&lt;b&gt;in&lt;/b&gt;</code> &lt;kbd&gt;k&lt;/kbd&gt;</p>",
        "<pre lang=\"rs\"><code>&lt;b&gt;</code></pre>&lt;i&gt;b&lt;/i&gt;",
        "<prefix>&lt;sup&gt;1&lt;/sup&gt;<pre>&lt;i&gt;</pre>",
    );

    assert_eq!(
        restore_supported_raw_html(html),
        concat!(
            "<p><sub>x</sub> <codex><b>y</b>",
            "<code>&lt;b&gt;in&lt;/b&gt;</code> <kbd>k</kbd></p>",
            "<pre lang=\"rs\"><code>&lt;b&gt;</code></pre><i>b</i>",
            "<prefix><sup>1</sup><pre>&lt;i&gt;</pre>",
        )
    );
}

#[test]
fn restoring_tags_handles_unterminated_and_skipped_code() {
    // A `<code` that ends the document, an unclosed `<code>`, and a `<code>`
    // inside an escaped span that is kept as text.
    for (html, expected) in [
        ("&lt;b&gt;x<code", "<b>x<code"),
        ("&lt;i&gt;a<code>&lt;b&gt;", "<i>a<code>&lt;b&gt;"),
        (
            "&lt;b <code>x</code> &gt;&lt;i&gt;y<code>&lt;b&gt;</code>",
            "&lt;b <code>x</code> &gt;<i>y<code>&lt;b&gt;</code>",
        ),
    ] {
        assert_eq!(restore_supported_raw_html(html), expected, "input: {html}");
    }
}
