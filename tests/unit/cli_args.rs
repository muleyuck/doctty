use doctty::cli::{DocttyArgs, Theme};

#[test]
fn parses_watch_theme_and_mermaid_flags() {
    let args = DocttyArgs::parse_from([
        "doctty",
        "README.md",
        "--watch",
        "--theme",
        "dark",
        "--no-mermaid",
    ]);

    assert_eq!(args.theme, Theme::Dark);
    assert!(args.watch);
    assert!(args.no_mermaid);
    assert_eq!(args.path.as_deref(), Some(std::path::Path::new("README.md")));
}

#[test]
fn defaults_theme_to_system() {
    let args = DocttyArgs::parse_from(["doctty", "README.md"]);

    assert_eq!(args.theme, Theme::System);
}
