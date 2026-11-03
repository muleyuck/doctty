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
fn decodes_html_entities_in_html_blocks_like_a_browser() {
    let source = "<div>&copy; &times 3 &#x1F600; &#0; &unknown; a & b</div>\n";

    assert_eq!(
        paragraph_texts(source),
        vec!["\u{a9} \u{d7} 3 \u{1F600} \u{FFFD} &unknown; a & b"]
    );
}

#[test]
fn keeps_escaped_tags_in_html_blocks_as_text() {
    let source = "<div>&lt;b&gt;not a tag&lt;/b&gt;</div>\n";

    assert_eq!(paragraph_texts(source), vec!["<b>not a tag</b>"]);
}

#[test]
fn ends_a_tag_only_at_a_greater_than_sign_outside_quotes() {
    let source = "<p title=\"a>b\" data-x='c>d'>shown</p>\n";

    assert_eq!(paragraph_texts(source), vec!["shown"]);
}

#[test]
fn treats_html_tag_names_case_insensitively() {
    let source = "<DIV>Upper<BR>case<SCRIPT>hidden</Script></DIV>\n";

    assert_eq!(paragraph_texts(source), vec!["Upper\ncase"]);
}

#[test]
fn accepts_self_closing_br_with_a_space() {
    assert_eq!(
        paragraph_texts("<div>one<br />two</div>\n"),
        vec!["one\ntwo"]
    );
}

#[test]
fn keeps_a_less_than_sign_not_followed_by_a_tag_name() {
    let source = "<div>1 <2 and 3 < 4 and </ 5</div>\n";

    assert_eq!(paragraph_texts(source), vec!["1 <2 and 3 < 4 and </ 5"]);
}

#[test]
fn keeps_a_tag_with_an_unterminated_quote_as_text() {
    let source = "<div>open <span title=\"unterminated</div>\n";

    assert_eq!(
        paragraph_texts(source),
        vec!["open <span title=\"unterminated"]
    );
}

#[test]
fn drops_the_rest_of_an_html_block_after_an_unclosed_comment() {
    let source = "<div>kept <!-- never closed\nhidden</div>\n";

    assert_eq!(paragraph_texts(source), vec!["kept"]);
}

#[test]
fn collapses_nested_block_tags_into_one_line_break() {
    let source = "<div>\n  <div><p>first</p></div>\n</div>\n";

    assert_eq!(paragraph_texts(source), vec!["first"]);
    assert_eq!(
        paragraph_texts("<div><p>first</p><p>second</p></div>\n"),
        vec!["first\nsecond"]
    );
}

#[test]
fn keeps_multibyte_text_around_tags() {
    let source = "<div>日本語<b>太字</b>テキスト</div>\n";

    assert_eq!(paragraph_texts(source), vec!["日本語太字テキスト"]);
}

#[test]
fn emits_no_block_for_an_html_block_without_text() {
    assert_eq!(
        paragraph_texts("<div>   </div>\n\n<p><!-- only a comment --></p>\n"),
        Vec::<String>::new()
    );
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
        paragraph_texts("Logo <img src=\"logo.png\" alt=\"A &amp; B\"> here.\n"),
        vec!["Logo A & B here."]
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
fn ends_script_only_at_its_own_closing_tag() {
    let source = "<div>a<script>x</scriptx>y</SCRIPT >b</div>\n";

    assert_eq!(paragraph_texts(source), vec!["ab"]);
}

#[test]
fn treats_quotes_as_quotes_only_in_attribute_values() {
    assert_eq!(paragraph_texts("<p data-it's>shown</p>\n"), vec!["shown"]);
}

#[test]
fn keeps_a_tag_like_run_cut_short_by_another_less_than_sign_as_text() {
    assert_eq!(
        paragraph_texts("<div>if x <y then</div>\n"),
        vec!["if x <y then"]
    );
}

#[test]
fn strips_long_blocks_of_unclosed_tags_and_quotes() {
    for unit in ["x <a ", "<a b=\"", "<a b='", "<a\"=\"", "<A \"x="] {
        for tail in ["", "\"<x"] {
            let source = format!("<div>{}{tail}</div>\n", unit.repeat(20_000));
            assert_eq!(paragraph_texts(&source).len(), 1, "unit: {unit}");
        }
    }
    for unit in ["<script>", "<style></stylex "] {
        let source = format!("<div>{}tail</div>\n", unit.repeat(20_000));
        assert_eq!(
            paragraph_texts(&source),
            Vec::<String>::new(),
            "unit: {unit}"
        );
    }
}

#[test]
fn trims_spaces_after_br_inside_emphasis_or_before_a_comment() {
    assert_eq!(
        paragraph_texts("*a<br>*   b\n\nc<br><!-- x -->   d\n"),
        vec!["a\nb", "c\nd"]
    );
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

#[test]
fn keeps_tags_after_an_unclosed_quote_working() {
    assert_eq!(
        paragraph_texts("<div>a <span title=\"open</span> <b>b</b></div>\n"),
        vec!["a <span title=\"open b"]
    );
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
fn keeps_quoted_attributes_working_after_an_unbalanced_quote() {
    assert_eq!(
        paragraph_texts("<div>a <span title=\"open</span> <b title='a>b'>b</b></div>\n"),
        vec!["a <span title=\"open b"]
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

#[test]
fn keeps_non_ascii_whitespace_as_content() {
    assert_eq!(
        paragraph_texts("<div>日本語\u{3000}\u{3000}空白 &nbsp;x&nbsp;</div>\n"),
        vec!["日本語\u{3000}\u{3000}空白 \u{a0}x\u{a0}"]
    );

    let document = match parse_document("docs/example.md".into(), "# &nbsp;Heading&nbsp;\n") {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };
    assert!(matches!(
        &document.blocks[0].kind,
        BlockKind::Heading { text, .. } if text.plain() == "\u{a0}Heading\u{a0}"
    ));
}

#[test]
fn ends_script_only_at_a_closing_tag_followed_by_space_slash_or_gt() {
    assert_eq!(
        paragraph_texts("<div>a<script>hidden</script!>still hidden</script\t>b</div>\n"),
        vec!["ab"]
    );
}
