use doctty::io::self_update::{main_binary_url, main_manifest_url};

#[test]
fn points_update_to_the_tracked_main_binary() {
    assert_eq!(
        main_binary_url(),
        "https://raw.githubusercontent.com/muleyuck/doctty/main/bin/doctty"
    );
    assert_eq!(
        main_manifest_url(),
        "https://raw.githubusercontent.com/muleyuck/doctty/main/Cargo.toml"
    );
}
