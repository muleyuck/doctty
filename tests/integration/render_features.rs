use assert_cmd::Command;
use predicates::str::contains;
use tempfile::NamedTempFile;

#[test]
fn headless_render_covers_core_mvp_blocks() {
    let file = match NamedTempFile::new() {
        Ok(file) => file,
        Err(error) => panic!("temp file should be created: {error}"),
    };
    let source = "# Title\n\nParagraph with a [link](https://example.com).\n\n> [!WARNING]\n> Be careful\n\n- [x] done\n- [ ] pending\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n![diagram](docs/missing.png)\n\n```rust\nfn main() {}\n```\n\n```mermaid\ngraph TD\n    A --> B\n```\n\nFootnote ref[^1].\n\n[^1]: detail\n";
    if let Err(error) = std::fs::write(file.path(), source) {
        panic!("fixture should be written: {error}");
    }

    let mut command = match Command::cargo_bin("doctty") {
        Ok(command) => command,
        Err(error) => panic!("binary should build: {error}"),
    };

    command
        .arg(file.path())
        .assert()
        .success()
        .stdout(contains("# Title"))
        .stdout(contains("link <https://example.com>"))
        .stdout(contains("[Warning] Be careful"))
        .stdout(contains("[x] done"))
        .stdout(contains("| A | B |"))
        .stdout(contains("[Image missing: diagram"))
        .stdout(contains("```rust"))
        .stdout(contains("[Mermaid"))
        .stdout(contains("[^1] detail"));
}

#[test]
fn headless_render_supports_repo_rich_fixture() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("rich_markdown.md");

    let mut command = match Command::cargo_bin("doctty") {
        Ok(command) => command,
        Err(error) => panic!("binary should build: {error}"),
    };

    command
        .arg(&fixture)
        .assert()
        .success()
        .stdout(contains("# Rich Markdown Fixture"))
        .stdout(contains("[Note]"))
        .stdout(contains("[Image: Fixture image"))
        .stdout(contains("[Mermaid"))
        .stdout(contains(
            "[^details] The fixture keeps every high-value Markdown block in one document.",
        ));
}

#[test]
fn headless_render_supports_stdin_with_dash_path() {
    let mut command = match Command::cargo_bin("doctty") {
        Ok(command) => command,
        Err(error) => panic!("binary should build: {error}"),
    };

    command
        .arg("-")
        .write_stdin("# Title\n\nParagraph with <br/> inline HTML.\n")
        .assert()
        .success()
        .stdout(contains("# Title"))
        .stdout(contains("Paragraph with\ninline HTML."));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn headless_render_rejects_tty_stdin() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        fs::File,
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };

    use assert_cmd::assert::OutputAssertExt;
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

    // A pipe does not exercise the interactive-stdin rejection path.
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let slave_path = ptsname(&master, Vec::new())?;
    let slave = File::open(slave_path.to_str()?)?;

    // Use std::process::Command: assert_cmd::Command replaces stdin with a pipe.
    let mut process = std::process::Command::new(env!("CARGO_BIN_EXE_doctty"));
    process
        .arg("-")
        .stdin(Stdio::from(slave))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = process.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            panic!("doctty - hung while reading from terminal stdin");
        }
        thread::sleep(Duration::from_millis(10));
    }

    child
        .wait_with_output()?
        .assert()
        .failure()
        .stderr(contains("'-' requires piped stdin"));

    Ok(())
}

// A file path with a terminal on stdin and stdout takes the interactive
// branch. Nothing answers the Kitty graphics query on this pty, so reaching
// that branch shows up as the Kitty support error instead of plain text.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn file_path_on_a_terminal_takes_the_interactive_branch() -> Result<(), Box<dyn std::error::Error>>
{
    use std::{
        fs::{File, OpenOptions},
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };

    use assert_cmd::assert::OutputAssertExt;
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

    let file = NamedTempFile::new()?;
    std::fs::write(file.path(), "# Interactive\n")?;

    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let slave_path = ptsname(&master, Vec::new())?;
    let slave_path = slave_path.to_str()?;
    let stdin = File::open(slave_path)?;
    let stdout = OpenOptions::new().write(true).open(slave_path)?;

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_doctty"))
        .arg(file.path())
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            panic!("doctty hung while probing Kitty graphics support");
        }
        thread::sleep(Duration::from_millis(10));
    }

    child
        .wait_with_output()?
        .assert()
        .failure()
        .stderr(contains(
            "interactive mode requires a terminal with Kitty graphics support",
        ));
    drop(master);
    Ok(())
}

#[test]
fn headless_render_keeps_math_expressions_visible() {
    let file = match NamedTempFile::new() {
        Ok(file) => file,
        Err(error) => panic!("temp file should be created: {error}"),
    };
    let source = "Inline $e^{i\\pi} + 1 = 0$ and $$a^2 + b^2 = c^2$$.\n";
    if let Err(error) = std::fs::write(file.path(), source) {
        panic!("fixture should be written: {error}");
    }

    let mut command = match Command::cargo_bin("doctty") {
        Ok(command) => command,
        Err(error) => panic!("binary should build: {error}"),
    };

    command
        .arg(file.path())
        .assert()
        .success()
        .stdout(contains("$e^{i\\pi} + 1 = 0$"))
        .stdout(contains("$$a^2 + b^2 = c^2$$"));
}
