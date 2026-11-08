use doctty::{
    cli::Theme,
    core::config::MermaidMode,
    render::{markdown::parse_document, text::render_plain_text},
};

#[test]
fn renders_links_inline_with_destination() {
    let document = match parse_document(
        "docs/example.md".into(),
        "Paragraph with a [link](https://example.com).\n",
    ) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    let rendered = render_plain_text(&document, Theme::Light, MermaidMode::Enabled);

    assert!(rendered.contains("link <https://example.com>"));
}

#[test]
fn renders_image_state_and_mermaid_unavailable_reason() {
    let document = match parse_document(
        "docs/example.md".into(),
        "![diagram](docs/missing.png)\n\n```mermaid\ngraph TD\n    A --> B\n```\n",
    ) {
        Ok(document) => document,
        Err(error) => panic!("document should parse: {error}"),
    };

    let rendered = render_plain_text(&document, Theme::Light, MermaidMode::Enabled);

    assert!(rendered.contains("[Image missing: diagram"));
    assert!(rendered.contains("[Mermaid"));
}

fn render(source: &str) -> String {
    let document = parse_document("docs/example.md".into(), source)
        .unwrap_or_else(|error| panic!("document should parse: {error}"));
    render_plain_text(&document, Theme::Light, MermaidMode::Disabled)
}

#[test]
fn keeps_code_and_html_blocks_nested_in_quotes_and_lists() {
    let source = "> quote\n>\n> ```\n> code\n> ```\n>\n> <div>\n> <p>supported</p>\n> </div>\n\n- item\n\n  <!-- x -->\n";

    assert_eq!(
        render(source),
        "> quote\n>\n> ```\n> code\n> ```\n>\n> supported\n\n- item\n\n  <!-- x -->\n"
    );
}

#[test]
fn separates_paragraphs_inside_a_block_quote() {
    assert_eq!(render("> one\n>\n> two\n"), "> one\n>\n> two\n");
}

#[test]
fn indents_nested_lists_and_continuation_lines_under_their_item() {
    let source = "- [x] done\n- two\n  - nested\n\n1. first\n   continued\n\n   ```\n   c\n   ```\n2. second\n";

    assert_eq!(
        render(source),
        "- [x] done\n- two\n  - nested\n\n1. first\n   continued\n\n   ```\n   c\n   ```\n\n2. second\n"
    );
}

#[test]
fn keeps_every_block_of_a_callout_and_a_footnote() {
    let source = "> [!TIP]\n> one\n>\n> ```\n> c\n> ```\n\nRef[^n].\n\n[^n]: first\n\n    second\n";

    assert_eq!(
        render(source),
        "[Tip] one\n\n      ```\n      c\n      ```\n\nRef[^n].\n\n[^n] first\n\n     second\n"
    );
}
