use assert_cmd::Command;
use predicates::str::{contains, diff};

#[test]
fn help_mentions_core_flags() {
    let mut command = match Command::cargo_bin("doctty") {
        Ok(command) => command,
        Err(error) => panic!("binary should build: {error}"),
    };

    command
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("Usage: doctty"))
        .stdout(contains("--watch"))
        .stdout(contains("--theme"))
        .stdout(contains("--no-mermaid"))
        .stdout(contains("--version"))
        .stdout(contains("system"));
}


#[test]
fn version_flag_prints_the_package_version() {
    let mut command = match Command::cargo_bin("doctty") {
        Ok(command) => command,
        Err(error) => panic!("binary should build: {error}"),
    };

    command
        .arg("--version")
        .assert()
        .success()
        .stdout(diff(format!("{}\n", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn version_flag_stays_numeric_only_when_combined_with_a_path() {
    let mut command = match Command::cargo_bin("doctty") {
        Ok(command) => command,
        Err(error) => panic!("binary should build: {error}"),
    };

    command
        .args(["README.md", "--version"])
        .assert()
        .success()
        .stdout(diff(format!("{}\n", env!("CARGO_PKG_VERSION"))));
}

