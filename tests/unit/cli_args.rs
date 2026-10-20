use doctty::cli::{DocttyArgs, Theme};

// This test only parses CLI arguments; the path does not need to exist.
#[test]
fn parses_watch_theme_and_mermaid_flags() {
    let args = DocttyArgs::parse_from([
        "doctty",
        "sample.md",
        "--watch",
        "--theme",
        "dark",
        "--no-mermaid",
    ]);

    assert_eq!(args.theme, Theme::Dark);
    assert!(args.watch);
    assert!(args.no_mermaid);
    assert_eq!(args.path.as_deref(), Some(std::path::Path::new("sample.md")));
}

#[test]
fn defaults_theme_to_system() {
    let args = DocttyArgs::parse_from(["doctty", "sample.md"]);

    assert_eq!(args.theme, Theme::System);
}
