use doctty::{
    cli::Theme,
    core::config::MermaidMode,
    render::{
        markdown::parse_document,
        text::{RenderedGraphicContent, RenderedInlineStyle, RenderedLineKind, render_document},
    },
};

#[test]
fn terminal_render_strips_raw_markdown_markers_from_headings_and_quotes() {
    let document = parse_document(
        "docs/example.md".into(),
        "# Title\n\n> quoted text\n\n```rust\nfn main() {}\n```\n",
    )
    .unwrap_or_else(|error| panic!("document should parse: {error}"));

    let rendered = render_document(&document, Theme::Light, MermaidMode::Enabled, 40, 0.5);

    assert_eq!(rendered.lines[0].display_text, "Title");
    assert_eq!(
        rendered.lines[0].kind,
        RenderedLineKind::Heading { level: 1 }
    );
    assert_eq!(rendered.lines[2].display_text, "│ quoted text");
    assert_eq!(rendered.lines[2].kind, RenderedLineKind::Quote);
    assert_eq!(rendered.lines[4].display_text, "rust");
    assert_eq!(
        rendered.lines[4].kind,
        RenderedLineKind::Code {
            language: Some("rust".to_string()),
            is_fence_delimiter: true
        }
    );
}

#[test]
fn terminal_render_wraps_long_lines_to_view_width() {
    let document = parse_document(
        "docs/example.md".into(),
        "This paragraph is intentionally long so it must wrap inside the terminal viewer.\n",
    )
    .unwrap_or_else(|error| panic!("document should parse: {error}"));

    let rendered = render_document(&document, Theme::Light, MermaidMode::Enabled, 20, 0.5);

    assert!(rendered.lines.len() > 2);
    assert!(rendered.lines[0].display_text.chars().count() <= 20);
    assert!(rendered.lines[1].display_text.chars().count() <= 20);
}

#[test]
fn terminal_render_carries_code_language_into_code_lines() {
    let document = parse_document("docs/example.md".into(), "```rust\nlet answer = 42;\n```\n")
        .unwrap_or_else(|error| panic!("document should parse: {error}"));

    let rendered = render_document(&document, Theme::Light, MermaidMode::Enabled, 40, 0.5);

    assert_eq!(
        rendered.lines[1].kind,
        RenderedLineKind::Code {
            language: Some("rust".to_string()),
            is_fence_delimiter: false
        }
    );
}

#[test]
fn terminal_render_formats_lists_links_and_tables_for_interactive_view() {
    let document = parse_document(
        "docs/example.md".into(),
        "Paragraph with a [link](https://example.com).\n\n- bullet item\n- [x] done\n- [ ] pending\n\n| Name | Value |\n| --- | --- |\n| alpha | beta |\n",
    )
    .unwrap_or_else(|error| panic!("document should parse: {error}"));

    let rendered = render_document(&document, Theme::Light, MermaidMode::Enabled, 48, 0.5);
    let display_lines = rendered
        .lines
        .iter()
        .map(|line| line.display_text.clone())
        .collect::<Vec<_>>();

    assert!(
        display_lines
            .iter()
            .any(|line| line.contains("Paragraph with a link."))
    );
    assert!(
        display_lines
            .iter()
            .all(|line| !line.contains("<https://example.com>"))
    );
    assert!(
        display_lines
            .iter()
            .any(|line| line.contains("• bullet item"))
    );
    assert!(display_lines.iter().any(|line| line.contains("☑ done")));
    assert!(display_lines.iter().any(|line| line.contains("☐ pending")));
    assert!(display_lines.iter().any(|line| line.contains("┌")));
    assert!(display_lines.iter().any(|line| line.contains("Name")));
    assert!(display_lines.iter().any(|line| line.contains("alpha")));
}

#[test]
fn terminal_render_defers_mermaid_rasterization_for_interactive_view() {
    let document = parse_document(
        "docs/example.md".into(),
        "```mermaid\ngraph TD\n    A --> B\n```\n",
    )
    .unwrap_or_else(|error| panic!("document should parse: {error}"));

    let rendered = render_document(&document, Theme::Light, MermaidMode::Enabled, 48, 0.5);
    let Some(graphic) = rendered.graphics.first() else {
        panic!("mermaid graphic should exist");
    };

    assert!(matches!(
        &graphic.content,
        RenderedGraphicContent::Mermaid {
            png_bytes: None,
            ..
        }
    ));
}

#[test]
fn terminal_render_preserves_inline_emphasis_segments() {
    let document = parse_document(
        "docs/example.md".into(),
        "Paragraph with **bold**, *italic*, and `code`.\n",
    )
    .unwrap_or_else(|error| panic!("document should parse: {error}"));

    let rendered = render_document(&document, Theme::Light, MermaidMode::Enabled, 80, 0.5);
    let paragraph = rendered
        .lines
        .iter()
        .find(|line| matches!(line.kind, RenderedLineKind::Paragraph))
        .unwrap_or_else(|| panic!("paragraph line should exist"));

    assert_eq!(
        paragraph.display_text.trim_end(),
        "Paragraph with bold, italic, and code."
    );
    assert!(
        paragraph
            .spans
            .iter()
            .any(|span| span.style == RenderedInlineStyle::BOLD)
    );
    assert!(
        paragraph
            .spans
            .iter()
            .any(|span| span.style == RenderedInlineStyle::ITALIC)
    );
    assert!(
        paragraph
            .spans
            .iter()
            .any(|span| span.style == RenderedInlineStyle::CODE)
    );
}

fn display_lines(source: &str, mermaid_mode: MermaidMode) -> Vec<(String, RenderedLineKind)> {
    let document = parse_document("docs/example.md".into(), source)
        .unwrap_or_else(|error| panic!("document should parse: {error}"));
    render_document(&document, Theme::Light, mermaid_mode, 40, 0.5)
        .lines
        .into_iter()
        .map(|line| (line.display_text, line.kind))
        .collect()
}

#[test]
fn terminal_render_keeps_each_block_of_a_quote_behind_the_bar() {
    let code = RenderedLineKind::Code {
        language: None,
        is_fence_delimiter: false,
    };

    let fence = |label: &str| {
        (
            label.to_string(),
            RenderedLineKind::Code {
                language: None,
                is_fence_delimiter: true,
            },
        )
    };

    let lines = display_lines("> one\n>\n> ```\n> c\n> ```\n", MermaidMode::Disabled);

    assert_eq!(lines[0], ("│ one".to_string(), RenderedLineKind::Quote));
    assert_eq!(lines[1], ("│".to_string(), RenderedLineKind::Quote));
    // The code frame is drawn from the line's own text, and an empty closing
    // delimiter is what draws the bottom border, so code lines get no prefix.
    assert_eq!(
        lines[2..5],
        [fence("code"), ("c".to_string(), code), fence("")]
    );
}

#[test]
fn terminal_render_indents_ordered_item_children_by_the_marker_width() {
    let lines = display_lines("1. parent\n   > child\n", MermaidMode::Disabled);

    assert_eq!(lines[0].0, "1. parent");
    assert_eq!(lines[1].0, "   │ child");
}

#[test]
fn terminal_render_keeps_the_checkbox_of_an_image_only_task_item() {
    let lines = display_lines("- [x] ![alt](missing.png)\n", MermaidMode::Disabled);

    assert_eq!(lines[0].0, "• ☑");
    assert_eq!(lines[1].0, "  Image unavailable: alt");
}

#[test]
fn terminal_render_indents_a_nested_list_under_its_item() {
    let lines = display_lines("- [x] done\n- two\n  - nested\n", MermaidMode::Disabled);

    assert_eq!(lines[0].0, "• ☑ done");
    assert_eq!(lines[1].0, "• two");
    assert_eq!(lines[2], ("  • nested".to_string(), RenderedLineKind::List));
}

#[test]
fn terminal_render_starts_each_callout_paragraph_at_the_bar() {
    let tip = RenderedLineKind::Callout {
        kind: doctty::core::document::CalloutKind::Tip,
    };

    let lines = display_lines("> [!TIP]\n> one\n>\n> two\n", MermaidMode::Disabled);

    assert_eq!(
        lines[..4],
        [
            ("Tip".to_string(), tip.clone()),
            ("one".to_string(), tip.clone()),
            (String::new(), tip.clone()),
            ("two".to_string(), tip),
        ]
    );
}

#[test]
fn terminal_render_places_a_quoted_diagram_on_unprefixed_rows() {
    let document = parse_document(
        "docs/example.md".into(),
        "> one\n>\n> ```mermaid\n> graph TD\n> ```\n",
    )
    .unwrap_or_else(|error| panic!("document should parse: {error}"));

    let rendered = render_document(&document, Theme::Light, MermaidMode::Enabled, 40, 0.5);

    let [graphic] = rendered.graphics.as_slice() else {
        panic!("one diagram expected: {:?}", rendered.graphics);
    };
    assert_eq!(rendered.lines[2].display_text, "│ Mermaid diagram");
    assert_eq!(graphic.line_index, 3);
    assert_eq!(graphic.width_cells, 38);
    let rows = graphic.line_index..graphic.line_index + usize::from(graphic.height_cells);
    assert!(
        rendered.lines[rows]
            .iter()
            .all(|line| line.kind == RenderedLineKind::Blank && line.display_text.is_empty())
    );
}

#[test]
fn terminal_render_keeps_the_marker_of_an_item_that_starts_with_code() {
    let lines = display_lines("1. ```rust\n   let x = 1;\n   ```\n", MermaidMode::Disabled);

    assert_eq!(lines[0], ("1.".to_string(), RenderedLineKind::List));
    assert_eq!(lines[1].0, "rust");
}

#[test]
fn terminal_render_keeps_a_task_marker_written_as_code() {
    let document = parse_document("docs/example.md".into(), "- `[x]`\n")
        .unwrap_or_else(|error| panic!("document should parse: {error}"));

    let rendered = render_document(&document, Theme::Light, MermaidMode::Disabled, 40, 0.5);

    assert_eq!(rendered.lines[0].display_text, "• [x]");
    assert_eq!(rendered.lines[0].spans[1].style, RenderedInlineStyle::CODE);
}
