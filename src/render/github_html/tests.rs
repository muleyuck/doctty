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
fn restoring_a_written_source_block_leaves_it_unchanged() {
    let literal = "<div align=\"center\">\n<br> <a href=\"https://x.y\">a</a> &amp; &lt;b&gt; &#60; &copy;\n</div>\n";
    let mut written = String::new();
    write_html_source_block(&mut written, literal)
        .unwrap_or_else(|error| panic!("writing should succeed: {error}"));

    assert_eq!(restore_supported_raw_html(&written), written);
}
