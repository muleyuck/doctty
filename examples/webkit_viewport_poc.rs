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
    use doctty::{cli::Theme, core::config::MermaidMode, render::github_html::build_github_html};
    use image::{ImageDecoder, codecs::png::PngDecoder};
    use serde_json::{Value, json};

    const FIRST_IMAGE_ID: u32 = 93_001;
    const SECOND_IMAGE_ID: u32 = 93_002;
    const CHUNK_SIZE: usize = 4096;

    struct ImageCache {
        image_id: u32,
        source_y: f64,
        image_width: u32,
        image_height: u32,
        document_height: f64,
        last_snapshot_ms: f64,
        png_bytes: usize,
    }

    impl ImageCache {
        fn covers(&self, position: f64, viewport_height: u32) -> bool {
            position >= self.source_y - 0.5
                && position + f64::from(viewport_height)
                    <= self.source_y + f64::from(self.image_height) + 0.5
        }
    }

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
            let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
        }
    }

    struct SwiftProcess {
        child: Child,
        input: std::process::ChildStdin,
        output: BufReader<std::process::ChildStdout>,
    }

    impl SwiftProcess {
        fn start(
            html: &Path,
            source_dir: &Path,
            png: &Path,
            width: u32,
            height: u32,
        ) -> Result<Self> {
            let helper =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("experiments/webkit_viewport.swift");
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
            Ok(Self {
                child,
                input,
                output,
            })
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
            let payload: Value =
                serde_json::from_str(&line).context("invalid WebKit JSON response")?;
            if payload["kind"] == "error" {
                bail!(
                    "WebKit helper: {}",
                    payload["message"].as_str().unwrap_or("unknown error")
                );
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
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
        {
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

    fn upload_png(out: &mut impl Write, image_id: u32, png: &[u8]) -> Result<()> {
        let payload = STANDARD.encode(png);
        let total_chunks = payload.len().div_ceil(CHUNK_SIZE);
        for (index, chunk) in payload.as_bytes().chunks(CHUNK_SIZE).enumerate() {
            let more = u8::from(index + 1 < total_chunks);
            if index == 0 {
                write!(
                    out,
                    "\x1b_Ga=t,t=d,f=100,i={image_id},q=2,m={more};"
                )?;
            } else {
                write!(out, "\x1b_Gm={more};")?;
            }
            out.write_all(chunk)?;
            out.write_all(b"\x1b\\")?;
        }
        Ok(())
    }

    fn place_image(
        out: &mut impl Write,
        image_id: u32,
        columns: u16,
        rows: u16,
        width: u32,
        height: u32,
        source_y: u32,
    ) -> Result<()> {
        write!(
            out,
            "\x1b[1;1H\x1b_Ga=p,i={image_id},p=1,c={columns},r={rows},x=0,y={source_y},w={width},h={height},C=1,z=0,q=2;\x1b\\"
        )?;
        Ok(())
    }

    fn render_scroll(
        helper: &mut SwiftProcess,
        png_path: &Path,
        position: f64,
        columns: u16,
        rows: u16,
        viewport_width: u32,
        viewport_height: u32,
        cache: &mut Option<ImageCache>,
    ) -> Result<f64> {
        let started = Instant::now();
        let cache_hit = cache
            .as_ref()
            .is_some_and(|entry| entry.covers(position, viewport_height));

        if !cache_hit {
            let document_height = cache
                .as_ref()
                .map_or(0.0, |entry| entry.document_height);
            let requested_y = if cache.is_some() {
                (position - f64::from(viewport_height) / 2.0)
                    .max(0.0)
                    .min((document_height - 2.0 * f64::from(viewport_height)).max(0.0))
            } else {
                0.0
            };
            let result = helper.request(&json!({"type": "snapshot", "y": requested_y}))?;
            if result["kind"] != "snapshot" {
                bail!("unexpected WebKit response: {result}");
            }
            let png = fs::read(png_path).context("missing viewport PNG")?;
            let decoder = PngDecoder::new(io::Cursor::new(&png))?;
            let (image_width, image_height) = decoder.dimensions();
            if image_width < viewport_width || image_height < viewport_height {
                bail!("cached image smaller than viewport: {image_width}x{image_height}");
            }
            let new_id = match cache.as_ref().map(|entry| entry.image_id) {
                Some(FIRST_IMAGE_ID) => SECOND_IMAGE_ID,
                _ => FIRST_IMAGE_ID,
            };
            let new_cache = ImageCache {
                image_id: new_id,
                source_y: result["actualY"].as_f64().unwrap_or(0.0),
                image_width,
                image_height,
                document_height: result["height"].as_f64().unwrap_or(0.0),
                last_snapshot_ms: result["elapsedMs"].as_f64().unwrap_or(0.0),
                png_bytes: png.len(),
            };
            let source_offset = (position - new_cache.source_y)
                .max(0.0)
                .min(f64::from(image_height.saturating_sub(viewport_height)))
                .round() as u32;
            let mut out = io::stdout().lock();
            // Keep the previous image visible while uploading the next image.
            upload_png(&mut out, new_id, &png)?;
            place_image(
                &mut out,
                new_id,
                columns,
                rows,
                viewport_width,
                viewport_height,
                source_offset,
            )?;
            if let Some(previous) = cache.as_ref() {
                write!(
                    out,
                    "\x1b_Ga=d,d=i,i={},p=1,q=2;\x1b\\",
                    previous.image_id
                )?;
            }
            *cache = Some(new_cache);
        } else if let Some(entry) = cache.as_ref() {
            let source_offset = (position - entry.source_y)
                .max(0.0)
                .min(f64::from(entry.image_height.saturating_sub(viewport_height)))
                .round() as u32;
            let mut out = io::stdout().lock();
            // Stable image ID and placement ID: crop/scroll without re-uploading.
            place_image(
                &mut out,
                entry.image_id,
                columns,
                rows,
                entry.image_width.min(viewport_width),
                viewport_height,
                source_offset,
            )?;
        }

        let current = cache.as_ref().context("viewport cache missing")?;
        let status = format!(
            "j/k scroll J/K page y copy q quit | {} y={position:.0}/{:.0} snap={:.0}ms PNG={}B redraw={:.0}ms",
            if cache_hit { "cache HIT" } else { "cache MISS" },
            current.document_height,
            current.last_snapshot_ms,
            current.png_bytes,
            started.elapsed().as_secs_f64() * 1000.0
        );
        let mut out = io::stdout().lock();
        execute!(
            out,
            cursor::MoveTo(0, rows),
            Clear(ClearType::CurrentLine)
        )?;
        write!(out, "{}", status.chars().take(usize::from(columns)).collect::<String>())?;
        out.flush()?;
        Ok(current.document_height)
    }

    fn graphics_probe() -> Result<()> {
        use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

        let (width, height) = terminal::size()?;
        let columns = width.min(18).max(1);
        let rows = height.saturating_sub(2).min(9).max(1);
        let image = ImageBuffer::from_pixel(12, 12, Rgba([255, 0, 140, 255]));
        let mut png = Vec::new();
        DynamicImage::ImageRgba8(image).write_to(
            &mut io::Cursor::new(&mut png),
            ImageFormat::Png,
        )?;

        let _session = TerminalSession::enter()?;
        let mut out = io::stdout().lock();
        // Deliberately bypasses WebKit and the experimental viewport cache.
        upload_png(&mut out, FIRST_IMAGE_ID, &png)?;
        place_image(&mut out, FIRST_IMAGE_ID, columns, rows, 12, 12, 0)?;
        execute!(out, cursor::MoveTo(0, rows))?;
        write!(out, "Magenta rectangle? Press q to exit.")?;
        out.flush()?;
        drop(out);
        loop {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                {
                    break;
                }
            }
        }
        Ok(())
    }

    pub fn run() -> Result<()> {
        if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
            bail!("run this interactive experiment in Ghostty or Herdr");
        }
        let arg = std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "README.md".into());
        if arg == "--graphics-probe" {
            return graphics_probe();
        }
        let document = PathBuf::from(arg)
            .canonicalize()
            .context("source file not found")?;
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
        let mut cache = None;
        let mut position = 0.0;
        let mut document_height = render_scroll(
            &mut swift,
            &output,
            position,
            cols,
            rows,
            width,
            height,
            &mut cache,
        )?;
        loop {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
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
            let clamped = next.clamp(0.0, max_pos);
            if (clamped - position).abs() > 0.5 {
                position = clamped;
                document_height = render_scroll(
                    &mut swift,
                    &output,
                    position,
                    cols,
                    rows,
                    width,
                    height,
                    &mut cache,
                )?;
            }
        }
        let _ = swift.request(&json!({"type": "quit"}));
        Ok(())
    }
}
