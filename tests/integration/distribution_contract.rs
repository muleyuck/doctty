use std::{fs, path::Path};

use serde_yaml::{Mapping, Value};

#[test]
fn ci_checks_code_without_write_permissions() {
    let workflow = fs::read_to_string(".github/workflows/ci.yml")
        .unwrap_or_else(|error| panic!("CI workflow should be readable: {error}"));
    let parsed = parse_yaml(&workflow);
    let root = parsed.as_mapping().unwrap_or_else(|| panic!("workflow root should be a mapping"));
    let jobs = mapping_field(root, "jobs");
    let permissions = mapping_field(root, "permissions");
    let triggers = mapping_field(root, "on");
    let push = mapping_field(triggers, "push");
    let branches = sequence_field(push, "branches");

    assert!(jobs.contains_key(Value::String("checks".to_string())));
    assert!(!jobs.contains_key(Value::String("refresh_tracked_binary".to_string())));
    assert_eq!(
        permissions.get(Value::String("contents".to_string())).and_then(Value::as_str),
        Some("read")
    );
    assert!(branches.iter().any(|branch| branch.as_str() == Some("main")));
}

#[test]
fn legacy_distribution_is_removed() {
    assert!(!Path::new("scripts/install.sh").exists());
    assert!(!Path::new("bin/doctty").exists());
    assert!(!Path::new("src/io/self_update.rs").exists());
}

#[test]
fn release_automation_files_are_removed() {
    assert!(
        !Path::new(".github/workflows/main-channel.yml").exists(),
        "main-channel workflow should be removed"
    );
    assert!(
        !Path::new(".github/workflows/release-assets.yml").exists(),
        "release-assets workflow should be removed"
    );
    assert!(
        !Path::new(".github/workflows/release.yml").exists(),
        "release workflow should be removed"
    );
}

#[test]
fn lefthook_config_uses_repo_quality_gates() {
    let config = fs::read_to_string("lefthook.yml")
        .unwrap_or_else(|error| panic!("lefthook config should be readable: {error}"));
    let parsed = parse_yaml(&config);
    let root = parsed.as_mapping().unwrap_or_else(|| panic!("lefthook root should be a mapping"));
    let pre_commit = mapping_field(root, "pre-commit");
    let pre_push = mapping_field(root, "pre-push");
    let pre_commit_commands = mapping_field(pre_commit, "commands");
    let pre_push_commands = mapping_field(pre_push, "commands");

    assert!(
        config.contains("cargo fmt --all -- --check"),
        "pre-commit should run rustfmt in check mode"
    );
    assert!(
        config.contains("cargo check --workspace --all-targets --all-features"),
        "pre-commit should run cargo check across the full workspace"
    );
    assert!(config.contains("make ci"), "pre-push should reuse the repo CI entrypoint");
    assert!(
        pre_commit_commands.contains_key(Value::String("fmt".to_string())),
        "pre-commit should define the fmt command"
    );
    assert!(
        pre_commit_commands.contains_key(Value::String("check".to_string())),
        "pre-commit should define the check command"
    );
    assert!(
        pre_push_commands.contains_key(Value::String("ci".to_string())),
        "pre-push should define the ci command"
    );
}

#[test]
fn make_build_refreshes_the_local_bin_copy() {
    let makefile = fs::read_to_string("Makefile")
        .unwrap_or_else(|error| panic!("Makefile should be readable: {error}"));

    assert!(
        makefile.contains("build:\n\tcargo build --release\n\tmkdir -p bin"),
        "make build should still compile the release binary and prepare the local bin directory"
    );
    assert!(
        makefile.contains("cp target/release/doctty bin/doctty"),
        "make build should refresh ./bin/doctty so the local runnable path is not stale"
    );
}

fn parse_yaml(source: &str) -> Value {
    serde_yaml::from_str::<Value>(source)
        .unwrap_or_else(|error| panic!("workflow should be valid YAML: {error}"))
}

fn mapping_field<'a>(mapping: &'a Mapping, key: &str) -> &'a Mapping {
    mapping
        .get(Value::String(key.to_string()))
        .and_then(Value::as_mapping)
        .unwrap_or_else(|| panic!("workflow field `{key}` should be a mapping"))
}

fn sequence_field<'a>(mapping: &'a Mapping, key: &str) -> &'a [Value] {
    mapping
        .get(Value::String(key.to_string()))
        .and_then(Value::as_sequence)
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("workflow field `{key}` should be a sequence"))
}
