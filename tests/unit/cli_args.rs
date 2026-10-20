use doctty::cli::{DocttyArgs, DocttyCommand, Theme};

#[test]
fn parses_watch_theme_and_mermaid_flags() {
    let args = DocttyArgs::parse_from([
        "doctty",
        "docs/PRD.md",
        "--watch",
        "--theme",
        "dark",
        "--no-mermaid",
    ]);

    assert_eq!(args.theme, Theme::Dark);
    assert!(args.watch);
    assert!(args.no_mermaid);
    assert_eq!(args.path.as_deref(), Some(std::path::Path::new("docs/PRD.md")));
}

#[test]
fn parses_update_subcommand_without_document_path() {
    let args = DocttyArgs::parse_from(["doctty", "update"]);

    assert_eq!(args.command, Some(DocttyCommand::Update));
    assert_eq!(args.path, None);
}

#[test]
fn defaults_theme_to_system() {
    let args = DocttyArgs::parse_from(["doctty", "docs/PRD.md"]);

    assert_eq!(args.theme, Theme::System);
}
