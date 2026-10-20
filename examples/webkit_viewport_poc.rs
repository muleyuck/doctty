// Feasibility experiment; not part of the production viewer.
// Run inside Ghostty and then in a Herdr pane:
// cargo run --release --example webkit_viewport_poc -- README.md
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This WebKit experiment requires macOS.");
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    experiment::run()
}

#[cfg(target_os = "macos")]
mod experiment {
    use std::{
        fs,
        io::{self, BufRead, BufReader, IsTerminal, Write},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        time::Instant,
    };

    use anyhow::{Context, Result, bail};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use crossterm::{
        cursor,
        event::{self, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use doctty::{
        cli::Theme,
        core::config::MermaidMode,
        render::github_html::build_github_html,
    };
    use image::{ImageDecoder, codecs::png::PngDecoder};
    use serde_json::{Value, json};

    const IMAGE_ID: u32 = 93_001;
    const CHUNK_SIZE: usize = 4096;

    struct TerminalSession;

    impl TerminalSession {
        fn enter() -> Result<Self> {
            terminal::enable_raw_mode()?;
            if let Err(error) = execute!(
                io::stdout(),
                EnterAlternateScreen,
                cursor::Hide,
                Clear(ClearType::All)
            ) {
                let _ = terminal::disable_raw_mode();
                return Err(error.into());
            }
            Ok(Self)
        }
    }

    impl Drop for TerminalSession {
        fn drop(&mut self) {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(
                io::stdout(),
                LeaveAlternateScreen,
                cursor::Show
            );
        }
    }

    struct SwiftProcess {
        child: Child,
        input: std::process::ChildStdin,
        output: BufReader<std::process::ChildStdout>,
    }

    impl SwiftProcess {
        fn start(html: &Path, source_dir: &Path, png: &Path, width: u32, height: u32) -> Result<Self> {
            let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("experiments/webkit_viewport.swift");
            let mut child = Command::new("swift")
                .arg("-swift-version")
                .arg("5")
                .arg(helper)
                .arg(html)
                .arg(source_dir)
                .arg(png)
                .arg(width.to_string())
                .arg(height.to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .context("unable to start Swift WebKit helper")?;
            let input = child.stdin.take().context("missing Swift stdin")?;
            let output = BufReader::new(child.stdout.take().context("missing Swift stdout")?);
            Ok(Self { child, input, output })
        }

        fn request(&mut self, request: &Value) -> Result<Value> {
            writeln!(self.input, "{request}")?;
            self.input.flush()?;
            self.response()
        }

        fn response(&mut self) -> Result<Value> {
            let mut line = String::new();
            if self.output.read_line(&mut line)? == 0 {
                bail!("WebKit helper closed its output; see Swift stderr");
            }
            let payload: Value = serde_json::from_str(&line).context("invalid WebKit JSON response")?;
            if payload["kind"] == "error" {
                bail!("WebKit helper: {}", payload["message"].as_str().unwrap_or("unknown error"));
            }
            Ok(payload)
        }
    }

    impl Drop for SwiftProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn html_input(path: &Path) -> Result<(PathBuf, Option<tempfile::NamedTempFile>)> {
        if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")) {
            return Ok((path.to_path_buf(), None));
        }
        let source = fs::read_to_string(path).context("failed to read Markdown source")?;
        let root = path.parent().context("source directory unavailable")?;
        let html = build_github_html(&source, root, Theme::Light, MermaidMode::Disabled)?;
        let mut file = tempfile::Builder::new()
            .prefix(".doctty-poc-")
            .suffix(".html")
            .tempfile_in(root)
            .context("unable to create temporary HTML alongside Markdown")?;
        file.write_all(html.as_bytes())?;
        file.flush()?;
        Ok((file.path().to_path_buf(), Some(file)))
    }

    fn draw(png_path: &Path, status: &str, columns: u16, rows: u16) -> Result<()> {
        let png = fs::read(png_path).context("missing viewport PNG")?;
        let decoder = PngDecoder::new(std::io::Cursor::new(&png))?;
        let (width, height) = decoder.dimensions();
        let payload = STANDARD.encode(&png);
        let total_chunks = payload.len().div_ceil(CHUNK_SIZE);
        let mut stdout = io::stdout().lock();
        execute!(
            stdout,
            cursor::MoveTo(0, 0),
            Clear(ClearType::All)
        )?;
        // This experiment uses a single atomic a=T upload and placement.
        // Only the displayed viewport pixels are ever transmitted.
        for (index, chunk) in payload.as_bytes().chunks(CHUNK_SIZE).enumerate() {
            let more = u8::from(index + 1 < total_chunks);
            if index == 0 {
                write!(
                    stdout,
                    "\x1b_Ga=T,t=d,f=100,i={IMAGE_ID},p=1,c={columns},r={rows},w={width},h={height},C=1,q=2,z=0,m={more};"
                )?;
            } else {
                write!(stdout, "\x1b_Gm={more};")?;
            }
            stdout.write_all(chunk)?;
            stdout.write_all(b"\x1b\\")?;
        }
        execute!(
            stdout,
            cursor::MoveTo(0, rows),
            Clear(ClearType::CurrentLine)
        )?;
        write!(stdout, "{status}")?;
        stdout.flush()?;
        Ok(())
    }

    fn snapshot(
        helper: &mut SwiftProcess,
        png: &Path,
        position: f64,
        cols: u16,
        rows: u16,
    ) -> Result<(f64, f64)> {
        let started = Instant::now();
        let result = helper.request(&json!({"type": "snapshot", "y": position}))?;
        if result["kind"] != "snapshot" {
            bail!("unexpected WebKit response: {result}");
        }
        let actual = result["actualY"].as_f64().unwrap_or(-1.0);
        let height = result["height"].as_f64().unwrap_or(0.0);
        let ms = result["elapsedMs"].as_f64().unwrap_or(0.0);
        let bytes = result["pngBytes"].as_u64().unwrap_or(0);
        let status = format!(
            "j/k: scroll  J/K: page  y: select+copy first paragraph  q: quit  |  y={actual:.0} / h={height:.0}  snapshot={ms:.0}ms  {bytes}B  total={:.0}ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        draw(png, &status, cols, rows)?;
        Ok((actual, height))
    }

    pub fn run() -> Result<()> {
        if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
            bail!("run this interactive experiment in Ghostty or Herdr");
        }
        let arg = std::env::args_os().nth(1).unwrap_or_else(|| "README.md".into());
        let document = PathBuf::from(arg).canonicalize().context("source file not found")?;
        let dir = document.parent().context("source directory not found")?;
        let (html, _temp_html) = html_input(&document)?;
        let work = tempfile::tempdir()?;
        let output = work.path().join("viewport.png");
        let (cols, total_rows) = terminal::size()?;
        let rows = total_rows.saturating_sub(1).max(1);
        let width = u32::from(cols).saturating_mul(8);
        let height = u32::from(rows).saturating_mul(16);
        eprintln!("Starting persistent WKWebView (initial Swift compilation may take time)...");
        let mut swift = SwiftProcess::start(&html, dir, &output, width, height)?;
        let ready = swift.response()?;
        if ready["kind"] != "ready" {
            bail!("expected WebKit readiness response: {ready}");
        }

        let _session = TerminalSession::enter()?;
        let (mut position, mut document_height) = snapshot(&mut swift, &output, 0.0, cols, rows)?;
        loop {
            let Event::Key(key) = event::read()? else { continue };
            if key.kind != KeyEventKind::Press { continue }
            let step = 16.0;
            let page = f64::from(height) * 0.85;
            let next = match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('j') | KeyCode::Down => position + step,
                KeyCode::Char('k') | KeyCode::Up => position - step,
                KeyCode::Char('J') | KeyCode::PageDown => position + page,
                KeyCode::Char('K') | KeyCode::PageUp => position - page,
                KeyCode::Char('y') => {
                    let selected = swift.request(&json!({"type": "select"}))?;
                    let text = selected["text"].as_str().unwrap_or("");
                    if !text.is_empty() {
                        let mut out = io::stdout().lock();
                        write!(out, "\x1b]52;c;{}\x07", STANDARD.encode(text))?;
                        out.flush()?;
                    }
                    continue;
                }
                _ => continue,
            };
            let max_pos = (document_height - f64::from(height)).max(0.0);
            (position, document_height) =
                snapshot(&mut swift, &output, next.clamp(0.0, max_pos), cols, rows)?;
        }
        let _ = swift.request(&json!({"type": "quit"}));
        Ok(())
    }
}
