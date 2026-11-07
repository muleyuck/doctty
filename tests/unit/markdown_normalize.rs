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

#[test]
fn turns_br_into_a_space_inside_table_cells_and_headings() {
    let source = "# Title<br>sub\n\n| a | b |\n|---|---|\n| x <br> y | z |\n";

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(matches!(
        &document.blocks[0].kind,
        BlockKind::Heading { text, .. } if text.plain() == "Title sub"
    ));
    assert!(matches!(
        &document.blocks[1].kind,
        BlockKind::Table { rows }
            if rows[1].iter().map(|cell| cell.plain()).collect::<Vec<_>>() == vec!["x y", "z"]
    ));
}

#[test]
fn keeps_leading_spaces_after_a_soft_break() {
    assert_eq!(paragraph_texts("a\n&#32;&#32;b\n"), vec!["a\n  b"]);
}

#[test]
fn drops_spaces_left_by_a_trailing_br_in_headings_and_cells() {
    let source = "# H <br>\n\n| x<br> | y |\n|---|---|\n";

    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    assert!(matches!(
        &document.blocks[0].kind,
        BlockKind::Heading { text, .. } if text.plain() == "H"
    ));
    assert!(matches!(
        &document.blocks[1].kind,
        BlockKind::Table { rows }
            if rows[0].iter().map(|cell| cell.plain()).collect::<Vec<_>>() == vec!["x", "y"]
    ));
}

fn table_rows(source: &str) -> Vec<Vec<String>> {
    let document = match parse_document("docs/example.md".into(), source) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };
    match &document.blocks[0].kind {
        BlockKind::Table { rows } => rows
            .iter()
            .map(|row| row.iter().map(|cell| cell.plain()).collect())
            .collect(),
        other => panic!("expected a table, got: {other:?}"),
    }
}

#[test]
fn keeps_empty_table_cells_so_columns_stay_aligned() {
    let source = "| Name | Note | Count |\n|---|---|---|\n| Alice | <br> | 3 |\n| Bob |  | 4 |\n";

    assert_eq!(
        table_rows(source),
        vec![
            vec!["Name", "Note", "Count"],
            vec!["Alice", "", "3"],
            vec!["Bob", "", "4"],
        ]
    );
}

#[test]
fn trims_heading_edges_across_styled_segments_but_not_code_spans() {
    let headings = |source: &str| match parse_document("docs/example.md".into(), source) {
        Ok(document) => match &document.blocks[0].kind {
            BlockKind::Heading { text, .. } => text.plain(),
            other => panic!("expected a heading, got: {other:?}"),
        },
        Err(error) => panic!("document should parse: {error}"),
    };

    assert_eq!(headings("# H<br>*<br>*\n"), "H");
    assert_eq!(headings("# *<br>*<br>H\n"), "H");
    assert_eq!(headings("# `  x  `\n"), " x ");
    assert_eq!(headings("# `  x  `<br>\n"), " x ");
    assert_eq!(
        headings(&format!("# {}H\n", "*<br>*<br>".repeat(20_000))),
        "H"
    );
}

fn block_kinds(source: &str) -> Vec<BlockKind> {
    match parse_document("docs/example.md".into(), source) {
        Ok(document) => document
            .blocks
            .into_iter()
            .map(|block| block.kind)
            .collect(),
        Err(error) => panic!("document should parse: {error}"),
    }
}

#[test]
fn keeps_a_block_with_only_a_linked_image_as_an_image() {
    assert!(matches!(
        block_kinds("<p align=\"center\"><a href=\"https://x.dev\"><img src=\"a.png\" alt=\"A\"></a></p>\n").as_slice(),
        [BlockKind::Image { src, alt, .. }] if src == "a.png" && alt == "A"
    ));
}

#[test]
fn renders_supported_block_tags_and_decodes_their_text() {
    let source = "<div align=\"center\">\n  <h1>Project</h1>\n  <p>a &amp; b &copy; 日本語\u{3000}\u{3000}x</p>\n</div>\n";

    assert_eq!(
        paragraph_texts(source),
        vec!["Project\na & b \u{a9} 日本語\u{3000}\u{3000}x"]
    );
}

#[test]
fn keeps_text_and_links_of_a_readme_hero_block_with_an_image() {
    let source = "<div align=\"center\">\n  <img src=\"logo.png\" alt=\"Logo\">\n  <h1>Project</h1>\n  <p><a href=\"https://ci.example\"><img src=\"badge.svg\" alt=\"CI\"></a> <a href=\"https://example.com/docs\">Docs</a></p>\n</div>\n";

    assert_eq!(
        paragraph_texts(source),
        vec!["Logo\nProject\nCI <https://ci.example> Docs <https://example.com/docs>"]
    );
}

#[test]
fn renders_details_and_summary() {
    assert_eq!(
        paragraph_texts("<details><summary>Click me</summary>\n\nbody\n\n</details>\n"),
        vec!["Click me", "body"]
    );
}

#[test]
fn shows_a_block_without_supported_tags_as_its_source() {
    let source = "<table>\n  <tr><td>a &amp; b</td></tr>\n</table>\n";

    assert_eq!(
        paragraph_texts(source),
        vec!["<table>\n  <tr><td>a &amp; b</td></tr>\n</table>"]
    );
    for source in [
        "<ul>\n  <li>first</li>\n</ul>\n",
        "<pre>\n  code\n</pre>\n",
        "<hr>\n",
        "<!-- note -->\n",
        "<script>alert(1)</script>\n",
        "<style>.x { color: red }</style>\n",
    ] {
        assert_eq!(
            paragraph_texts(source),
            vec![source.trim_end().to_string()],
            "source: {source}"
        );
    }
}

#[test]
fn keeps_unsupported_tags_and_comments_as_source_next_to_supported_ones() {
    assert_eq!(
        paragraph_texts("<p>Press <kbd>Ctrl</kbd> <!-- note --> <script>x()</script></p>\n"),
        vec!["Press <kbd>Ctrl</kbd> <!-- note --> <script>x()</script>"]
    );
}

#[test]
fn keeps_unsupported_inline_tags_and_comments_as_source() {
    assert_eq!(
        paragraph_texts("Press <kbd>Ctrl</kbd>+<kbd>C</kbd><!-- note -->. H<sub>2</sub>O\n"),
        vec!["Press <kbd>Ctrl</kbd>+<kbd>C</kbd><!-- note -->. H2O"]
    );
}

#[test]
fn supports_exactly_the_tags_the_rich_view_restores() {
    // The rich view restores only lower-case tags with double-quoted
    // attribute values, so plain-text output treats the others as source.
    for (source, expected) in [
        ("<DIV>x</DIV>", "<DIV>x</DIV>"),
        // `</p>` alone is restored there, like here.
        ("<p align=\"justify\">x</p>", "<p align=\"justify\">x"),
        ("<img src='a.png' alt='A'>", "<img src='a.png' alt='A'>"),
        ("<a name=\"x\">x</a>", "<a name=\"x\">x"),
    ] {
        assert_eq!(
            paragraph_texts(&format!("{source}\n")),
            vec![expected.to_string()],
            "source: {source}"
        );
    }
}

#[test]
fn prints_the_url_of_an_html_link_like_a_markdown_link() {
    assert_eq!(
        paragraph_texts("See <a href=\"https://example.com\">the site</a>.\n"),
        vec!["See the site <https://example.com>."]
    );
    assert_eq!(
        paragraph_texts("<p>See <a href=\"https://example.com?a=1&amp;b=2\">docs</a>.</p>\n"),
        vec!["See docs <https://example.com?a=1&b=2>."]
    );
}

#[test]
fn replaces_an_inline_img_with_its_decoded_alt_text() {
    assert_eq!(
        paragraph_texts("Logo <img src=\"logo.png\" alt=\"A &amp; B\"> here.\n"),
        vec!["Logo A & B here."]
    );
}

#[test]
fn keeps_alt_text_of_every_image_in_a_block() {
    assert_eq!(
        paragraph_texts("<p><img src=\"a.png\" alt=\"A\"> <img src=\"b.png\" alt=\"B\"></p>\n"),
        vec!["A B"]
    );
}

#[test]
fn trims_spaces_after_br_inside_emphasis() {
    assert_eq!(paragraph_texts("*a<br>*   b\n"), vec!["a\nb"]);
}

#[test]
fn keeps_less_than_signs_and_broken_tags_as_text() {
    assert_eq!(
        paragraph_texts("<div>1 <2 and 3 < 4 and </ 5 <span title=\"open</div>\n"),
        vec!["1 <2 and 3 < 4 and </ 5 <span title=\"open"]
    );
}

#[test]
fn strips_long_blocks_of_unclosed_tags_and_quotes() {
    for unit in [
        "x <a ", "<a b=\"", "<a b='", "<a\"=\"", "<A \"x=", "<kbd>", "<!--",
    ] {
        for tail in ["", "\"<x"] {
            let source = format!("<div>{}{tail}</div>\n", unit.repeat(20_000));
            assert_eq!(paragraph_texts(&source).len(), 1, "unit: {unit}");
        }
    }
}

#[test]
fn supports_an_image_whose_alt_text_contains_a_less_than_sign() {
    assert_eq!(
        paragraph_texts(
            "Plot <img src=\"plot.png\" alt=\"x &lt; y\"> and <img src=\"b.png\" alt=\"a < b\">.\n"
        ),
        vec!["Plot x < y and a < b."]
    );
}

#[test]
fn starts_a_new_line_for_block_tags_inside_a_paragraph() {
    assert_eq!(
        paragraph_texts("one<p>two</p>three\n"),
        vec!["one\ntwo\nthree"]
    );
}

#[test]
fn keeps_the_indentation_of_unsupported_source_inside_supported_html() {
    assert_eq!(
        paragraph_texts("<div><!-- a\n  b --><span\n    title=\"x\">y</span></div>\n"),
        vec!["<!-- a\n  b --><span\n    title=\"x\">y</span>"]
    );
}

#[test]
fn keeps_blank_lines_made_by_consecutive_br_tags() {
    assert_eq!(paragraph_texts("<div>a<br><br>b</div>\n"), vec!["a\n\nb"]);
}

#[test]
fn handles_a_tag_with_many_attribute_flags() {
    let source = format!("<div {}>x</div>\n", "open ".repeat(20_000));

    assert_eq!(paragraph_texts(&source), vec!["x"]);
}

#[test]
fn does_not_stack_line_breaks_from_inline_block_tags() {
    assert_eq!(paragraph_texts("one<div></div>two\n"), vec!["one\ntwo"]);
    assert_eq!(paragraph_texts("one<div>\ntwo\n"), vec!["one\ntwo"]);
    assert_eq!(paragraph_texts("<p>one</p>\n"), vec!["one"]);
    assert_eq!(paragraph_texts("a<br><br>b\n"), vec!["a\n\nb"]);
    assert_eq!(
        paragraph_texts("one<div><img src=\"a\" alt=\" \"></div>\ntwo\n"),
        vec!["one\ntwo"]
    );
    assert_eq!(
        paragraph_texts("one<div><img src=\"a\" alt=\" \"></div>  \ntwo\n"),
        vec!["one\ntwo"]
    );
    assert_eq!(
        paragraph_texts("one<div><img src=\"a\" alt=\" \"></div>   two\n"),
        vec!["one\ntwo"]
    );
    assert_eq!(
        paragraph_texts("one<br><img src=\"a\" alt=\" \"><br> two\n"),
        vec!["one\n\ntwo"]
    );

    let document = match parse_document(
        "docs/example.md".into(),
        "# one<div><div>two</div></div>three\n",
    ) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };
    assert!(matches!(
        &document.blocks[0].kind,
        BlockKind::Heading { text, .. } if text.plain() == "one two three"
    ));
}
