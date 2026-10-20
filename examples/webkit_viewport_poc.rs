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
        sync::mpsc::{self, Receiver, Sender, TryRecvError},
        thread,
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result, bail};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use crossterm::{
        cursor,
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
        execute,
        terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use doctty::{cli::Theme, core::config::MermaidMode, render::github_html::build_github_html};
    use image::{ImageDecoder, codecs::png::PngDecoder};
    use serde_json::{Value, json};

    const FIRST_IMAGE_ID: u32 = 93_001;
    const SECOND_IMAGE_ID: u32 = 93_002;
    const CHUNK_SIZE: usize = 4096;
    const MAX_INPUT_BATCH: usize = 32;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ScrollDirection {
        Forward,
        Backward,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum InputAction {
        Continue,
        Select,
        Quit,
    }

    // Apply individual keys in order so clamping and direction changes retain
    // their original semantics. The caller batches redraws, not navigation.
    fn apply_key(
        key: KeyEvent,
        position: &mut f64,
        direction: &mut ScrollDirection,
        document_height: f64,
        viewport_height: u32,
    ) -> InputAction {
        if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
            return InputAction::Continue;
        }
        let step = 16.0;
        let page = f64::from(viewport_height) * 0.85;
        let next = match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return InputAction::Quit,
            KeyCode::Char('j') | KeyCode::Down => *position + step,
            KeyCode::Char('k') | KeyCode::Up => *position - step,
            KeyCode::Char('J') | KeyCode::PageDown => *position + page,
            KeyCode::Char('K') | KeyCode::PageUp => *position - page,
            KeyCode::Char('y') => return InputAction::Select,
            _ => return InputAction::Continue,
        };
        let max_pos = (document_height - f64::from(viewport_height)).max(0.0);
        let next_position = next.clamp(0.0, max_pos);
        if next_position > *position {
            *direction = ScrollDirection::Forward;
        } else if next_position < *position {
            *direction = ScrollDirection::Backward;
        }
        *position = next_position;
        InputAction::Continue
    }

    #[derive(Clone, Debug)]
    struct ImageCache {
        image_id: u32,
        source_y: f64,
        image_width: u32,
        image_height: u32,
        pixels_per_point: f64,
        document_height: f64,
        last_snapshot_ms: f64,
        last_upload_ms: f64,
        png_bytes: usize,
    }

    impl ImageCache {
        fn covers(&self, position: f64, viewport_height: u32) -> bool {
            position >= self.source_y - 0.5
                && position + f64::from(viewport_height)
                    <= self.source_y + f64::from(self.image_height) / self.pixels_per_point + 0.5
        }

        fn viewport_height_px(&self, viewport_height: u32) -> u32 {
            (f64::from(viewport_height) * self.pixels_per_point)
                .round()
                .max(1.0) as u32
        }

        fn source_offset_px(&self, position: f64, viewport_height: u32) -> u32 {
            ((position - self.source_y) * self.pixels_per_point)
                .round()
                .max(0.0)
                .min(f64::from(
                    self.image_height
                        .saturating_sub(self.viewport_height_px(viewport_height)),
                )) as u32
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
            let mut stdout = io::stdout().lock();
            // Free both of this PoC's fixed image IDs, even on early exit.
            let _ = delete_image(&mut stdout, FIRST_IMAGE_ID);
            let _ = delete_image(&mut stdout, SECOND_IMAGE_ID);
            let _ = stdout.flush();
            let _ = terminal::disable_raw_mode();
            let _ = execute!(stdout, LeaveAlternateScreen, cursor::Show);
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

    enum WorkerCommand {
        Snapshot { origin: f64, image_id: u32 },
        Select,
    }

    enum WorkerEvent {
        Snapshot(ImageCache),
        Selection(String),
        Error(String),
    }

    // WebKit operations and PNG uploading run off the event loop. The worker
    // transmits images without placing them; only the UI thread changes placements.
    fn spawn_worker(
        mut swift: SwiftProcess,
        png_path: PathBuf,
        viewport_width: u32,
    ) -> (Sender<WorkerCommand>, Receiver<WorkerEvent>) {
        let (requests, incoming) = mpsc::channel::<WorkerCommand>();
        let (outgoing, responses) = mpsc::channel::<WorkerEvent>();
        thread::spawn(move || {
            while let Ok(command) = incoming.recv() {
                let reply = match command {
                    WorkerCommand::Snapshot { origin, image_id } => {
                        let result = (|| -> Result<ImageCache> {
                            let response =
                                swift.request(&json!({"type": "snapshot", "y": origin}))?;
                            if response["kind"] != "snapshot" {
                                bail!("unexpected WebKit snapshot reply: {response}");
                            }
                            let png = fs::read(&png_path).context("missing prefetched PNG")?;
                            let decoder = PngDecoder::new(io::Cursor::new(&png))?;
                            let (image_width, image_height) = decoder.dimensions();
                            let scale = f64::from(image_width) / f64::from(viewport_width);
                            if scale <= 0.0 {
                                bail!("invalid prefetched PNG width");
                            }
                            // Never place the image before the complete transmission.
                            let upload_started = Instant::now();
                            let mut stdout = io::stdout().lock();
                            upload_png(&mut stdout, image_id, &png)?;
                            stdout.flush()?;
                            let upload_ms = upload_started.elapsed().as_secs_f64() * 1000.0;
                            Ok(ImageCache {
                                image_id,
                                source_y: response["actualY"].as_f64().unwrap_or(origin),
                                image_width,
                                image_height,
                                pixels_per_point: scale,
                                document_height: response["height"].as_f64().unwrap_or(0.0),
                                last_snapshot_ms: response["elapsedMs"].as_f64().unwrap_or(0.0),
                                last_upload_ms: upload_ms,
                                png_bytes: png.len(),
                            })
                        })();
                        match result {
                            Ok(cache) => WorkerEvent::Snapshot(cache),
                            Err(error) => WorkerEvent::Error(error.to_string()),
                        }
                    }
                    WorkerCommand::Select => match swift.request(&json!({"type": "select"})) {
                        Ok(value) => {
                            WorkerEvent::Selection(value["text"].as_str().unwrap_or("").to_owned())
                        }
                        Err(error) => WorkerEvent::Error(error.to_string()),
                    },
                };
                if outgoing.send(reply).is_err() {
                    break;
                }
            }
            // Closing the channel eventually drops SwiftProcess and kills the helper.
        });
        (requests, responses)
    }

    fn prefetch_origin(
        active: &ImageCache,
        desired: f64,
        height: u32,
        direction: ScrollDirection,
    ) -> Option<f64> {
        let viewport = f64::from(height);
        let cache_bottom =
            active.source_y + f64::from(active.image_height) / active.pixels_per_point;
        let max_origin = (active.document_height - viewport * 2.0).max(0.0);
        let candidate = if !active.covers(desired, height) {
            // Recovery when the user outpaces a pending snapshot.
            (desired - viewport * 0.5).clamp(0.0, max_origin)
        } else {
            match direction {
                ScrollDirection::Forward
                    if desired + viewport * 1.8 >= cache_bottom
                        && active.source_y < max_origin =>
                {
                    (active.source_y + viewport).min(max_origin)
                }
                ScrollDirection::Backward
                    if desired <= active.source_y + viewport * 0.65
                        && active.source_y > 0.0 =>
                {
                    (active.source_y - viewport).max(0.0)
                }
                _ => return None,
            }
        };
        ((candidate - active.source_y).abs() > 1.0).then_some(candidate)
    }

    fn prefetch_is_stale(
        active: &ImageCache,
        prefetched: &ImageCache,
        direction: ScrollDirection,
    ) -> bool {
        match direction {
            ScrollDirection::Forward => prefetched.source_y < active.source_y - 1.0,
            ScrollDirection::Backward => prefetched.source_y > active.source_y + 1.0,
        }
    }

    fn display_cached(
        cache: &ImageCache,
        position: f64,
        columns: u16,
        rows: u16,
        viewport_height: u32,
        state: &str,
    ) -> Result<()> {
        let started = Instant::now();
        let mut out = io::stdout().lock();
        // Re-place pixels already stored in the terminal: zero PNG encoding/transfer.
        place_image(
            &mut out,
            cache.image_id,
            columns,
            rows,
            cache.image_width,
            cache.viewport_height_px(viewport_height),
            cache.source_offset_px(position, viewport_height),
        )?;
        execute!(out, cursor::MoveTo(0, rows), Clear(ClearType::CurrentLine))?;
        let status = format!(
            "{state} snap={:.0}ms upload={:.0}ms place={:.0}ms y={position:.0}/{:.0} | j/k J/K q",
            cache.last_snapshot_ms,
            cache.last_upload_ms,
            started.elapsed().as_secs_f64() * 1000.0,
            cache.document_height
        );
        write!(
            out,
            "{}",
            status
                .chars()
                .take(usize::from(columns))
                .collect::<String>()
        )?;
        out.flush()?;
        Ok(())
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
                write!(out, "\x1b_Ga=t,t=d,f=100,i={image_id},q=2,m={more};")?;
            } else {
                write!(out, "\x1b_Gm={more};")?;
            }
            out.write_all(chunk)?;
            out.write_all(b"\x1b\\")?;
        }
        Ok(())
    }

    fn delete_image(out: &mut impl Write, image_id: u32) -> Result<()> {
        // Uppercase I frees the stored image data as well as its placements.
        write!(out, "\x1b_Ga=d,d=I,i={image_id},q=2;\x1b\\")?;
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
            let document_height = cache.as_ref().map_or(0.0, |entry| entry.document_height);
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
            let scale = f64::from(image_width) / f64::from(viewport_width);
            if scale <= 0.0 || f64::from(image_height) < f64::from(viewport_height) * scale {
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
                pixels_per_point: scale,
                document_height: result["height"].as_f64().unwrap_or(0.0),
                last_snapshot_ms: result["elapsedMs"].as_f64().unwrap_or(0.0),
                last_upload_ms: 0.0,
                png_bytes: png.len(),
            };
            let source_offset = new_cache.source_offset_px(position, viewport_height);
            let mut out = io::stdout().lock();
            // Keep the previous image visible while uploading the next image.
            upload_png(&mut out, new_id, &png)?;
            place_image(
                &mut out,
                new_id,
                columns,
                rows,
                new_cache.image_width,
                new_cache.viewport_height_px(viewport_height),
                source_offset,
            )?;
            if let Some(previous) = cache.as_ref() {
                delete_image(&mut out, previous.image_id)?;
            }
            *cache = Some(new_cache);
        } else if let Some(entry) = cache.as_ref() {
            let source_offset = entry.source_offset_px(position, viewport_height);
            let mut out = io::stdout().lock();
            // Stable image ID and placement ID: crop/scroll without re-uploading.
            place_image(
                &mut out,
                entry.image_id,
                columns,
                rows,
                entry.image_width,
                entry.viewport_height_px(viewport_height),
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
        execute!(out, cursor::MoveTo(0, rows), Clear(ClearType::CurrentLine))?;
        write!(
            out,
            "{}",
            status
                .chars()
                .take(usize::from(columns))
                .collect::<String>()
        )?;
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
        DynamicImage::ImageRgba8(image)
            .write_to(&mut io::Cursor::new(&mut png), ImageFormat::Png)?;

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
        let initial_height = render_scroll(
            &mut swift, &output, position, cols, rows, width, height, &mut cache,
        )?;
        let mut active = cache.context("initial snapshot did not create a cache")?;
        let mut document_height = initial_height;
        let (requests, responses) = spawn_worker(swift, output, width);
        let mut pending = false;
        let mut prefetched: Option<ImageCache> = None;
        let mut last_displayed: Option<(u32, u64)> = None;
        let mut direction = ScrollDirection::Forward;

        'viewer: loop {
            // A completed prefetch is already uploaded to the terminal in the
            // background; activating it requires only a placement and an old delete.
            loop {
                match responses.try_recv() {
                    Ok(WorkerEvent::Snapshot(next)) => {
                        pending = false;
                        prefetched = Some(next);
                    }
                    Ok(WorkerEvent::Selection(text)) => {
                        if !text.is_empty() {
                            let mut out = io::stdout().lock();
                            write!(out, "\x1b]52;c;{}\x07", STANDARD.encode(text))?;
                            out.flush()?;
                        }
                    }
                    Ok(WorkerEvent::Error(error)) => bail!("WebKit prefetch: {error}"),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => bail!("WebKit worker disconnected"),
                }
            }

            // A direction change can make an already-uploaded prefetch useless.
            // Discard it so it cannot starve the required image in the new direction.
            if prefetched
                .as_ref()
                .is_some_and(|next| prefetch_is_stale(&active, next, direction))
            {
                if let Some(stale) = prefetched.take() {
                    let mut out = io::stdout().lock();
                    delete_image(&mut out, stale.image_id)?;
                    out.flush()?;
                }
            }

            // Only two terminal images are kept: active and one prefetched.
            if let Some(next) = prefetched.take() {
                let active_covers = active.covers(position, height);
                let next_covers = next.covers(position, height);
                let should_switch = next_covers
                    && (!active_covers
                        || (next.source_y > active.source_y
                            && position >= active.source_y + f64::from(height) * 0.95)
                        || (next.source_y < active.source_y
                            && position <= active.source_y + f64::from(height) * 0.15));
                if should_switch {
                    let previous = active.image_id;
                    active = next;
                    document_height = active.document_height;
                    let mut out = io::stdout().lock();
                    // The incoming image is uploaded, but is not yet visible.
                    place_image(
                        &mut out,
                        active.image_id,
                        cols,
                        rows,
                        active.image_width,
                        active.viewport_height_px(height),
                        active.source_offset_px(position, height),
                    )?;
                    delete_image(&mut out, previous)?;
                    out.flush()?;
                } else if !active_covers && !next_covers {
                    // Direction changed while prefetching: discard the stale image.
                    let mut out = io::stdout().lock();
                    delete_image(&mut out, next.image_id)?;
                    out.flush()?;
                } else {
                    prefetched = Some(next);
                }
            }

            if !pending && prefetched.is_none() {
                if let Some(origin) = prefetch_origin(&active, position, height, direction) {
                    let image_id = if active.image_id == FIRST_IMAGE_ID {
                        SECOND_IMAGE_ID
                    } else {
                        FIRST_IMAGE_ID
                    };
                    requests.send(WorkerCommand::Snapshot { origin, image_id })?;
                    pending = true;
                }
            }

            let state = if !active.covers(position, height) {
                // Do not block the input loop. Preserve the old image while waiting.
                "waiting for prefetch"
            } else if prefetched.is_some() {
                "prefetch READY"
            } else if pending {
                "prefetch RUNNING"
            } else {
                "cache HIT"
            };
            if active.covers(position, height) {
                // Do not send redundant placements every 12 ms when nothing moved.
                let fingerprint = (active.image_id, position.to_bits());
                if last_displayed != Some(fingerprint) {
                    display_cached(&active, position, cols, rows, height, state)?;
                    last_displayed = Some(fingerprint);
                }
            }

            if !event::poll(Duration::from_millis(12))? {
                continue;
            }
            // Ghostty may enqueue several arrow-key events for one fast wheel
            // gesture. Consume a bounded burst before the next image placement.
            for index in 0..MAX_INPUT_BATCH {
                if index > 0 && !event::poll(Duration::ZERO)? {
                    break;
                }
                let Event::Key(key) = event::read()? else {
                    continue;
                };
                match apply_key(key, &mut position, &mut direction, document_height, height) {
                    InputAction::Quit => break 'viewer,
                    InputAction::Select => requests.send(WorkerCommand::Select)?,
                    InputAction::Continue => {}
                }
            }
        }
        // Dropping requests stops the worker and its Swift child after the
        // current operation. TerminalSession cleans up both image IDs.
        drop(requests);
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crossterm::event::KeyModifiers;

        fn key(code: KeyCode) -> KeyEvent {
            KeyEvent::new(code, KeyModifiers::NONE)
        }

        #[test]
        fn batches_arrow_scroll_without_losing_distance() {
            let mut position = 0.0;
            let mut direction = ScrollDirection::Forward;
            for code in [KeyCode::Down, KeyCode::Down, KeyCode::Down] {
                assert_eq!(
                    apply_key(key(code), &mut position, &mut direction, 10_000.0, 1000),
                    InputAction::Continue
                );
            }
            assert_eq!(position, 48.0);
            assert_eq!(direction, ScrollDirection::Forward);
        }

        #[test]
        fn preserves_clamping_and_direction_during_reversal() {
            let mut position = 0.0;
            let mut direction = ScrollDirection::Forward;
            for code in [KeyCode::Up, KeyCode::Down, KeyCode::Up] {
                let _ = apply_key(key(code), &mut position, &mut direction, 10_000.0, 1000);
            }
            assert_eq!(position, 0.0);
            assert_eq!(direction, ScrollDirection::Backward);

            let _ = apply_key(key(KeyCode::PageDown), &mut position, &mut direction, 1100.0, 1000);
            assert_eq!(position, 100.0);
            assert_eq!(direction, ScrollDirection::Forward);
        }

        #[test]
        fn preserves_control_keys_and_ignores_key_releases() {
            let mut position = 0.0;
            let mut direction = ScrollDirection::Forward;
            assert_eq!(
                apply_key(key(KeyCode::Char('y')), &mut position, &mut direction, 2000.0, 1000),
                InputAction::Select
            );
            assert_eq!(
                apply_key(key(KeyCode::Char('q')), &mut position, &mut direction, 2000.0, 1000),
                InputAction::Quit
            );
            let mut released = key(KeyCode::Down);
            released.kind = KeyEventKind::Release;
            let _ = apply_key(released, &mut position, &mut direction, 2000.0, 1000);
            assert_eq!(position, 0.0);
        }

        fn cached_at(source_y: f64) -> ImageCache {
            ImageCache {
                image_id: FIRST_IMAGE_ID,
                source_y,
                image_width: 800,
                image_height: 2000,
                pixels_per_point: 1.0,
                document_height: 10_000.0,
                last_snapshot_ms: 0.0,
                last_upload_ms: 0.0,
                png_bytes: 100,
            }
        }

        #[test]
        fn prefetches_early_in_the_scroll_direction() {
            let first = cached_at(0.0);
            assert_eq!(
                prefetch_origin(&first, 300.0, 1000, ScrollDirection::Forward),
                Some(1000.0)
            );
            assert_eq!(
                prefetch_origin(&first, 300.0, 1000, ScrollDirection::Backward),
                None
            );
            assert_eq!(
                prefetch_origin(&cached_at(1000.0), 1000.0, 1000, ScrollDirection::Forward),
                None
            );
            assert_eq!(
                prefetch_origin(&cached_at(1000.0), 1000.0, 1000, ScrollDirection::Backward),
                Some(0.0)
            );
        }

        #[test]
        fn rejects_prefetch_from_opposite_direction() {
            let active = cached_at(1000.0);
            assert!(prefetch_is_stale(
                &active,
                &cached_at(2000.0),
                ScrollDirection::Backward
            ));
            assert!(prefetch_is_stale(
                &active,
                &cached_at(0.0),
                ScrollDirection::Forward
            ));
            assert!(!prefetch_is_stale(
                &active,
                &cached_at(2000.0),
                ScrollDirection::Forward
            ));
        }

        #[test]
        fn does_not_prefetch_past_document_end() {
            let mut last = cached_at(8000.0);
            last.document_height = 10_000.0;
            assert_eq!(
                prefetch_origin(&last, 9000.0, 1000, ScrollDirection::Forward),
                None
            );
        }
    }
}
