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

    let checks = mapping_field(jobs, "checks");
    let inputs = mapping_field(checks, "with");
    assert_eq!(
        checks.get(Value::String("uses".to_string())).and_then(Value::as_str),
        Some("muleyuck/github-actions/.github/workflows/ci-rust.yml@v1")
    );
    assert_eq!(
        inputs.get(Value::String("test-command".to_string())).and_then(Value::as_str),
        Some("make ci")
    );
    let macos_build = mapping_field(jobs, "macos-build");
    let macos_inputs = mapping_field(macos_build, "with");
    assert_eq!(
        macos_build.get(Value::String("uses".to_string())).and_then(Value::as_str),
        Some("muleyuck/github-actions/.github/workflows/ci-rust.yml@v1")
    );
    assert_eq!(
        macos_inputs.get(Value::String("test-command".to_string())).and_then(Value::as_str),
        Some("cargo build --release --locked")
    );
    assert_eq!(
        macos_inputs.get(Value::String("runs-on".to_string())).and_then(Value::as_str),
        Some("macos-latest")
    );
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
fn prek_config_uses_repo_quality_gates() {
    let config = fs::read_to_string(".pre-commit-config.yml")
        .unwrap_or_else(|error| panic!("prek config should be readable: {error}"));
    let parsed = parse_yaml(&config);
    let root = parsed.as_mapping().unwrap_or_else(|| panic!("prek root should be a mapping"));
    let repos = root
        .get(Value::String("repos".to_string()))
        .and_then(Value::as_sequence)
        .unwrap_or_else(|| panic!("prek should have a repos list"));
    let hook_ids = repos
        .iter()
        .filter_map(|repo| repo.get("hooks").and_then(Value::as_sequence))
        .flatten()
        .filter_map(|hook| hook.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();

    for id in [
        "check-yaml",
        "check-toml",
        "check-merge-conflict",
        "cargo-fmt",
        "cargo-check",
        "cargo-clippy",
        "cargo-test",
    ] {
        assert!(hook_ids.contains(&id), "prek should define {id}");
    }

    for args in [
        "args: [fmt, --all, --, --check]",
        "args: [check, --workspace, --all-targets, --all-features]",
        "args: [clippy, --workspace, --all-targets, --all-features, --, -D, warnings]",
        "args: [test, --workspace, --all-targets, --all-features]",
    ] {
        assert!(config.contains(args), "prek should run {args}");
    }

    assert!(!Path::new("lefthook.yml").exists(), "lefthook config should be removed");

    let mise = fs::read_to_string("mise.toml")
        .unwrap_or_else(|error| panic!("mise config should be readable: {error}"));
    assert!(mise.contains("prek = \"0.5.5\""), "mise should pin prek");

    let makefile = fs::read_to_string("Makefile")
        .unwrap_or_else(|error| panic!("Makefile should be readable: {error}"));
    assert!(
        makefile.contains("hooks-install:\n\tprek install --overwrite"),
        "make hooks-install should install prek"
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
