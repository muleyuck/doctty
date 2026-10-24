use std::path::Path;

use super::{
    SnapshotDiagnostics,
    paths::{cleanup_workspace, create_workspace, snapshot_asset_root},
};

#[test]
fn diagnostics_default_to_empty_assets() {
    let diagnostics = SnapshotDiagnostics::default();

    assert!(diagnostics.images.is_empty());
    assert!(diagnostics.mermaids.is_empty());
}

#[test]
fn snapshot_asset_root_expands_to_cover_relative_parent_assets() {
    let html = r#"<html><body><img src="../../../../examples/pixel.png" /></body></html>"#;
    let base_dir = Path::new("/Users/example/project/tests/fixtures/gfm/image");

    let root = snapshot_asset_root(html, base_dir);

    assert_eq!(root, Path::new("/Users/example/project"));
}

#[test]
fn compiled_eco_read_root_includes_parent_directory_assets() {
    let html = r#"<article><img src="../assets/screenshot.png"></article>"#;
    let base_dir = Path::new("/Users/example/project/docs");
    let generated_html = Path::new("/Users/example/project/docs/.doctty-poc-test.html");

    let root = super::compiled_eco_read_root(html, base_dir, generated_html)
        .unwrap_or_else(|error| panic!("safe project asset root expected: {error}"));
    assert_eq!(root, Path::new("/Users/example/project"));
}

#[test]
fn compiled_eco_read_root_refuses_cross_filesystem_root_access() {
    let html = r#"<article><img src="images/screenshot.png"></article>"#;
    let base_dir = Path::new("/Users/example/project/docs");
    let generated_html = Path::new("/private/var/folders/tmp/.doctty-poc-test.html");

    assert!(
        super::compiled_eco_read_root(html, base_dir, generated_html)
            .is_err_and(|error| error.to_string().contains("refusing unrestricted WebKit"))
    );
}

#[test]
fn compiled_eco_read_root_refuses_absolute_asset_outside_document_tree() {
    let html = r#"<article><img src="/private/tmp/external.png"></article>"#;
    let base_dir = Path::new("/Users/example/project/docs");
    let generated_html = base_dir.join(".doctty-poc-test.html");

    assert!(super::compiled_eco_read_root(html, base_dir, &generated_html).is_err());
}

#[test]
fn snapshot_workspace_uses_temp_dir() {
    let workspace = create_workspace(Path::new("/tmp"), Path::new("/"))
        .unwrap_or_else(|error| panic!("workspace should be created: {error}"));

    assert!(workspace.starts_with(Path::new("/tmp")));
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn cleanup_workspace_removes_empty_doctty_webkit_parent() {
    let base = std::env::temp_dir().join(format!(
        "doctty-webkit-cleanup-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base)
        .unwrap_or_else(|error| panic!("base directory should be created: {error}"));
    let workspace = create_workspace(&base, &base)
        .unwrap_or_else(|error| panic!("workspace should be created: {error}"));
    let parent = workspace
        .parent()
        .unwrap_or_else(|| panic!("workspace should have a parent"))
        .to_path_buf();

    cleanup_workspace(&workspace)
        .unwrap_or_else(|error| panic!("workspace cleanup should succeed: {error}"));

    assert!(
        !parent.exists(),
        "empty .doctty-webkit parent should be removed"
    );
    let _ = std::fs::remove_dir_all(base);
}
