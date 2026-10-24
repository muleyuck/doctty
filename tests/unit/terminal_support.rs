use doctty::ui::terminal::is_supported_terminal;

#[test]
fn accepts_ghostty_and_kitty() {
    assert!(is_supported_terminal(Some("ghostty"), None));
    assert!(is_supported_terminal(None, Some("xterm-kitty")));
}

#[test]
fn rejects_unknown_terminal() {
    assert!(!is_supported_terminal(
        Some("iterm"),
        Some("xterm-256color")
    ));
}

#[test]
fn accepts_herdr_panes_in_compatible_outer_terminals() {
    assert!(is_supported_terminal(Some("herdr"), Some("xterm-256color")));
    assert!(is_supported_terminal(Some("HERDR"), Some("xterm-256color")));
    assert!(!is_supported_terminal(None, Some("xterm-256color")));
}
