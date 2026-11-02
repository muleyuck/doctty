use doctty::core::document::BlockKind;
use doctty::render::markdown::parse_document;

#[test]
fn normalizes_common_blocks_from_gfm_markdown() {
    let source = r#"# Heading

Paragraph with a [link](https://example.com).

> [!NOTE]
> Callout body

```mermaid
graph TD
    A --> B
```
"#;

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(matches!(
        document.blocks[0].kind,
        BlockKind::Heading { level: 1, .. }
    ));
    assert!(matches!(
        document.blocks[1].kind,
        BlockKind::Paragraph { .. }
    ));
    assert!(matches!(document.blocks[2].kind, BlockKind::Callout { .. }));
    assert!(matches!(document.blocks[3].kind, BlockKind::Mermaid { .. }));
    assert_eq!(document.meta.links, vec!["https://example.com".to_string()]);
}

#[test]
fn normalizes_task_list_and_footnote_reference_text() {
    let source = "- [x] done\n- [ ] pending\n\nFootnote ref[^1].\n\n[^1]: detail\n";

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(matches!(
        document.blocks[0].kind,
        BlockKind::List { ref items, .. }
            if items
                == &vec![
                    doctty::core::document::StyledText::from_plain("[x] done"),
                    doctty::core::document::StyledText::from_plain("[ ] pending"),
                ]
    ));
    assert!(matches!(
        document.blocks[1].kind,
        BlockKind::Paragraph { ref text } if text.plain().contains("[^1]")
    ));
    assert!(matches!(
        document.blocks[2].kind,
        BlockKind::Footnote { .. }
    ));
}

#[test]
fn skips_layout_only_html_blocks() {
    let source = "<div align=\"center\">\n</div>\n";

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(
        document.blocks.is_empty(),
        "layout-only HTML blocks (<div>, </div>) should be skipped, got: {:?}",
        document.blocks.iter().map(|b| &b.kind).collect::<Vec<_>>()
    );
}

fn paragraph_texts(source: &str) -> Vec<String> {
    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };
    document
        .blocks
        .iter()
        .map(|block| match &block.kind {
            BlockKind::Paragraph { text } => text.plain(),
            other => panic!("expected only paragraphs, got: {other:?}"),
        })
        .collect()
}

#[test]
fn strips_tags_from_html_blocks_and_keeps_their_text() {
    let source = "<table>\n  <tr><th>Name</th><th>Value</th></tr>\n  <tr>\n    <td>a &amp; b</td>\n    <td>1 &lt; 2 &#x2713;</td>\n  </tr>\n</table>\n";

    assert_eq!(
        paragraph_texts(source),
        vec!["Name Value\na & b 1 < 2 \u{2713}".to_string()]
    );
}

#[test]
fn keeps_details_summary_text_and_drops_the_closing_tag() {
    let source = "<details><summary>Click me</summary>\n\nbody\n\n</details>\n";

    assert_eq!(paragraph_texts(source), vec!["Click me", "body"]);
}

#[test]
fn drops_html_comments_and_script_or_style_contents() {
    let source = "<!-- hidden -->\n\n<style>.x { color: red }</style>\n\n<p>shown<script>alert(1)</script></p>\n";

    assert_eq!(paragraph_texts(source), vec!["shown"]);
}

#[test]
fn keeps_a_less_than_sign_that_does_not_open_a_tag() {
    let source = "<div>\na < b and c > d\n</div>\n";

    assert_eq!(paragraph_texts(source), vec!["a < b and c > d"]);
}

#[test]
fn parses_html_img_tag_as_image_block() {
    let source = "<img src=\"photo.png\" alt=\"A photo\" />\n";

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(matches!(
        document.blocks.as_slice(),
        [doctty::core::document::Block {
            kind: BlockKind::Image { src, alt, .. },
            ..
        }] if src == "photo.png" && alt == "A photo"
    ));
}

#[test]
fn parses_html_img_wrapped_in_p_as_image_block() {
    let source = "<p align=\"center\">\n  <img src=\"hero.jpg\" alt=\"Centered\" />\n</p>\n";

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(matches!(
        document.blocks.as_slice(),
        [doctty::core::document::Block {
            kind: BlockKind::Image { src, alt, .. },
            ..
        }] if src == "hero.jpg" && alt == "Centered"
    ));
}

#[test]
fn parses_html_img_wrapped_in_div_as_image_block() {
    let source = "<div align=\"center\">\n<img src=\"banner.png\" alt=\"Banner\" />\n</div>\n";

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(matches!(
        document.blocks.as_slice(),
        [doctty::core::document::Block {
            kind: BlockKind::Image { src, alt, .. },
            ..
        }] if src == "banner.png" && alt == "Banner"
    ));
}

#[test]
fn parses_html_img_tag_with_width_as_image_block() {
    let source = "<img src=\"hero.jpg\" width=\"700\" alt=\"Hero\" />\n";

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(matches!(
        document.blocks.as_slice(),
        [doctty::core::document::Block {
            kind: BlockKind::Image { src, alt, .. },
            ..
        }] if src == "hero.jpg" && alt == "Hero"
    ));
}

#[test]
fn turns_inline_br_into_a_line_break() {
    assert_eq!(
        paragraph_texts("Before <br/> after.\n"),
        vec!["Before\nafter."]
    );
}

#[test]
fn strips_inline_tags_and_keeps_their_text() {
    assert_eq!(
        paragraph_texts("Press <kbd>Ctrl</kbd>+<kbd>C</kbd><!-- note -->.\n"),
        vec!["Press Ctrl+C."]
    );
}

#[test]
fn replaces_an_inline_img_with_its_alt_text() {
    assert_eq!(
        paragraph_texts("Logo <img src=\"logo.png\" alt=\"doctty\"> here.\n"),
        vec!["Logo doctty here."]
    );
}

#[test]
fn preserves_math_text_in_plain_document_normalization() {
    let source = "Inline $e^{i\\pi} + 1 = 0$ and $$a^2 + b^2 = c^2$$.\n";

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(matches!(
        document.blocks.as_slice(),
        [doctty::core::document::Block {
            kind: BlockKind::Paragraph { text },
            ..
        }] if text.plain() == "Inline $e^{i\\pi} + 1 = 0$ and $$a^2 + b^2 = c^2$$."
    ));
}
