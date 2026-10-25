// Compiled Rust renderer viewport companion for interactive macOS terminals.
// Distributed alongside doctty and the native Swift/WebKit helper.
// Developer invocation: cargo run --release --bin doctty-renderer -- README.md
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("The doctty renderer viewport requires macOS.");
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    renderer_viewer::run()
}

#[cfg(target_os = "macos")]
#[path = "../viewport/hybrid_image_preview.rs"]
mod hybrid_image_preview;

#[cfg(target_os = "macos")]
mod renderer_viewer {
    use std::{
        ffi::OsStr,
        fs,
        io::{self, BufRead, BufReader, IsTerminal, Write},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
            mpsc::{self, Receiver, Sender, TryRecvError},
        },
        thread,
        time::{Duration, Instant, SystemTime},
    };

    use super::hybrid_image_preview;
    use anyhow::{Context, Result, bail};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use crossterm::{
        cursor,
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
        execute,
        terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use doctty::{
        cli::Theme,
        core::config::MermaidMode,
        io::browser,
        render::{github_html::build_github_html, markdown::parse_document},
    };
    use image::{ImageDecoder, codecs::png::PngDecoder};
    use serde_json::{Value, json};

    const FIRST_IMAGE_ID: u32 = 93_001;
    const SECOND_IMAGE_ID: u32 = 93_002;
    const CHUNK_SIZE: usize = 4096;
    // Keep each visible scroll step small (4 arrow keys x 16px = 64px).
    // Larger batches reduce Kitty placements but look jumpy on fast trackpad input.
    const MAX_INPUT_BATCH: usize = 4;
    // Experimental bounded tile store: four uploaded 2-viewport snapshots
    // and at most one additional in-flight image.
    const MAX_TILE_CACHE: usize = 4;
    const LAST_TILE_IMAGE_ID: u32 = 93_006;
    // Alternating IDs let a resized frame arrive before the old placement is deleted.
    const FIRST_RESIZE_IMAGE_ID: u32 = 93_007;
    const SECOND_RESIZE_IMAGE_ID: u32 = 93_008;
    const HYBRID_PREVIEW_AFTER: Duration = Duration::from_millis(80);
    const HYBRID_SETTLE_AFTER: Duration = Duration::from_millis(160);
    const HYBRID_PREVIEW_FRAME: Duration = Duration::from_millis(33);
    // Watch only metadata at a bounded cadence. Debounce atomic saves to
    // avoid starting a new WKWebView while the editor is still writing.
    const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(150);
    const WATCH_SETTLE_DELAY: Duration = Duration::from_millis(250);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct WatchRevision {
        modified: Option<SystemTime>,
        len: u64,
    }

    impl WatchRevision {
        fn read(path: &Path) -> Result<Self> {
            let metadata = fs::metadata(path)?;
            Ok(Self {
                modified: metadata.modified().ok(),
                len: metadata.len(),
            })
        }
    }

    struct WatchState {
        path: PathBuf,
        applied: WatchRevision,
        candidate: Option<(WatchRevision, Instant)>,
        last_poll: Instant,
    }

    impl WatchState {
        fn new(path: &Path) -> Result<Self> {
            Ok(Self {
                path: path.to_path_buf(),
                applied: WatchRevision::read(path)?,
                candidate: None,
                last_poll: Instant::now(),
            })
        }

        fn should_reload(&mut self, now: Instant) -> bool {
            if now.saturating_duration_since(self.last_poll) < WATCH_POLL_INTERVAL {
                return false;
            }
            self.last_poll = now;
            match WatchRevision::read(&self.path) {
                Ok(revision) => self.observe(revision, now),
                Err(_) => {
                    // Atomic rename/saves can temporarily hide the file.
                    self.candidate = None;
                    false
                }
            }
        }

        // A manual 'r' reload uses the same HTML/WebKit replacement path as
        // --watch. Mark the *pre-read* file revision as rendered to avoid a
        // redundant watch reload while still noticing subsequent edits.
        fn acknowledge_loaded(&mut self, revision: WatchRevision) {
            self.applied = revision;
            self.candidate = None;
        }

        fn observe(&mut self, revision: WatchRevision, now: Instant) -> bool {
            if revision == self.applied {
                self.candidate = None;
                return false;
            }
            if let Some((previous, since)) = self.candidate {
                if previous == revision {
                    if now.saturating_duration_since(since) >= WATCH_SETTLE_DELAY {
                        // Do not acknowledge the new revision until rebuilding
                        // the Markdown succeeds. If a save is temporarily
                        // unreadable, the same revision must be retried.
                        self.candidate = None;
                        return true;
                    }
                    return false;
                }
            }
            self.candidate = Some((revision, now));
            false
        }
    }

    // In the standard mode, the raster outline handles uncached movement.
    // WebKit only fills the settled viewport, not speculative future tiles.
    fn should_capture_settled(idle_for: Duration, tile_ready: bool) -> bool {
        idle_for >= HYBRID_SETTLE_AFTER && !tile_ready
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ScrollDirection {
        Forward,
        Backward,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum InputAction {
        Continue,
        Select,
        OpenLink,
        Reload,
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
            // Match the production CLI: g jumps to top and G to the
            // last scrollable viewport. Jumps use the existing redraw and
            // capture-after-settle path; do not force WebKit prefetch.
            KeyCode::Char('g') if key.kind == KeyEventKind::Press => 0.0,
            KeyCode::Char('G') if key.kind == KeyEventKind::Press => {
                (document_height - f64::from(viewport_height)).max(0.0)
            }
            KeyCode::Char('y') => return InputAction::Select,
            KeyCode::Char('o') if key.kind == KeyEventKind::Press => {
                return InputAction::OpenLink;
            }
            KeyCode::Char('r') if key.kind == KeyEventKind::Press => {
                return InputAction::Reload;
            }
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

    // A cache miss temporarily freezes the visible image until prefetch completes.
    // Track these intervals to distinguish snapshot delays from placement jitter.
    #[derive(Default)]
    struct CacheWaitStats {
        since: Option<Instant>,
        episodes: usize,
        longest: Duration,
    }

    impl CacheWaitStats {
        fn observe(&mut self, covered: bool, now: Instant) {
            match (covered, self.since) {
                (false, None) => {
                    self.since = Some(now);
                    self.episodes += 1;
                }
                (true, Some(started)) => {
                    self.longest = self.longest.max(now.saturating_duration_since(started));
                    self.since = None;
                }
                _ => {}
            }
        }

        fn elapsed(&self, now: Instant) -> Duration {
            self.since.map_or(Duration::ZERO, |started| {
                now.saturating_duration_since(started)
            })
        }

        fn worst(&self, now: Instant) -> Duration {
            self.longest.max(self.elapsed(now))
        }
    }

    // Per-run high-water marks: retain diagnostics after the screen catches up.
    // 'snap' includes WebKit capture + PNG encoding, 'upload' measures the
    // Kitty terminal transfer. Skipped captures still count towards snap.
    #[derive(Default)]
    struct TileTimingStats {
        max_snapshot_ms: f64,
        max_capture_ms: f64,
        max_encode_ms: f64,
        max_write_ms: f64,
        max_upload_ms: f64,
    }

    impl TileTimingStats {
        fn observe_snapshot(&mut self, elapsed_ms: f64) {
            if elapsed_ms.is_finite() {
                self.max_snapshot_ms = self.max_snapshot_ms.max(elapsed_ms);
            }
        }

        fn observe_stages(&mut self, capture_ms: f64, encode_ms: f64, write_ms: f64) {
            if capture_ms.is_finite() {
                self.max_capture_ms = self.max_capture_ms.max(capture_ms);
            }
            if encode_ms.is_finite() {
                self.max_encode_ms = self.max_encode_ms.max(encode_ms);
            }
            if write_ms.is_finite() {
                self.max_write_ms = self.max_write_ms.max(write_ms);
            }
        }

        fn observe_uploaded(&mut self, cache: &ImageCache) {
            self.observe_snapshot(cache.last_snapshot_ms);
            self.observe_stages(
                cache.last_capture_ms,
                cache.last_encode_ms,
                cache.last_write_ms,
            );
            if cache.last_upload_ms.is_finite() {
                self.max_upload_ms = self.max_upload_ms.max(cache.last_upload_ms);
            }
        }
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
        last_capture_ms: f64,
        last_encode_ms: f64,
        last_write_ms: f64,
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
            for id in FIRST_IMAGE_ID..=SECOND_RESIZE_IMAGE_ID {
                let _ = delete_image(&mut stdout, id);
            }
            for id in hybrid_image_preview::FIRST_PREVIEW_ID..=hybrid_image_preview::LAST_PREVIEW_ID
            {
                let _ = delete_image(&mut stdout, id);
            }
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
                Path::new(env!("CARGO_MANIFEST_DIR")).join("native/macos/webkit_viewport.swift");
            // The generated HTML must be readable by WebKit, but relative
            // Markdown images may live outside the source directory.
            // Reuse the legacy snapshot's asset-aware root calculation.
            let html_source =
                fs::read_to_string(html).context("unable to read WebKit HTML input")?;
            let read_root = if html.starts_with(source_dir) {
                doctty::io::webkit_snapshot::compiled_renderer_read_root(&html_source, source_dir, html)?
            } else {
                // Out-of-tree generated HTML has only staged media inside its
                // private temp folder; never combine this with source_dir.
                html.parent()
                    .context("isolated HTML has no parent directory")?
                    .to_path_buf()
            };
            if read_root == Path::new("/") {
                bail!("refusing unrestricted WebKit filesystem read access");
            }
            // Compare the interpreter with a compiled executable without
            // changing the image production or cache algorithm.
            let mut command = if let Some(binary) = std::env::var_os("DOCTTY_WEBKIT_HELPER_BIN") {
                let path = PathBuf::from(binary);
                if !path.is_file() {
                    bail!("DOCTTY_WEBKIT_HELPER_BIN is not a file: {}", path.display());
                }
                Command::new(path)
            } else {
                let mut command = Command::new("swift");
                command.arg("-swift-version").arg("5").arg(helper);
                command
            };
            let mut child = command
                .arg(html)
                .arg(&read_root)
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
                if std::env::var_os("DOCTTY_WEBKIT_HELPER_BIN").is_some() {
                    bail!(
                        "WebKit helper closed its output; see Swift stderr. \
                         If this is a downloaded, non-notarized macOS bundle, \
                         Gatekeeper may have blocked doctty-webkit-swift separately. \
                         Verify the archive checksum, then check System Settings > \
                         Privacy & Security for a blocked helper. \
                         Do not disable Gatekeeper system-wide."
                    );
                }
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
        Snapshot {
            origin: f64,
            image_id: u32,
            requested_at: f64,
        },
        Select,
    }

    enum WorkerEvent {
        Snapshot(ImageCache),
        Skipped {
            snapshot_ms: f64,
            capture_ms: f64,
            encode_ms: f64,
            write_ms: f64,
        },
        Selection(String),
        Error(String),
    }

    // A finished tile may be too old to be useful after a long trackpad
    // gesture. Keep directional prefetches if they still cover the viewport;
    // skip only snapshots made obsolete by substantial user movement.
    fn snapshot_is_obsolete(
        requested_at: f64,
        current_position: f64,
        snapshot_y: f64,
        viewport_height: u32,
    ) -> bool {
        let h = f64::from(viewport_height);
        (current_position - requested_at).abs() > h
            && (current_position < snapshot_y - 0.5
                || current_position + h > snapshot_y + 2.0 * h + 0.5)
    }

    // WebKit operations and PNG uploading run off the event loop. The worker
    // transmits images without placing them; only the UI thread changes placements.
    fn spawn_worker(
        mut swift: SwiftProcess,
        png_path: PathBuf,
        viewport_width: u32,
        viewport_height: u32,
        latest_tiled_position: Option<Arc<AtomicU64>>,
    ) -> (
        Sender<WorkerCommand>,
        Receiver<WorkerEvent>,
        thread::JoinHandle<SwiftProcess>,
    ) {
        let (requests, incoming) = mpsc::channel::<WorkerCommand>();
        let (outgoing, responses) = mpsc::channel::<WorkerEvent>();
        let worker = thread::spawn(move || {
            while let Ok(command) = incoming.recv() {
                let reply = match command {
                    WorkerCommand::Snapshot {
                        origin,
                        image_id,
                        requested_at,
                    } => {
                        let mut snapshot_ms = 0.0;
                        let mut capture_ms = 0.0;
                        let mut encode_ms = 0.0;
                        let mut write_ms = 0.0;
                        let result = (|| -> Result<Option<ImageCache>> {
                            let response =
                                swift.request(&json!({"type": "snapshot", "y": origin}))?;
                            if response["kind"] != "snapshot" {
                                bail!("unexpected WebKit snapshot reply: {response}");
                            }
                            snapshot_ms = response["elapsedMs"].as_f64().unwrap_or(0.0);
                            capture_ms = response["captureMs"].as_f64().unwrap_or(0.0);
                            encode_ms = response["encodeMs"].as_f64().unwrap_or(0.0);
                            write_ms = response["writeMs"].as_f64().unwrap_or(0.0);
                            let actual_y = response["actualY"].as_f64().unwrap_or(origin);
                            let obsolete = || {
                                latest_tiled_position.as_ref().is_some_and(|latest| {
                                    snapshot_is_obsolete(
                                        requested_at,
                                        f64::from_bits(latest.load(Ordering::Relaxed)),
                                        actual_y,
                                        viewport_height,
                                    )
                                })
                            };
                            // A long scroll can make a capture obsolete while
                            // WebKit is rendering it. Avoid needless PNG reads
                            // and Kitty transfers if the viewport has moved on.
                            if obsolete() {
                                return Ok(None);
                            }
                            let png = fs::read(&png_path).context("missing prefetched PNG")?;
                            let decoder = PngDecoder::new(io::Cursor::new(&png))?;
                            let (image_width, image_height) = decoder.dimensions();
                            let scale = f64::from(image_width) / f64::from(viewport_width);
                            if scale <= 0.0 {
                                bail!("invalid prefetched PNG width");
                            }
                            // The user can move again during file IO/decoding.
                            // Recheck just before the expensive Kitty upload.
                            if obsolete() {
                                return Ok(None);
                            }
                            // Never place the image before the complete transmission.
                            let upload_started = Instant::now();
                            let mut stdout = io::stdout().lock();
                            upload_png(&mut stdout, image_id, &png)?;
                            stdout.flush()?;
                            let upload_ms = upload_started.elapsed().as_secs_f64() * 1000.0;
                            Ok(Some(ImageCache {
                                image_id,
                                source_y: actual_y,
                                image_width,
                                image_height,
                                pixels_per_point: scale,
                                document_height: response["height"].as_f64().unwrap_or(0.0),
                                last_snapshot_ms: snapshot_ms,
                                last_capture_ms: capture_ms,
                                last_encode_ms: encode_ms,
                                last_write_ms: write_ms,
                                last_upload_ms: upload_ms,
                                png_bytes: png.len(),
                            }))
                        })();
                        match result {
                            Ok(Some(cache)) => WorkerEvent::Snapshot(cache),
                            Ok(None) => WorkerEvent::Skipped {
                                snapshot_ms,
                                capture_ms,
                                encode_ms,
                                write_ms,
                            },
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
            // Hand the live WKWebView process back to the resize coordinator.
            swift
        });
        (requests, responses, worker)
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
                // The cache holds about two viewports. Start the next image as
                // soon as navigation enters the first viewport rather than
                // waiting until the user is close to the cache edge.
                ScrollDirection::Forward
                    if desired > active.source_y + 0.5 && active.source_y < max_origin =>
                {
                    (active.source_y + viewport).min(max_origin)
                }
                ScrollDirection::Backward
                    if desired <= cache_bottom - viewport && active.source_y > 0.0 =>
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
        previous_image: Option<u32>,
        clear_preview: bool,
    ) -> Result<()> {
        let started = Instant::now();
        let mut out = io::stdout().lock();
        if clear_preview {
            // Erase only preview text, not cached Kitty image data.
            execute!(out, cursor::MoveTo(0, 0), Clear(ClearType::FromCursorDown))?;
        }
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
        if let Some(previous) = previous_image.filter(|id| *id != cache.image_id) {
            hide_image_placement(&mut out, previous)?;
        }
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

    // Update the status line while the image is frozen; otherwise the existing
    // status disappears exactly when a cache miss is causing visible stutter.
    fn display_cache_wait(
        cache: &ImageCache,
        position: f64,
        columns: u16,
        rows: u16,
        viewport_height: u32,
        pending: bool,
        stats: &CacheWaitStats,
        now: Instant,
        timings: Option<&TileTimingStats>,
    ) -> Result<()> {
        let cache_end = cache.source_y
            + (f64::from(cache.image_height) / cache.pixels_per_point - f64::from(viewport_height))
                .max(0.0);
        let status = if let Some(timing) = timings {
            // Show useful values at the beginning: narrow panes truncate HUD.
            format!(
                "WAIT snapMax={:.0} capMax={:.0} encMax={:.0} wrMax={:.0} upMax={:.0} wait={:.0}ms",
                timing.max_snapshot_ms,
                timing.max_capture_ms,
                timing.max_encode_ms,
                timing.max_write_ms,
                timing.max_upload_ms,
                stats.elapsed(now).as_secs_f64() * 1000.0
            )
        } else {
            format!(
                "WAIT {:.0}ms pending={pending} count={} max={:.0}ms y={position:.0} cache={:.0}..{cache_end:.0} snap={:.0}ms up={:.0}ms",
                stats.elapsed(now).as_secs_f64() * 1000.0,
                stats.episodes,
                stats.worst(now).as_secs_f64() * 1000.0,
                cache.source_y,
                cache.last_snapshot_ms,
                cache.last_upload_ms
            )
        };
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
        Ok(())
    }

    // The regular doctty CLI resolves --theme system and passes light/dark
    // to the native companion. Direct companion launches retain the
    // historical light-theme default unless this variable is supplied.
    fn webkit_theme(value: Option<&OsStr>) -> Result<Theme> {
        match value {
            None => Ok(Theme::Light),
            Some(v) if v == "light" => Ok(Theme::Light),
            Some(v) if v == "dark" => Ok(Theme::Dark),
            Some(v) if v == "system" => Ok(Theme::System.resolve()),
            Some(v) => bail!("invalid DOCTTY_WEBKIT_THEME: {v:?} (expected light or dark)"),
        }
    }

    // The bundled doctty CLI forwards its regular Mermaid setting here.
    // Keep standalone experiment launches disabled as before.
    fn webkit_mermaid_mode(value: Option<&OsStr>) -> Result<MermaidMode> {
        match value {
            None => Ok(MermaidMode::Disabled),
            Some(v) if v == "enabled" => Ok(MermaidMode::Enabled),
            Some(v) if v == "disabled" => Ok(MermaidMode::Disabled),
            Some(v) => bail!("invalid DOCTTY_WEBKIT_MERMAID: {v:?} (expected enabled or disabled)"),
        }
    }

    // The WebKit helper may read every file below allowingReadAccessTo.
    // Never grant the filesystem root just to render assets referenced from
    // separate directories. Isolate only the affected documents; the normal
    // writable-folder path keeps its zero-copy image behavior.
    const MAX_STAGED_ASSETS: usize = 256;
    const MAX_STAGED_ASSET_BYTES: u64 = 32 * 1024 * 1024;
    const MAX_STAGED_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

    struct HtmlLease {
        _file: Option<tempfile::NamedTempFile>,
        _isolated_assets: Option<tempfile::TempDir>,
    }

    fn asset_file_url(path: &Path) -> String {
        use std::fmt::Write as _;
        let mut result = String::from("file://");
        for byte in path.to_string_lossy().bytes() {
            if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
                result.push(char::from(byte));
            } else {
                let _ = write!(result, "%{byte:02X}");
            }
        }
        result
    }

    fn decoded_local_asset(source_dir: &Path, reference: &str) -> Option<PathBuf> {
        if reference.is_empty()
            || reference.starts_with('#')
            || reference.starts_with("//")
            || reference.starts_with("data:")
            || reference.starts_with("http:")
            || reference.starts_with("https:")
        {
            return None;
        }
        let raw = if let Some(path) = reference.strip_prefix("file://") {
            path
        } else if reference.contains(':') {
            // No custom URI schemes or Windows drive names in image sources.
            return None;
        } else {
            reference
        };
        let raw = raw.split(['?', '#']).next()?;
        let mut decoded = Vec::with_capacity(raw.len());
        let bytes = raw.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                if let (Some(a), Some(b)) = (
                    char::from(bytes[i + 1]).to_digit(16),
                    char::from(bytes[i + 2]).to_digit(16),
                ) {
                    decoded.push(u8::try_from(a * 16 + b).ok()?);
                    i += 3;
                    continue;
                }
            }
            decoded.push(bytes[i]);
            i += 1;
        }
        let decoded = String::from_utf8(decoded).ok()?.replace("&amp;", "&");
        let path = Path::new(&decoded);
        Some(if path.is_absolute() {
            path.to_path_buf()
        } else {
            source_dir.join(path)
        })
    }

    fn stage_image_reference(
        reference: &str,
        source_dir: &Path,
        stage: &Path,
        copies: &mut Vec<(String, String)>,
        total_bytes: &mut u64,
    ) -> Result<String> {
        if let Some((_, uri)) = copies.iter().find(|(original, _)| original == reference) {
            return Ok(uri.clone());
        }
        let Some(path) = decoded_local_asset(source_dir, reference) else {
            return Ok(reference.to_string());
        };
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            return Ok(reference.to_string());
        };
        let extension = extension.to_ascii_lowercase();
        if !matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif"
        ) {
            return Ok(reference.to_string());
        }
        let metadata = match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => return Ok(reference.to_string()), // Preserve missing-image placeholders.
        };
        if copies.len() >= MAX_STAGED_ASSETS
            || metadata.len() > MAX_STAGED_ASSET_BYTES
            || total_bytes.saturating_add(metadata.len()) > MAX_STAGED_TOTAL_BYTES
        {
            bail!(
                "isolated WebKit image staging limit exceeded for {}",
                path.display()
            );
        }
        let assets = stage.join("assets");
        fs::create_dir_all(&assets)?;
        let destination = assets.join(format!("image-{}.{}", copies.len(), extension));
        fs::copy(&path, &destination)
            .with_context(|| format!("unable to stage WebKit image {}", path.display()))?;
        *total_bytes += metadata.len();
        let uri = asset_file_url(&destination);
        copies.push((reference.to_string(), uri.clone()));
        Ok(uri)
    }

    fn stage_isolated_html(html: &str, source_dir: &Path) -> Result<(PathBuf, tempfile::TempDir)> {
        let stage = tempfile::tempdir().context("unable to create isolated WebKit workspace")?;
        let mut copies = Vec::new();
        let mut total_bytes = 0;
        let mut output = String::with_capacity(html.len());
        let mut rest = html;
        // Only the media URL attributes generated by the sanitizer are staged.
        // External resources stay external. Original document paths never gain
        // WebKit read permission just because the HTML lives in /tmp.
        loop {
            let next = ["src=\"", "srcset=\""]
                .into_iter()
                .filter_map(|marker| rest.find(marker).map(|index| (index, marker)))
                .min_by_key(|(index, _)| *index);
            let Some((index, marker)) = next else {
                output.push_str(rest);
                break;
            };
            output.push_str(&rest[..index + marker.len()]);
            rest = &rest[index + marker.len()..];
            let Some(end) = rest.find('"') else {
                output.push_str(rest);
                break;
            };
            let value = &rest[..end];
            if marker == "src=\"" {
                output.push_str(&stage_image_reference(
                    value,
                    source_dir,
                    stage.path(),
                    &mut copies,
                    &mut total_bytes,
                )?);
            } else {
                let mut candidates = Vec::new();
                for candidate in value.split(',') {
                    let trimmed = candidate.trim();
                    let mut pieces = trimmed.splitn(2, char::is_whitespace);
                    let url = pieces.next().unwrap_or_default();
                    let descriptor = pieces.next().unwrap_or_default().trim();
                    let staged = stage_image_reference(
                        url,
                        source_dir,
                        stage.path(),
                        &mut copies,
                        &mut total_bytes,
                    )?;
                    candidates.push(if descriptor.is_empty() {
                        staged
                    } else {
                        format!("{staged} {descriptor}")
                    });
                }
                output.push_str(&candidates.join(", "));
            }
            rest = &rest[end..];
        }
        let html_path = stage.path().join("document.html");
        fs::write(&html_path, output).context("unable to write isolated WebKit HTML")?;
        Ok((html_path, stage))
    }

    fn html_input(
        path: &Path,
        theme: Theme,
    ) -> Result<(PathBuf, Option<HtmlLease>, Option<String>)> {
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
        {
            return Ok((path.to_path_buf(), None, None));
        }
        let source = fs::read_to_string(path).context("failed to read Markdown source")?;
        let root = path.parent().context("source directory unavailable")?;
        let mermaid_mode =
            webkit_mermaid_mode(std::env::var_os("DOCTTY_WEBKIT_MERMAID").as_deref())?;
        let html = build_github_html(&source, root, theme, mermaid_mode)?;
        let first_link = parse_document(path.to_path_buf(), &source)?
            .meta
            .links
            .into_iter()
            .next();

        match tempfile::Builder::new()
            .prefix(".doctty-poc-")
            .suffix(".html")
            .tempfile_in(root)
        {
            Ok(mut file) => {
                let read_root =
                    doctty::io::webkit_snapshot::compiled_renderer_read_root(&html, root, file.path());
                if read_root.is_ok() {
                    file.write_all(html.as_bytes())?;
                    file.flush()?;
                    return Ok((
                        file.path().to_path_buf(),
                        Some(HtmlLease {
                            _file: Some(file),
                            _isolated_assets: None,
                        }),
                        first_link,
                    ));
                }
                // Even a writable document may reference an image from an
                // unrelated filesystem root. Never hand "/" to WebKit.
                drop(file);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
                ) => {}
            Err(error) => {
                return Err(error).context("unable to create temporary HTML alongside Markdown");
            }
        }

        let (html_path, stage) = stage_isolated_html(&html, root)?;
        Ok((
            html_path,
            Some(HtmlLease {
                _file: None,
                _isolated_assets: Some(stage),
            }),
            first_link,
        ))
    }

    // Open links only on explicit 'o' presses. This intentionally reuses the
    // production viewer's browser dispatch; it never calls WebKit or changes
    // the image/tile cache during scrolling.
    fn open_document_link(first_link: Option<&str>, columns: u16, rows: u16) -> Result<()> {
        let message = match first_link {
            Some(url) => match browser::open_url(url) {
                Ok(()) => format!("opened {url}"),
                Err(error) => format!("open failed: {error}"),
            },
            None => "no link in document".to_string(),
        };
        let clean: String = message
            .chars()
            .filter(|character| !character.is_control())
            .take(usize::from(columns))
            .collect();
        let mut out = io::stdout().lock();
        execute!(out, cursor::MoveTo(0, rows), Clear(ClearType::CurrentLine))?;
        write!(out, "{clean}")?;
        out.flush()?;
        Ok(())
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

    fn hide_image_placement(out: &mut impl Write, image_id: u32) -> Result<()> {
        // Lowercase 'i' deletes only the placement, retaining Kitty image data.
        write!(out, "\x1b_Ga=d,d=i,i={image_id},p=1,q=2;\x1b\\")?;
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
        document_height_hint: f64,
        initial_image_id: Option<u32>,
        cache: &mut Option<ImageCache>,
    ) -> Result<f64> {
        let started = Instant::now();
        let cache_hit = cache
            .as_ref()
            .is_some_and(|entry| entry.covers(position, viewport_height));

        if !cache_hit {
            let document_height = cache
                .as_ref()
                .map_or(document_height_hint, |entry| entry.document_height);
            let requested_y = (position - f64::from(viewport_height) / 2.0)
                .max(0.0)
                .min((document_height - 2.0 * f64::from(viewport_height)).max(0.0));
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
            let new_id = initial_image_id.unwrap_or_else(|| {
                if cache
                    .as_ref()
                    .is_some_and(|entry| entry.image_id == FIRST_IMAGE_ID)
                {
                    SECOND_IMAGE_ID
                } else {
                    FIRST_IMAGE_ID
                }
            });
            let new_cache = ImageCache {
                image_id: new_id,
                source_y: result["actualY"].as_f64().unwrap_or(0.0),
                image_width,
                image_height,
                pixels_per_point: scale,
                document_height: result["height"].as_f64().unwrap_or(0.0),
                last_snapshot_ms: result["elapsedMs"].as_f64().unwrap_or(0.0),
                last_capture_ms: result["captureMs"].as_f64().unwrap_or(0.0),
                last_encode_ms: result["encodeMs"].as_f64().unwrap_or(0.0),
                last_write_ms: result["writeMs"].as_f64().unwrap_or(0.0),
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

    #[derive(Clone, Debug, PartialEq)]
    struct OutlineItem {
        y: f64,
        text: String,
    }

    fn parse_outline(value: &Value) -> Result<Vec<OutlineItem>> {
        if value["kind"] != "outline" {
            bail!("unexpected outline reply: {value}");
        }
        let items = value["items"].as_array().context("missing outline items")?;
        let mut result = Vec::with_capacity(items.len().min(12_000));
        for item in items.iter().take(12_000) {
            if let (Some(y), Some(text)) = (item["y"].as_f64(), item["text"].as_str()) {
                if y.is_finite() && y >= 0.0 && !text.is_empty() {
                    result.push(OutlineItem {
                        y,
                        text: text.to_owned(),
                    });
                }
            }
        }
        result.sort_by(|a, b| a.y.total_cmp(&b.y));
        Ok(result)
    }

    fn parse_preview_scene(value: &Value) -> Result<hybrid_image_preview::Scene> {
        let items = value["items"].as_array().context("missing outline items")?;
        let mut scene = hybrid_image_preview::Scene {
            background: value["background"].as_str().unwrap_or("#ffffff").to_owned(),
            items: Vec::with_capacity(items.len().min(12_000)),
        };
        for item in items.iter().take(12_000) {
            let (Some(y), Some(text)) = (item["y"].as_f64(), item["text"].as_str()) else {
                continue;
            };
            if !y.is_finite() || y < 0.0 || text.is_empty() {
                continue;
            }
            let number = |name: &str, fallback: f64| {
                item[name]
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .unwrap_or(fallback)
            };
            scene.items.push(hybrid_image_preview::StyledItem {
                y,
                x: number("x", 8.0),
                width: number("width", 300.0),
                height: number("height", 30.0),
                font_size: number("fontSize", 16.0),
                line_height: number("lineHeight", 24.0),
                weight: item["weight"].as_u64().unwrap_or(400).min(900) as u16,
                color: item["color"].as_str().unwrap_or("#1f2328").to_owned(),
                background: item["background"].as_str().unwrap_or("").to_owned(),
                tag: item["tag"].as_str().unwrap_or("p").to_owned(),
                text: text.to_owned(),
            });
        }
        scene.items.sort_by(|a, b| a.y.total_cmp(&b.y));
        Ok(scene)
    }

    fn preview_lines(
        outline: &[OutlineItem],
        position: f64,
        viewport_height: u32,
        rows: u16,
    ) -> Vec<(u16, String)> {
        if rows == 0 || viewport_height == 0 {
            return Vec::new();
        }
        let end = position + f64::from(viewport_height);
        let start = outline.partition_point(|entry| entry.y < position);
        let mut visible = Vec::new();
        let mut last_row = None;
        for item in &outline[start..] {
            if item.y >= end {
                break;
            }
            let row =
                ((item.y - position) * f64::from(rows) / f64::from(viewport_height)).floor() as u16;
            if row >= rows || last_row == Some(row) {
                continue;
            }
            visible.push((row, item.text.clone()));
            last_row = Some(row);
        }
        visible
    }

    fn display_preview(
        outline: &[OutlineItem],
        position: f64,
        columns: u16,
        rows: u16,
        viewport_height: u32,
        previously_placed: Option<u32>,
    ) -> Result<()> {
        let mut out = io::stdout().lock();
        if let Some(image_id) = previously_placed {
            hide_image_placement(&mut out, image_id)?;
        }
        // CSI 0 J clears text but does not wipe the terminal's image cache.
        // Full-screen Clear(All) also clears Kitty placements and risks losing
        // otherwise reusable image data when switching back to WebKit.
        execute!(out, cursor::MoveTo(0, 0), Clear(ClearType::FromCursorDown))?;
        for (row, line) in preview_lines(outline, position, viewport_height, rows) {
            execute!(out, cursor::MoveTo(0, row))?;
            let clean: String = line
                .chars()
                .filter(|ch| !ch.is_control())
                .take(usize::from(columns).saturating_sub(1))
                .collect();
            write!(out, "{clean}")?;
        }
        execute!(out, cursor::MoveTo(0, rows))?;
        write!(out, "PREVIEW y={position:.0} (WebKit image pending)")?;
        out.flush()?;
        Ok(())
    }

    // Viewport-sized origin steps with 2-viewport overlap. This first tiled
    // experiment can render a viewport from one cached snapshot; no compositing
    // or changes to the stable viewer are required.
    fn select_tile(
        cached: &[ImageCache],
        position: f64,
        viewport_height: u32,
    ) -> Option<&ImageCache> {
        cached
            .iter()
            .filter(|entry| entry.covers(position, viewport_height))
            .min_by(|a, b| {
                let midpoint_a = a.source_y + f64::from(viewport_height) * 0.5;
                let midpoint_b = b.source_y + f64::from(viewport_height) * 0.5;
                (position - midpoint_a)
                    .abs()
                    .total_cmp(&(position - midpoint_b).abs())
            })
    }

    // Directional queue priority: current viewport, two ahead, one behind.
    // Work is admitted only when the previous snapshot has finished, so
    // unused tiles cannot accumulate in an unbounded worker queue.
    fn next_tile_origin(
        cached: &[ImageCache],
        position: f64,
        viewport_height: u32,
        document_height: f64,
        direction: ScrollDirection,
    ) -> Option<f64> {
        let height = f64::from(viewport_height);
        let max_origin = (document_height - height * 2.0).max(0.0);
        let tile_index = (position / height).floor() as i64;
        let offsets: [i64; 4] = match direction {
            ScrollDirection::Forward => [0, 1, 2, -1],
            ScrollDirection::Backward => [0, -1, -2, 1],
        };
        offsets
            .into_iter()
            .filter_map(|delta| tile_index.checked_add(delta))
            .filter(|index| *index >= 0)
            .map(|index| ((index as f64) * height).min(max_origin))
            .find(|origin| {
                !cached
                    .iter()
                    .any(|entry| (entry.source_y - origin).abs() <= 1.0)
            })
    }

    // A 2-viewport initial/idle-recovery tile need not sit on the regular
    // 1-viewport grid. It may occupy one of the four cache slots while all
    // four prefetch origins compete for the other three. Without a priority
    // check the fourth candidate evicts the third, which is immediately
    // requested again even when the user has stopped scrolling.
    fn prefetch_rank(
        origin: f64,
        position: f64,
        viewport_height: u32,
        document_height: f64,
        direction: ScrollDirection,
    ) -> usize {
        let height = f64::from(viewport_height);
        let max_origin = (document_height - height * 2.0).max(0.0);
        let index = (position / height).floor() as i64;
        let offsets: [i64; 4] = match direction {
            ScrollDirection::Forward => [0, 1, 2, -1],
            ScrollDirection::Backward => [0, -1, -2, 1],
        };
        offsets
            .into_iter()
            .enumerate()
            .filter_map(|(rank, delta)| index.checked_add(delta).map(|value| (rank, value)))
            .filter(|(_, value)| *value >= 0)
            .find(|(_, value)| (((*value as f64) * height).min(max_origin) - origin).abs() <= 1.0)
            .map_or(usize::MAX, |(rank, _)| rank)
    }

    fn should_admit_tile(
        cached: &[ImageCache],
        position: f64,
        viewport_height: u32,
        document_height: f64,
        direction: ScrollDirection,
        origin: f64,
        displayed_id: Option<u32>,
    ) -> bool {
        if cached.len() < MAX_TILE_CACHE {
            return true;
        }
        // Match evict_tile's actual selection, then admit only if the new
        // tile outranks the victim. Otherwise stop at a stable cache set.
        let victim = cached
            .iter()
            .filter(|tile| {
                Some(tile.image_id) != displayed_id && !tile.covers(position, viewport_height)
            })
            .max_by(|a, b| {
                (a.source_y - position)
                    .abs()
                    .total_cmp(&(b.source_y - position).abs())
            });
        victim.is_some_and(|old| {
            prefetch_rank(
                origin,
                position,
                viewport_height,
                document_height,
                direction,
            ) < prefetch_rank(
                old.source_y,
                position,
                viewport_height,
                document_height,
                direction,
            )
        })
    }

    fn tile_image_id(cached: &[ImageCache]) -> Option<u32> {
        (FIRST_IMAGE_ID..=LAST_TILE_IMAGE_ID)
            .find(|id| cached.iter().all(|entry| entry.image_id != *id))
    }

    fn evict_tile(
        cached: &mut Vec<ImageCache>,
        position: f64,
        viewport_height: u32,
        displayed_image_id: Option<u32>,
    ) -> Option<ImageCache> {
        let index = cached
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                Some(entry.image_id) != displayed_image_id
                    && !entry.covers(position, viewport_height)
            })
            .max_by(|(_, a), (_, b)| {
                (a.source_y - position)
                    .abs()
                    .total_cmp(&(b.source_y - position).abs())
            })
            .map(|(index, _)| index)?;
        Some(cached.remove(index))
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum HybridDisplayMode {
        Image,
        Preview,
        Hold,
    }

    // Once the user has seen the preview, keep it for the rest of a fast
    // gesture. Repeated image/text switches at cache boundaries are jarring.
    // Idle and tile availability are both required to restore WebKit.
    fn hybrid_display_mode(
        tile_ready: bool,
        preview_active: bool,
        missing_for: Duration,
        idle_for: Duration,
    ) -> HybridDisplayMode {
        if preview_active {
            if tile_ready && idle_for >= HYBRID_SETTLE_AFTER {
                HybridDisplayMode::Image
            } else {
                HybridDisplayMode::Preview
            }
        } else if tile_ready {
            HybridDisplayMode::Image
        } else if missing_for >= HYBRID_PREVIEW_AFTER {
            HybridDisplayMode::Preview
        } else {
            HybridDisplayMode::Hold
        }
    }

    // At rest, a missing viewport is more important than speculative tiles
    // ahead or behind it. Use a centered origin rather than a distant grid
    // tile. This avoids a parked preview whose nearby cache images don't
    // actually cover the requested viewport.
    fn recovery_tile_origin(
        cached: &[ImageCache],
        position: f64,
        viewport_height: u32,
        document_height: f64,
    ) -> Option<f64> {
        if select_tile(cached, position, viewport_height).is_some() {
            return None;
        }
        let height = f64::from(viewport_height);
        let max_origin = (document_height - height * 2.0).max(0.0);
        let candidates = [
            (position - height * 0.5).clamp(0.0, max_origin),
            position.clamp(0.0, max_origin),
            (position - height * 0.75).clamp(0.0, max_origin),
        ];
        candidates.into_iter().find(|origin| {
            cached
                .iter()
                .all(|tile| (tile.source_y - origin).abs() > 1.0)
        })
    }

    // Independent prototype for comparison with the original single-prefetch
    // viewer. All costly WebKit captures and PNG transfers stay on the worker.
    // The event loop never waits for a snapshot. Images held by Kitty are
    // bounded to MAX_TILE_CACHE + one upload and freed on eviction/exit.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum ViewerExit {
        Quit,
        Resized(f64),
        Reload(f64),
    }

    // Use relative document progress because a width change causes reflow.
    fn reading_fraction(position: f64, height: f64, viewport_height: u32) -> f64 {
        let max_scroll = (height - f64::from(viewport_height)).max(0.0);
        if max_scroll <= 0.0 {
            0.0
        } else {
            (position / max_scroll).clamp(0.0, 1.0)
        }
    }

    fn resumed_position(fraction: f64, height: f64, viewport_height: u32) -> f64 {
        fraction.clamp(0.0, 1.0) * (height - f64::from(viewport_height)).max(0.0)
    }

    fn clear_session_images_except(keep_image_id: u32) -> Result<()> {
        let mut out = io::stdout().lock();
        for id in FIRST_IMAGE_ID..=SECOND_RESIZE_IMAGE_ID {
            if id != keep_image_id {
                delete_image(&mut out, id)?;
            }
        }
        for id in hybrid_image_preview::FIRST_PREVIEW_ID..=hybrid_image_preview::LAST_PREVIEW_ID {
            delete_image(&mut out, id)?;
        }
        // Do not clear the terminal: the newly placed image is already visible.
        out.flush()?;
        Ok(())
    }

    fn run_tiled(
        swift: SwiftProcess,
        png_path: PathBuf,
        columns: u16,
        rows: u16,
        viewport_width: u32,
        viewport_height: u32,
        initial: ImageCache,
        initial_position: f64,
        outline: Option<Vec<OutlineItem>>,
        image_scene: Option<hybrid_image_preview::Scene>,
        capture_after_settle: bool,
        watcher: Option<&mut WatchState>,
        first_link: Option<&str>,
    ) -> Result<(ViewerExit, SwiftProcess)> {
        let mut document_height = initial.document_height;
        let latest_position = Arc::new(AtomicU64::new(initial_position.to_bits()));
        let (requests, responses, webkit_worker) = spawn_worker(
            swift,
            png_path,
            viewport_width,
            viewport_height,
            Some(Arc::clone(&latest_position)),
        );
        let (preview_requests, preview_responses, preview_worker) = if let Some(scene) = image_scene
        {
            let (sender, receiver, worker) = hybrid_image_preview::spawn(
                scene,
                viewport_width,
                viewport_height,
                Arc::clone(&latest_position),
            );
            (Some(sender), Some(receiver), Some(worker))
        } else {
            (None, None, None)
        };
        let mut preview_tiles: Vec<hybrid_image_preview::Tile> = Vec::new();
        let mut preview_pending = false;
        let mut placed_preview_id: Option<u32> = None;
        let mut cached = vec![initial];
        let mut position = initial_position;
        let mut direction = ScrollDirection::Forward;
        let mut pending: Option<u32> = None;
        let mut skipped_uploads = 0usize;
        let mut tile_timings = TileTimingStats::default();
        tile_timings.observe_uploaded(&cached[0]);
        let mut showing_preview = false;
        let mut last_preview_position: Option<u64> = None;
        let mut last_preview_frame: Option<Instant> = None;
        let mut last_movement_at = Instant::now();
        let mut last_displayed: Option<(u32, u64)> = None;
        let mut wait = CacheWaitStats::default();
        let mut last_wait_status_at: Option<Instant> = None;

        let mut exit = ViewerExit::Quit;
        let mut watcher = watcher;
        // Always close and join both image workers, even if the event loop
        // exits early with an I/O/rendering error. Otherwise they could still
        // write Kitty graphics after TerminalSession restores the shell.
        let loop_result: Result<ViewerExit> = (|| {
            'viewer: loop {
                if watcher
                    .as_mut()
                    .is_some_and(|state| state.should_reload(Instant::now()))
                {
                    exit = ViewerExit::Reload(reading_fraction(
                        position,
                        document_height,
                        viewport_height,
                    ));
                    break 'viewer;
                }
                if let Some(ref receiver) = preview_responses {
                    loop {
                        match receiver.try_recv() {
                            Ok(hybrid_image_preview::Response::Ready(tile)) => {
                                preview_pending = false;
                                preview_tiles.push(tile);
                            }
                            Ok(hybrid_image_preview::Response::Skipped) => {
                                preview_pending = false;
                            }
                            Ok(hybrid_image_preview::Response::Failed(error)) => {
                                bail!("raster preview worker: {error}");
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                bail!("raster preview worker disconnected");
                            }
                        }
                    }
                }
                loop {
                    match responses.try_recv() {
                        Ok(WorkerEvent::Snapshot(next)) => {
                            pending = None;
                            document_height = next.document_height;
                            tile_timings.observe_uploaded(&next);
                            // Skip images too far behind a quick direction reversal.
                            if (next.source_y - position).abs() > f64::from(viewport_height) * 4.0 {
                                let mut out = io::stdout().lock();
                                delete_image(&mut out, next.image_id)?;
                                out.flush()?;
                            } else {
                                cached.push(next);
                            }
                        }
                        Ok(WorkerEvent::Skipped {
                            snapshot_ms,
                            capture_ms,
                            encode_ms,
                            write_ms,
                        }) => {
                            pending = None;
                            skipped_uploads += 1;
                            tile_timings.observe_snapshot(snapshot_ms);
                            tile_timings.observe_stages(capture_ms, encode_ms, write_ms);
                        }
                        Ok(WorkerEvent::Selection(text)) => {
                            if !text.is_empty() {
                                let mut out = io::stdout().lock();
                                write!(out, "\x1b]52;c;{}\x07", STANDARD.encode(text))?;
                                out.flush()?;
                            }
                        }
                        Ok(WorkerEvent::Error(error)) => bail!("WebKit tile: {error}"),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => bail!("WebKit tile worker disconnected"),
                    }
                }

                let now = Instant::now();
                let selected = select_tile(&cached, position, viewport_height);
                wait.observe(selected.is_some(), now);
                let idle_for = now.saturating_duration_since(last_movement_at);
                let mode = outline.as_ref().map(|_| {
                    hybrid_display_mode(
                        selected.is_some(),
                        showing_preview,
                        wait.elapsed(now),
                        idle_for,
                    )
                });

                if selected.is_some() && mode.is_none_or(|value| value == HybridDisplayMode::Image)
                {
                    let tile = selected.context("tile selection disappeared")?;
                    let fingerprint = (tile.image_id, position.to_bits());
                    if showing_preview || last_displayed != Some(fingerprint) {
                        let status = format!(
                            "TILE snapMax={:.0} capMax={:.0} encMax={:.0} wrMax={:.0} upMax={:.0} wait={:.0}ms",
                            tile_timings.max_snapshot_ms,
                            tile_timings.max_capture_ms,
                            tile_timings.max_encode_ms,
                            tile_timings.max_write_ms,
                            tile_timings.max_upload_ms,
                            wait.worst(now).as_secs_f64() * 1000.0
                        );
                        display_cached(
                            tile,
                            position,
                            columns,
                            rows,
                            viewport_height,
                            &status,
                            placed_preview_id
                                .or_else(|| last_displayed.map(|fingerprint| fingerprint.0)),
                            showing_preview && placed_preview_id.is_none(),
                        )?;
                        placed_preview_id = None;
                        showing_preview = false;
                        last_preview_position = None;
                        last_preview_frame = None;
                        last_displayed = Some(fingerprint);
                    }
                    last_wait_status_at = None;
                } else if mode == Some(HybridDisplayMode::Preview) {
                    let frame_due = last_preview_frame.is_none_or(|last| {
                        now.saturating_duration_since(last) >= HYBRID_PREVIEW_FRAME
                    });
                    if preview_requests.is_some() {
                        if let Some(tile) =
                            hybrid_image_preview::select(&preview_tiles, position, viewport_height)
                        {
                            if last_preview_position != Some(position.to_bits()) && frame_due {
                                let mut out = io::stdout().lock();
                                place_image(
                                    &mut out,
                                    tile.id,
                                    columns,
                                    rows,
                                    tile.width,
                                    hybrid_image_preview::Tile::viewport_height_px(viewport_height),
                                    tile.source_y(position, viewport_height),
                                )?;
                                if let Some(old_id) = placed_preview_id
                                    .or_else(|| last_displayed.map(|v| v.0))
                                    .filter(|id| *id != tile.id)
                                {
                                    hide_image_placement(&mut out, old_id)?;
                                }
                                out.flush()?;
                                placed_preview_id = Some(tile.id);
                                showing_preview = true;
                                last_displayed = None;
                                last_preview_position = Some(position.to_bits());
                                last_preview_frame = Some(now);
                            }
                        }
                    } else if let Some(ref outline) = outline {
                        if last_preview_position != Some(position.to_bits()) && frame_due {
                            display_preview(
                                outline,
                                position,
                                columns,
                                rows,
                                viewport_height,
                                last_displayed.map(|fingerprint| fingerprint.0),
                            )?;
                            showing_preview = true;
                            last_displayed = None;
                            last_preview_position = Some(position.to_bits());
                            last_preview_frame = Some(now);
                        }
                    }
                } else if last_wait_status_at.is_none_or(|previous| {
                    now.saturating_duration_since(previous) >= Duration::from_millis(100)
                }) {
                    // No image can cover this position yet. Keep the previous
                    // placement until the worker publishes the missing tile.
                    let last = cached
                        .iter()
                        .find(|entry| Some(entry.image_id) == last_displayed.map(|v| v.0));
                    if let Some(tile) = last.or_else(|| cached.first()) {
                        display_cache_wait(
                            tile,
                            position,
                            columns,
                            rows,
                            viewport_height,
                            pending.is_some(),
                            &wait,
                            now,
                            Some(&tile_timings),
                        )?;
                    }
                    last_wait_status_at = Some(now);
                }

                if let Some(ref sender) = preview_requests {
                    if !preview_pending
                        && mode == Some(HybridDisplayMode::Preview)
                        && hybrid_image_preview::select(&preview_tiles, position, viewport_height)
                            .is_none()
                    {
                        if preview_tiles.len() >= hybrid_image_preview::MAX_PREVIEW_CACHE {
                            if let Some(old) = hybrid_image_preview::evict(
                                &mut preview_tiles,
                                position,
                                viewport_height,
                                placed_preview_id,
                            ) {
                                let mut out = io::stdout().lock();
                                delete_image(&mut out, old.id)?;
                                out.flush()?;
                            }
                        }
                        if preview_tiles.len() < hybrid_image_preview::MAX_PREVIEW_CACHE {
                            if let Some(image_id) = hybrid_image_preview::free_id(&preview_tiles) {
                                let origin = hybrid_image_preview::tile_origin(
                                    position,
                                    viewport_height,
                                    document_height,
                                );
                                sender.send(hybrid_image_preview::Request {
                                    origin,
                                    image_id,
                                    requested_at: position,
                                })?;
                                preview_pending = true;
                            }
                        }
                    }
                }

                if pending.is_none()
                    && (!capture_after_settle || should_capture_settled(idle_for, selected.is_some()))
                {
                    let recovery =
                        outline.is_some() && selected.is_none() && idle_for >= HYBRID_SETTLE_AFTER;
                    let origin = if recovery {
                        recovery_tile_origin(&cached, position, viewport_height, document_height)
                    } else {
                        next_tile_origin(
                            &cached,
                            position,
                            viewport_height,
                            document_height,
                            direction,
                        )
                    };
                    if let Some(origin) = origin.filter(|candidate| {
                        // The exact viewport recovery tile must not be blocked
                        // by the speculative prefetch priority guard.
                        recovery
                            || should_admit_tile(
                                &cached,
                                position,
                                viewport_height,
                                document_height,
                                direction,
                                *candidate,
                                last_displayed.map(|v| v.0),
                            )
                    }) {
                        if cached.len() >= MAX_TILE_CACHE {
                            if let Some(old) = evict_tile(
                                &mut cached,
                                position,
                                viewport_height,
                                last_displayed.map(|v| v.0),
                            ) {
                                let mut out = io::stdout().lock();
                                delete_image(&mut out, old.image_id)?;
                                out.flush()?;
                            }
                        }
                        if cached.len() < MAX_TILE_CACHE {
                            if let Some(image_id) = tile_image_id(&cached) {
                                requests.send(WorkerCommand::Snapshot {
                                    origin,
                                    image_id,
                                    requested_at: position,
                                })?;
                                pending = Some(image_id);
                            }
                        }
                    }
                }

                if !event::poll(Duration::from_millis(12))? {
                    continue;
                }
                for index in 0..MAX_INPUT_BATCH {
                    if index > 0 && !event::poll(Duration::ZERO)? {
                        break;
                    }
                    let key = match event::read()? {
                        Event::Resize(new_cols, new_rows) => {
                            if (new_cols, new_rows) != (columns, rows.saturating_add(1)) {
                                exit = ViewerExit::Resized(reading_fraction(
                                    position,
                                    document_height,
                                    viewport_height,
                                ));
                                break 'viewer;
                            }
                            continue;
                        }
                        Event::Key(key) => key,
                        _ => continue,
                    };
                    let previous_position = position;
                    match apply_key(
                        key,
                        &mut position,
                        &mut direction,
                        document_height,
                        viewport_height,
                    ) {
                        InputAction::Quit => break 'viewer,
                        InputAction::Select => requests.send(WorkerCommand::Select)?,
                        InputAction::OpenLink => {
                            open_document_link(first_link, columns, rows)?;
                        }
                        InputAction::Reload => {
                            exit = ViewerExit::Reload(reading_fraction(
                                position,
                                document_height,
                                viewport_height,
                            ));
                            break 'viewer;
                        }
                        InputAction::Continue => {}
                    }
                    if position != previous_position {
                        last_movement_at = Instant::now();
                    }
                    // Publish every accepted movement, including reversals, before
                    // returning to the drawing loop.
                    latest_position.store(position.to_bits(), Ordering::Relaxed);
                }
            }
            Ok(exit)
        })();

        drop(requests);
        drop(preview_requests);
        // Join *both* even if either the event loop or a worker failed.
        // The terminal session must never be restored while graphics writes
        // remain in flight.
        let webkit_result = webkit_worker.join();
        let preview_result = preview_worker.map(|worker| worker.join());
        let exit = loop_result?;
        let mut swift =
            webkit_result.map_err(|_| anyhow::anyhow!("WebKit worker panicked while stopping"))?;
        if preview_result.is_some_and(|result| result.is_err()) {
            bail!("preview worker panicked while stopping");
        }
        // Capture metrics once at teardown, not inside the idle/render loop.
        if let Some(path) = std::env::var_os("DOCTTY_WEBKIT_METRICS_FILE") {
            let metrics = swift.request(&json!({"type": "metrics"}))?;
            let mut report = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            writeln!(report, "{metrics} (session={exit:?})")?;
        }
        Ok((exit, swift))
    }

    // Recreate WebKit with the new CSS width after a resize. Unlike the
    // scroll hot path, this intentionally favors correctness over latency.
    fn run_resizable_tiled(
        document: &Path,
        theme: Theme,
        hybrid: bool,
        hybrid_image: bool,
        capture_after_settle: bool,
        watch: bool,
    ) -> Result<()> {
        let source_dir = document.parent().context("source directory unavailable")?;
        // Observe the initial revision before reading HTML: a save racing
        // with startup is then picked up by the watcher.
        let mut watcher = if watch {
            Some(WatchState::new(document)?)
        } else {
            None
        };
        let (mut html, mut _temp_html, mut first_link) = html_input(document, theme)?;
        let work = tempfile::tempdir()?;
        let output = work.path().join("viewport.png");
        let _session = TerminalSession::enter()?;
        let mut progress = 0.0;

        let mut swift: Option<SwiftProcess> = None;
        let mut last_geometry: Option<(u16, u16)> = None;
        let mut resize_count = 0_usize;
        loop {
            let (columns, total_rows) = terminal::size()?;
            let rows = total_rows.saturating_sub(1).max(1);
            let width = u32::from(columns.max(1)).saturating_mul(8);
            let height = u32::from(rows).saturating_mul(16);
            let mut helper = if let Some(mut existing) = swift.take() {
                if last_geometry != Some((columns, rows)) {
                    let response = existing.request(&json!({
                        "type": "resize", "width": width, "height": height
                    }))?;
                    if response["kind"] != "resized" {
                        bail!("unexpected WebKit resize reply: {response}");
                    }
                }
                existing
            } else {
                let mut process = SwiftProcess::start(&html, source_dir, &output, width, height)?;
                let ready = process.response()?;
                if ready["kind"] != "ready" {
                    bail!("expected WebKit readiness response: {ready}");
                }
                process
            };
            last_geometry = Some((columns, rows));
            // DOM metrics and preview coordinates must be regenerated after
            // WebKit's CSS reflow, even though the WKWebView process is reused.
            let outline_reply = if hybrid {
                Some(helper.request(&json!({"type": "outline"}))?)
            } else {
                None
            };
            let outline = match outline_reply {
                Some(ref value) if !hybrid_image => Some(parse_outline(value)?),
                Some(_) => Some(Vec::new()),
                None => None,
            };
            let scene = if hybrid_image {
                Some(parse_preview_scene(
                    outline_reply.as_ref().context("missing DOM outline")?,
                )?)
            } else {
                None
            };
            let geometry = helper.request(&json!({"type": "dimensions"}))?;
            if geometry["kind"] != "dimensions" {
                bail!("unexpected WebKit dimensions reply: {geometry}");
            }
            let document_height = geometry["height"].as_f64().unwrap_or(0.0);
            let initial_position = resumed_position(progress, document_height, height);
            let mut cached = None;
            let initial_id = if resize_count == 0 {
                FIRST_IMAGE_ID
            } else if resize_count.is_multiple_of(2) {
                FIRST_RESIZE_IMAGE_ID
            } else {
                SECOND_RESIZE_IMAGE_ID
            };
            let _ = render_scroll(
                &mut helper,
                &output,
                initial_position,
                columns,
                rows,
                width,
                height,
                document_height,
                Some(initial_id),
                &mut cached,
            )?;
            // New placement is now complete. Old cached image bytes may be freed.
            clear_session_images_except(initial_id)?;
            let active = cached.context("initial snapshot did not create a cache")?;
            let (exit, returned) = run_tiled(
                helper,
                output.clone(),
                columns,
                rows,
                width,
                height,
                active,
                initial_position,
                outline,
                scene,
                capture_after_settle,
                watcher.as_mut(),
                first_link.as_deref(),
            )?;
            match exit {
                ViewerExit::Resized(next_progress) => {
                    progress = next_progress;
                    resize_count += 1;
                    swift = Some(returned);
                }
                ViewerExit::Reload(next_progress) => {
                    progress = next_progress;
                    resize_count += 1;
                    // Rebuild the HTML from the updated Markdown before
                    // switching helpers. Keep the last valid render if the
                    // document is temporarily unreadable or incomplete.
                    // Take the revision before reading HTML, so an edit
                    // racing with the reload is still detected by --watch.
                    let revision_to_ack = WatchRevision::read(document).ok();
                    match html_input(document, theme) {
                        Ok((new_html, new_temp_html, new_first_link)) => {
                            if let (Some(state), Some(revision)) =
                                (watcher.as_mut(), revision_to_ack)
                            {
                                state.acknowledge_loaded(revision);
                            }
                            drop(returned);
                            // Old temporary HTML stays alive until the old
                            // Swift process has exited.
                            html = new_html;
                            _temp_html = new_temp_html;
                            first_link = new_first_link;
                            swift = None;
                        }
                        Err(_) => {
                            // Keep the last valid render, but do not mark the
                            // changed source as applied. Back off failed
                            // rebuilds to avoid busy retrying a broken file.
                            if let Some(state) = watcher.as_mut() {
                                state.last_poll = Instant::now() + Duration::from_secs(1);
                            }
                            swift = Some(returned);
                        }
                    }
                }
                ViewerExit::Quit => return Ok(()),
            }
        }
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
        // Deliberately bypasses WebKit and the renderer viewport cache.
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

    // is the default for the hybrid image viewer. Preserve the explicit
    // --hybrid-image-removed alias for existing scripts, and retain the old
    // speculative snapshot mode as --hybrid-image-prefetch for A/B testing.
    // Other modes such as --hybrid and --tiled are unchanged.
    fn hybrid_image_flags(first: &OsStr) -> (bool, bool) {
        let eco = first == "--hybrid-image" || first == "--hybrid-image-removed";
        let prefetch = first == "--hybrid-image-prefetch";
        (eco || prefetch, eco)
    }

    pub fn run() -> Result<()> {
        if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
            bail!("run this interactive experiment in Ghostty or Herdr");
        }
        let mut arguments = std::env::args_os().skip(1);
        let first = arguments.next().unwrap_or_else(|| "README.md".into());
        let (hybrid_image, capture_after_settle) = hybrid_image_flags(&first);
        let hybrid = first == "--hybrid" || hybrid_image;
        let tiled = first == "--tiled" || hybrid;
        let arg = if tiled {
            arguments.next().unwrap_or_else(|| "README.md".into())
        } else {
            first
        };
        if arg == "--graphics-probe" {
            return graphics_probe();
        }
        let document = PathBuf::from(arg)
            .canonicalize()
            .context("source file not found")?;
        let dir = document.parent().context("source directory not found")?;
        let theme = webkit_theme(std::env::var_os("DOCTTY_WEBKIT_THEME").as_deref())?;
        let watch = match arguments.next() {
            None => false,
            Some(flag) if flag == "--watch" && hybrid_image => true,
            Some(flag) => bail!("unsupported WebKit experiment option: {flag:?}"),
        };
        if let Some(flag) = arguments.next() {
            bail!("unexpected extra WebKit experiment option: {flag:?}");
        }
        if tiled {
            return run_resizable_tiled(
                &document,
                theme,
                hybrid,
                hybrid_image,
                capture_after_settle,
                watch,
            );
        }
        let (html, _temp_html, first_link) = html_input(&document, theme)?;
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
            &mut swift,
            &output,
            position,
            cols,
            rows,
            width,
            height,
            ready["height"].as_f64().unwrap_or(0.0),
            None,
            &mut cache,
        )?;
        let mut active = cache.context("initial snapshot did not create a cache")?;
        let mut document_height = initial_height;
        let (requests, responses, _worker) = spawn_worker(swift, output, width, height, None);
        let mut pending = false;
        let mut prefetched: Option<ImageCache> = None;
        let mut last_displayed: Option<(u32, u64)> = None;
        let mut direction = ScrollDirection::Forward;
        let mut cache_wait = CacheWaitStats::default();
        let mut last_wait_status_at: Option<Instant> = None;

        'viewer: loop {
            // A completed prefetch is already uploaded to the terminal in the
            // background; activating it requires only a placement and an old delete.
            loop {
                match responses.try_recv() {
                    Ok(WorkerEvent::Snapshot(next)) => {
                        pending = false;
                        prefetched = Some(next);
                    }
                    Ok(WorkerEvent::Skipped { .. }) => {
                        // Only the opt-in tiled worker can skip uploads.
                        pending = false;
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
                    requests.send(WorkerCommand::Snapshot {
                        origin,
                        image_id,
                        requested_at: position,
                    })?;
                    pending = true;
                }
            }

            let now = Instant::now();
            let covered = active.covers(position, height);
            cache_wait.observe(covered, now);
            if covered {
                let state = if prefetched.is_some() {
                    "prefetch READY"
                } else if pending {
                    "prefetch RUNNING"
                } else {
                    "cache HIT"
                };
                let status = format!(
                    "{state} waits={} max={:.0}ms",
                    cache_wait.episodes,
                    cache_wait.worst(now).as_secs_f64() * 1000.0
                );
                // Do not send redundant placements every 12 ms when nothing moved.
                let fingerprint = (active.image_id, position.to_bits());
                if last_displayed != Some(fingerprint) {
                    display_cached(&active, position, cols, rows, height, &status, None, false)?;
                    last_displayed = Some(fingerprint);
                }
                last_wait_status_at = None;
            } else if last_wait_status_at.is_none_or(|previous| {
                now.saturating_duration_since(previous) >= Duration::from_millis(100)
            }) {
                // No Kitty placement here: keep the previous image visible.
                // Throttle diagnostics so the HUD itself does not cause stutter.
                display_cache_wait(
                    &active,
                    position,
                    cols,
                    rows,
                    height,
                    pending,
                    &cache_wait,
                    now,
                    None,
                )?;
                last_wait_status_at = Some(now);
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
                    InputAction::OpenLink => {
                        open_document_link(first_link.as_deref(), cols, rows)?;
                    }
                    InputAction::Reload => {
                        let mut out = io::stdout().lock();
                        execute!(out, cursor::MoveTo(0, rows), Clear(ClearType::CurrentLine))?;
                        write!(out, "manual reload is available with --hybrid-image")?;
                        out.flush()?;
                    }
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
        fn resize_progress_preserves_top_middle_and_bottom() {
            assert_eq!(reading_fraction(0.0, 12000.0, 1000), 0.0);
            assert_eq!(reading_fraction(5500.0, 12000.0, 1000), 0.5);
            assert_eq!(reading_fraction(11000.0, 12000.0, 1000), 1.0);
            assert_eq!(resumed_position(0.5, 24000.0, 2000), 11000.0);
            assert_eq!(resumed_position(1.0, 24000.0, 2000), 22000.0);
            assert_eq!(resumed_position(1.0, 700.0, 1000), 0.0);
        }

        #[test]
        fn hybrid_image_selects_standard_renderer() {
            assert_eq!(
                hybrid_image_flags(OsStr::new("--hybrid-image")),
                (true, true)
            );
            assert_eq!(hybrid_image_flags(OsStr::new("--hybrid-image-removed")), (false, false));
            assert_eq!(hybrid_image_flags(OsStr::new("--hybrid-image-prefetch")), (false, false));
            assert_eq!(hybrid_image_flags(OsStr::new("--hybrid")), (false, false));
            assert_eq!(hybrid_image_flags(OsStr::new("--tiled")), (false, false));
            assert_eq!(hybrid_image_flags(OsStr::new("README.md")), (false, false));
        }

        #[test]
        fn webkit_theme_preserves_standalone_default_and_rejects_bad_values() {
            assert_eq!(webkit_theme(None).ok(), Some(Theme::Light));
            assert_eq!(
                webkit_theme(Some(OsStr::new("light"))).ok(),
                Some(Theme::Light)
            );
            assert_eq!(
                webkit_theme(Some(OsStr::new("dark"))).ok(),
                Some(Theme::Dark)
            );
            assert!(webkit_theme(Some(OsStr::new("neon"))).is_err());
        }

        #[test]
        fn mermaid_mode_defaults_to_standalone_behavior_and_respects_cli() {
            assert_eq!(webkit_mermaid_mode(None).ok(), Some(MermaidMode::Disabled));
            assert_eq!(
                webkit_mermaid_mode(Some(OsStr::new("enabled"))).ok(),
                Some(MermaidMode::Enabled)
            );
            assert_eq!(
                webkit_mermaid_mode(Some(OsStr::new("disabled"))).ok(),
                Some(MermaidMode::Disabled)
            );
            assert!(webkit_mermaid_mode(Some(OsStr::new("invalid"))).is_err());
        }

        #[test]
        fn markdown_html_uses_requested_theme() -> Result<()> {
            let root = tempfile::tempdir()?;
            let md = root.path().join("test-theme.md");
            fs::write(&md, "# Theme preview")?;
            let (light_path, _light_temp, light_link) = html_input(&md, Theme::Light)?;
            assert_eq!(light_link, None);
            let light_html = fs::read_to_string(light_path)?;
            assert!(light_html.contains("data-color-mode=\"light\""));
            let (dark_path, _dark_temp, dark_link) = html_input(&md, Theme::Dark)?;
            assert_eq!(dark_link, None);
            let dark_html = fs::read_to_string(dark_path)?;
            assert!(dark_html.contains("data-color-mode=\"dark\""));
            Ok(())
        }

        #[test]
        fn readonly_document_folder_uses_out_of_tree_html_tempfile() -> Result<()> {
            use std::os::unix::fs::PermissionsExt;

            let root = tempfile::tempdir()?;
            let documents = root.path().join("read-only docs");
            fs::create_dir(&documents)?;
            let markdown = documents.join("README.md");
            fs::write(&markdown, "# Read-only docs\n")?;

            let original = fs::metadata(&documents)?.permissions();
            fs::set_permissions(&documents, fs::Permissions::from_mode(0o555))?;
            let rendered = html_input(&markdown, Theme::Light);
            fs::set_permissions(&documents, original)?;

            let (html, _temp, _) = rendered?;
            assert!(html.exists());
            assert!(!html.starts_with(&documents));

            let content = fs::read_to_string(&html)?;
            let read_root = html.parent().context("staged HTML has no parent")?;
            assert_ne!(read_root, Path::new("/"));
            assert!(html.starts_with(read_root));
            assert!(!markdown.starts_with(read_root));
            assert!(content.contains("Read-only docs"));
            Ok(())
        }

        #[test]
        fn isolated_html_copies_only_media_and_preserves_link_origin() -> Result<()> {
            let source = tempfile::tempdir()?;
            let docs = source.path().join("docs");
            let assets = source.path().join("assets");
            fs::create_dir(&docs)?;
            fs::create_dir(&assets)?;
            fs::write(assets.join("picture with spaces.png"), b"test image")?;
            let input = r#"<base href="file:///original/docs/">
                <img src="../assets/picture%20with%20spaces.png">
                <picture><source srcset="../assets/picture%20with%20spaces.png 1x"></picture>
                <a href="../other.md">Link</a>"#;
            let (html, stage) = stage_isolated_html(input, &docs)?;
            let staged = fs::read_to_string(html)?;
            let expected = asset_file_url(&stage.path().join("assets/image-0.png"));
            assert_eq!(staged.matches(&expected).count(), 2);
            assert!(staged.contains("href=\"../other.md\""));
            assert!(stage.path().join("assets/image-0.png").is_file());
            assert!(!stage.path().join("assets/image-1.png").exists());
            Ok(())
        }

        #[test]
        fn markdown_link_metadata_matches_production_and_tracks_reload() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let md = dir.path().join("links.md");
            fs::write(
                &md,
                "# Links\n\n[first](https://example.com/first) [second](https://example.org/)\n",
            )?;
            let (_, _initial_html, first_link) = html_input(&md, Theme::Light)?;
            assert_eq!(first_link.as_deref(), Some("https://example.com/first"));

            fs::write(&md, "# Updated\n\n[new](https://example.com/new)\n")?;
            let (_, _updated_html, updated_link) = html_input(&md, Theme::Dark)?;
            assert_eq!(updated_link.as_deref(), Some("https://example.com/new"));
            Ok(())
        }

        #[test]
        fn link_action_only_fires_on_press_not_key_repeat_or_release() {
            let mut position = 0.0;
            let mut direction = ScrollDirection::Forward;
            let pressed = key(KeyCode::Char('o'));
            assert_eq!(
                apply_key(pressed, &mut position, &mut direction, 2000.0, 1000),
                InputAction::OpenLink
            );
            let mut repeated = pressed;
            repeated.kind = KeyEventKind::Repeat;
            assert_eq!(
                apply_key(repeated, &mut position, &mut direction, 2000.0, 1000),
                InputAction::Continue
            );
            let mut released = pressed;
            released.kind = KeyEventKind::Release;
            assert_eq!(
                apply_key(released, &mut position, &mut direction, 2000.0, 1000),
                InputAction::Continue
            );
            assert_eq!(position, 0.0);
        }

        #[test]
        fn watch_debounces_rapid_writes_and_triggers_once_after_settle() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let source = dir.path().join("document.md");
            fs::write(&source, "old")?;
            let mut state = WatchState::new(&source)?;
            let start = Instant::now();
            let first = WatchRevision {
                modified: Some(SystemTime::UNIX_EPOCH),
                len: 7,
            };
            let second = WatchRevision {
                modified: Some(SystemTime::UNIX_EPOCH),
                len: 8,
            };
            assert!(!state.observe(first, start));
            assert!(!state.observe(first, start + WATCH_SETTLE_DELAY / 2));
            assert!(!state.observe(second, start + WATCH_SETTLE_DELAY));
            assert!(!state.observe(
                second,
                start + WATCH_SETTLE_DELAY * 2 - Duration::from_millis(1)
            ));
            assert!(state.observe(second, start + WATCH_SETTLE_DELAY * 2));
            // Detection is not acceptance: acknowledge only after a
            // successful HTML rebuild, not before.
            assert_ne!(state.applied, second);
            assert!(!state.observe(second, start + WATCH_SETTLE_DELAY * 3));
            assert!(state.observe(second, start + WATCH_SETTLE_DELAY * 4));
            state.acknowledge_loaded(second);
            assert_eq!(state.applied, second);
            assert!(!state.observe(second, start + WATCH_SETTLE_DELAY * 5));
            Ok(())
        }

        #[test]
        fn failed_watch_rebuild_keeps_revision_pending_until_success() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let source = dir.path().join("watched.md");
            fs::write(&source, "old")?;
            let mut state = WatchState::new(&source)?;
            let old = state.applied;
            let changed = WatchRevision {
                modified: Some(SystemTime::UNIX_EPOCH),
                len: old.len + 10,
            };
            let start = Instant::now();
            assert!(!state.observe(changed, start));
            assert!(state.observe(changed, start + WATCH_SETTLE_DELAY));
            // Simulated HTML read/render failure: no acknowledge_loaded.
            assert_eq!(state.applied, old);
            assert!(!state.observe(changed, start + WATCH_SETTLE_DELAY * 2));
            assert!(state.observe(changed, start + WATCH_SETTLE_DELAY * 3));
            // The next successful rebuild consumes the revision only once.
            state.acknowledge_loaded(changed);
            assert!(!state.observe(changed, start + WATCH_SETTLE_DELAY * 4));
            assert_eq!(state.applied, changed);
            Ok(())
        }

        #[test]
        fn watch_file_deletion_does_not_trigger_reload() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let source = dir.path().join("watched.md");
            fs::write(&source, "hello")?;
            let mut state = WatchState::new(&source)?;
            fs::remove_file(&source)?;
            assert!(!state.should_reload(Instant::now() + WATCH_POLL_INTERVAL));
            Ok(())
        }

        #[test]
        fn renderer_mode_captures_only_a_missing_settled_viewport() {
            assert!(!should_capture_settled(Duration::ZERO, false));
            assert!(!should_capture_settled(
                Duration::from_millis(159),
                false
            ));
            assert!(should_capture_settled(HYBRID_SETTLE_AFTER, false));
            assert!(should_capture_settled(Duration::from_secs(2), false));
            assert!(!should_capture_settled(Duration::from_secs(2), true));
        }

        #[test]
        fn idle_prefetch_does_not_evict_higher_priority_cached_tiles() {
            let position = 5000.0;
            let h = 1000;
            let total = 50_000.0;
            let mut cached = vec![
                cached_at(4500.0),
                cached_at(5000.0),
                cached_at(6000.0),
                cached_at(7000.0),
            ];
            for (index, tile) in cached.iter_mut().enumerate() {
                tile.image_id = FIRST_IMAGE_ID + index as u32;
            }
            // The initial off-grid tile covers the viewport and is pinned.
            // A naive admission loop repeatedly swaps 6000/7000 for 4000.
            let next = next_tile_origin(&cached, position, h, total, ScrollDirection::Forward);
            assert_eq!(next, Some(4000.0));
            assert!(!should_admit_tile(
                &cached,
                position,
                h,
                total,
                ScrollDirection::Forward,
                4000.0,
                Some(FIRST_IMAGE_ID),
            ));
            // Moving forward must still allow required tiles to replace
            // lower-priority cache entries.
            assert!(should_admit_tile(
                &cached,
                8000.0,
                h,
                total,
                ScrollDirection::Forward,
                8000.0,
                Some(FIRST_IMAGE_ID),
            ));
        }

        #[test]
        fn tile_timing_marks_include_skipped_snapshots_and_uploaded_tiles() {
            let mut stats = TileTimingStats::default();
            stats.observe_snapshot(420.0);
            let mut cache = cached_at(0.0);
            cache.last_snapshot_ms = 210.0;
            cache.last_capture_ms = 100.0;
            cache.last_encode_ms = 90.0;
            cache.last_write_ms = 20.0;
            cache.last_upload_ms = 95.0;
            stats.observe_uploaded(&cache);
            assert_eq!(stats.max_snapshot_ms, 420.0);
            assert_eq!(stats.max_upload_ms, 95.0);
            cache.last_snapshot_ms = 510.0;
            cache.last_upload_ms = 40.0;
            stats.observe_uploaded(&cache);
            assert_eq!(stats.max_snapshot_ms, 510.0);
            assert_eq!(stats.max_upload_ms, 95.0);
            stats.observe_stages(200.0, 180.0, 10.0);
            assert_eq!(stats.max_capture_ms, 200.0);
            assert_eq!(stats.max_encode_ms, 180.0);
            assert_eq!(stats.max_write_ms, 20.0);
        }

        #[test]
        fn obsolete_snapshots_skip_only_after_large_position_changes() {
            // Existing lookahead is useful even if its tile does not yet
            // cover the viewport at the moment it was requested.
            assert!(!snapshot_is_obsolete(0.0, 0.0, 1000.0, 1000));
            assert!(!snapshot_is_obsolete(0.0, 1000.0, 1000.0, 1000));
            assert!(!snapshot_is_obsolete(0.0, 2000.0, 1000.0, 1000));
            // A long forward or backward gesture makes an in-flight capture
            // irrelevant before its potentially costly PNG upload.
            assert!(snapshot_is_obsolete(0.0, 5000.0, 1000.0, 1000));
            assert!(snapshot_is_obsolete(3000.0, 0.0, 3000.0, 1000));
            // Moving rapidly within an overlapping snapshot stays valid.
            assert!(!snapshot_is_obsolete(0.0, 1750.0, 1000.0, 1000));
        }

        #[test]
        fn hybrid_preview_does_not_flicker_at_cache_boundaries() {
            assert_eq!(
                hybrid_display_mode(
                    true,
                    true,
                    Duration::from_millis(300),
                    Duration::from_millis(80),
                ),
                HybridDisplayMode::Preview
            );
            assert_eq!(
                hybrid_display_mode(true, true, Duration::from_millis(300), HYBRID_SETTLE_AFTER,),
                HybridDisplayMode::Image
            );
            assert_eq!(
                hybrid_display_mode(false, false, Duration::from_millis(50), Duration::ZERO,),
                HybridDisplayMode::Hold
            );
            assert_eq!(
                hybrid_display_mode(false, false, HYBRID_PREVIEW_AFTER, Duration::ZERO,),
                HybridDisplayMode::Preview
            );
        }

        #[test]
        fn idle_cache_miss_prioritizes_exact_position() {
            let cached = vec![cached_at(0.0), cached_at(1000.0)];
            assert_eq!(
                recovery_tile_origin(&cached, 5100.0, 1000, 10_000.0),
                Some(4600.0),
            );
            let mut centered = cached_at(4600.0);
            centered.image_id = SECOND_IMAGE_ID;
            let mut extended = cached;
            extended.push(centered);
            assert_eq!(
                recovery_tile_origin(&extended, 5100.0, 1000, 10_000.0),
                None,
            );
            assert_eq!(
                recovery_tile_origin(&[cached_at(0.0)], 9000.0, 1000, 10_000.0),
                Some(8000.0),
            );
        }

        #[test]
        fn hybrid_image_scene_parses_styles() {
            let response = json!({
                "kind": "outline",
                "background": "#0d1117",
                "items": [{"y": 20, "x": 12, "width": 400, "height": 48,
                    "fontSize": 28, "lineHeight": 32, "weight": 700,
                    "color": "#f0f6fc", "tag": "h1", "text": "Test"}]
            });
            let scene = parse_preview_scene(&response)
                .unwrap_or_else(|error| panic!("parse scene: {error}"));
            assert_eq!(scene.background, "#0d1117");
            assert_eq!(scene.items.len(), 1);
            assert_eq!(scene.items[0].font_size, 28.0);
            assert_eq!(scene.items[0].text, "Test");
        }

        #[test]
        fn hybrid_preview_matches_dom_positions() {
            let items = vec![
                OutlineItem {
                    y: 100.0,
                    text: "before".into(),
                },
                OutlineItem {
                    y: 1000.0,
                    text: "# Section".into(),
                },
                OutlineItem {
                    y: 1040.0,
                    text: "paragraph".into(),
                },
                OutlineItem {
                    y: 1180.0,
                    text: "end".into(),
                },
            ];
            assert_eq!(
                preview_lines(&items, 1000.0, 200, 10),
                vec![
                    (0, "# Section".into()),
                    (2, "paragraph".into()),
                    (9, "end".into()),
                ],
            );
            assert!(preview_lines(&items, 1400.0, 200, 10).is_empty());
        }

        #[test]
        fn hybrid_outline_rejects_invalid_entries_and_sorts() {
            let payload = json!({
                "kind": "outline",
                "items": [
                    {"y": 500, "text": "later"},
                    {"y": -1, "text": "invalid"},
                    {"y": 100, "text": "earlier"},
                    {"y": 140, "text": ""}
                ]
            });
            let outline =
                parse_outline(&payload).unwrap_or_else(|error| panic!("outline parse: {error}"));
            assert_eq!(outline.len(), 2);
            assert_eq!(outline[0].text, "earlier");
            assert_eq!(outline[1].text, "later");
        }

        #[test]
        fn tile_placement_hide_keeps_image_data_cached() {
            let mut encoded = Vec::new();
            hide_image_placement(&mut encoded, FIRST_IMAGE_ID)
                .unwrap_or_else(|error| panic!("placement hide failed: {error}"));
            assert_eq!(
                encoded,
                format!("\x1b_Ga=d,d=i,i={FIRST_IMAGE_ID},p=1,q=2;\x1b\\").into_bytes()
            );
        }

        #[test]
        fn tiled_prefetch_stays_bounded_and_prioritizes_direction() {
            let first = cached_at(0.0);
            assert_eq!(
                next_tile_origin(
                    &[first.clone()],
                    0.0,
                    1000,
                    10_000.0,
                    ScrollDirection::Forward
                ),
                Some(1000.0)
            );
            let next = cached_at(1000.0);
            assert_eq!(
                next_tile_origin(
                    &[first.clone(), next.clone()],
                    0.0,
                    1000,
                    10_000.0,
                    ScrollDirection::Forward
                ),
                Some(2000.0)
            );
            assert_eq!(
                next_tile_origin(&[next], 1000.0, 1000, 10_000.0, ScrollDirection::Backward),
                Some(0.0)
            );
        }

        #[test]
        fn tile_eviction_preserves_visible_and_displayed_images() {
            let mut cached = vec![
                cached_at(0.0),
                cached_at(1000.0),
                cached_at(2000.0),
                cached_at(3000.0),
            ];
            for (index, tile) in cached.iter_mut().enumerate() {
                tile.image_id = FIRST_IMAGE_ID + index as u32;
            }
            let removed = evict_tile(&mut cached, 1500.0, 1000, Some(FIRST_IMAGE_ID))
                .unwrap_or_else(|| panic!("should find an unused tile"));
            assert_eq!(removed.source_y, 3000.0);
            assert_eq!(cached.len(), 3);
            assert!(cached.iter().any(|tile| tile.source_y == 0.0));
            assert!(cached.iter().any(|tile| tile.source_y == 1000.0));
        }

        #[test]
        fn tile_selection_respects_image_coverage() {
            let cached = vec![cached_at(0.0), cached_at(1000.0)];
            assert_eq!(
                select_tile(&cached, 1200.0, 1000).map(|tile| tile.source_y),
                Some(1000.0)
            );
            assert!(select_tile(&cached, 2500.0, 1000).is_none());
            assert_eq!(
                next_tile_origin(
                    &[cached_at(0.0)],
                    0.0,
                    1000,
                    2000.0,
                    ScrollDirection::Forward
                ),
                None
            );
        }

        #[test]
        fn tile_cache_remains_bounded_during_long_bidirectional_scrolls() {
            use std::collections::BTreeSet;

            let viewport = 1000;
            let document_height = 50_000.0;
            let mut first = cached_at(0.0);
            first.document_height = document_height;
            let mut cached = vec![first];
            let mut last_displayed_id = Some(FIRST_IMAGE_ID);

            // Sweep the full document, reverse direction, and repeat. Model
            // instant snapshot completion to stress admission and eviction
            // independently of WebKit/terminal timing.
            for _ in 0..3 {
                for (position, direction) in (0..=196)
                    .map(|step| (f64::from(step * 250), ScrollDirection::Forward))
                    .chain(
                        (0..=196)
                            .rev()
                            .map(|step| (f64::from(step * 250), ScrollDirection::Backward)),
                    )
                {
                    if let Some(tile) = select_tile(&cached, position, viewport) {
                        last_displayed_id = Some(tile.image_id);
                    }

                    if let Some(origin) =
                        next_tile_origin(&cached, position, viewport, document_height, direction)
                    {
                        if cached.len() == MAX_TILE_CACHE {
                            if let Some(evicted) =
                                evict_tile(&mut cached, position, viewport, last_displayed_id)
                            {
                                assert_ne!(Some(evicted.image_id), last_displayed_id);
                                assert!(!evicted.covers(position, viewport));
                            }
                        }

                        if cached.len() < MAX_TILE_CACHE {
                            let image_id = tile_image_id(&cached)
                                .unwrap_or_else(|| panic!("no free tile image id"));
                            let mut tile = cached_at(origin);
                            tile.image_id = image_id;
                            tile.document_height = document_height;
                            cached.push(tile);
                        }
                    }

                    assert!(cached.len() <= MAX_TILE_CACHE);
                    assert!(cached.iter().all(|tile| {
                        (FIRST_IMAGE_ID..=LAST_TILE_IMAGE_ID).contains(&tile.image_id)
                    }));
                    let ids: BTreeSet<_> = cached.iter().map(|tile| tile.image_id).collect();
                    assert_eq!(ids.len(), cached.len(), "image ids must not collide");
                    let origins: BTreeSet<_> =
                        cached.iter().map(|tile| tile.source_y.to_bits()).collect();
                    assert_eq!(origins.len(), cached.len(), "duplicate tile snapshots");
                }
            }
        }

        #[test]
        fn tile_eviction_skips_all_current_and_displayed_images() {
            let mut cached = vec![cached_at(0.0), cached_at(0.0)];
            cached[1].image_id = SECOND_IMAGE_ID;
            assert!(evict_tile(&mut cached, 500.0, 1000, Some(FIRST_IMAGE_ID)).is_none());
            assert_eq!(cached.len(), 2);
        }

        #[test]
        fn final_document_tile_does_not_queue_duplicate_snapshots() {
            let viewport = 1000;
            let mut last = cached_at(8000.0);
            last.document_height = 10_000.0;
            assert_eq!(
                next_tile_origin(
                    &[last.clone()],
                    9000.0,
                    viewport,
                    last.document_height,
                    ScrollDirection::Forward,
                ),
                None
            );
            assert_eq!(
                select_tile(&[last], 9000.0, viewport).map(|tile| tile.source_y),
                Some(8000.0)
            );
        }

        #[test]
        fn cache_wait_counts_only_distinct_misses_and_records_duration() {
            let mut stats = CacheWaitStats::default();
            let start = Instant::now();
            stats.observe(true, start);
            stats.observe(false, start + Duration::from_millis(10));
            stats.observe(false, start + Duration::from_millis(80));
            assert_eq!(stats.episodes, 1);
            assert_eq!(
                stats.elapsed(start + Duration::from_millis(90)),
                Duration::from_millis(80)
            );
            stats.observe(true, start + Duration::from_millis(110));
            assert_eq!(stats.longest, Duration::from_millis(100));

            stats.observe(false, start + Duration::from_millis(130));
            assert_eq!(stats.episodes, 2);
            assert_eq!(
                stats.worst(start + Duration::from_millis(250)),
                Duration::from_millis(120)
            );
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
        fn limits_displacement_per_input_batch() {
            let mut position = 0.0;
            let mut direction = ScrollDirection::Forward;
            for _ in 0..MAX_INPUT_BATCH {
                assert_eq!(
                    apply_key(
                        key(KeyCode::Down),
                        &mut position,
                        &mut direction,
                        10_000.0,
                        1000
                    ),
                    InputAction::Continue
                );
            }
            assert_eq!(position, 64.0);
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

            let _ = apply_key(
                key(KeyCode::PageDown),
                &mut position,
                &mut direction,
                1100.0,
                1000,
            );
            assert_eq!(position, 100.0);
            assert_eq!(direction, ScrollDirection::Forward);
        }

        #[test]
        fn g_and_capital_g_jump_to_document_edges_without_changing_scroll_contract() {
            let mut position = 320.0;
            let mut direction = ScrollDirection::Forward;
            assert_eq!(
                apply_key(
                    key(KeyCode::Char('G')),
                    &mut position,
                    &mut direction,
                    10_000.0,
                    1000
                ),
                InputAction::Continue
            );
            assert_eq!(position, 9000.0);
            assert_eq!(direction, ScrollDirection::Forward);

            assert_eq!(
                apply_key(
                    key(KeyCode::Char('g')),
                    &mut position,
                    &mut direction,
                    10_000.0,
                    1000
                ),
                InputAction::Continue
            );
            assert_eq!(position, 0.0);
            assert_eq!(direction, ScrollDirection::Backward);

            // A jump followed by a normal scroll in the same input batch
            // must retain navigation order and existing 16 px step size.
            let _ = apply_key(
                key(KeyCode::Char('G')),
                &mut position,
                &mut direction,
                10_000.0,
                1000,
            );
            let _ = apply_key(
                key(KeyCode::Char('k')),
                &mut position,
                &mut direction,
                10_000.0,
                1000,
            );
            assert_eq!(position, 8984.0);
            assert_eq!(direction, ScrollDirection::Backward);
        }

        #[test]
        fn g_and_capital_g_ignore_repeat_and_clamp_short_documents() {
            let mut position = 0.0;
            let mut direction = ScrollDirection::Forward;
            let mut repeat = key(KeyCode::Char('G'));
            repeat.kind = KeyEventKind::Repeat;
            assert_eq!(
                apply_key(repeat, &mut position, &mut direction, 10_000.0, 1000),
                InputAction::Continue
            );
            assert_eq!(position, 0.0);

            let mut released = key(KeyCode::Char('g'));
            released.kind = KeyEventKind::Release;
            assert_eq!(
                apply_key(released, &mut position, &mut direction, 10_000.0, 1000),
                InputAction::Continue
            );
            assert_eq!(position, 0.0);
            assert_eq!(
                apply_key(
                    key(KeyCode::Char('G')),
                    &mut position,
                    &mut direction,
                    400.0,
                    1000
                ),
                InputAction::Continue
            );
            assert_eq!(position, 0.0);
        }

        #[test]
        fn manual_reload_requires_a_press_and_preserves_scroll_position() {
            let mut position = 630.0;
            let mut direction = ScrollDirection::Forward;
            let pressed = key(KeyCode::Char('r'));
            assert_eq!(
                apply_key(pressed, &mut position, &mut direction, 10_000.0, 1000),
                InputAction::Reload
            );
            assert_eq!(position, 630.0);
            assert_eq!(direction, ScrollDirection::Forward);
            assert_eq!(reading_fraction(position, 10_000.0, 1000), 0.07);

            let mut repeated = pressed;
            repeated.kind = KeyEventKind::Repeat;
            assert_eq!(
                apply_key(repeated, &mut position, &mut direction, 10_000.0, 1000),
                InputAction::Continue
            );
            let mut released = pressed;
            released.kind = KeyEventKind::Release;
            assert_eq!(
                apply_key(released, &mut position, &mut direction, 10_000.0, 1000),
                InputAction::Continue
            );
            assert_eq!(position, 630.0);
        }

        #[test]
        fn manual_reload_acknowledges_revision_without_masking_later_writes() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let source = dir.path().join("document.md");
            fs::write(&source, "old")?;
            let mut state = WatchState::new(&source)?;
            let start = Instant::now();
            let loaded = WatchRevision {
                modified: Some(SystemTime::UNIX_EPOCH),
                len: 10,
            };
            state.acknowledge_loaded(loaded);
            assert!(!state.observe(loaded, start));
            let later = WatchRevision {
                modified: Some(SystemTime::UNIX_EPOCH),
                len: 12,
            };
            assert!(!state.observe(later, start + WATCH_SETTLE_DELAY));
            assert!(state.observe(later, start + WATCH_SETTLE_DELAY * 2));
            Ok(())
        }

        #[test]
        fn preserves_control_keys_and_ignores_key_releases() {
            let mut position = 0.0;
            let mut direction = ScrollDirection::Forward;
            assert_eq!(
                apply_key(
                    key(KeyCode::Char('y')),
                    &mut position,
                    &mut direction,
                    2000.0,
                    1000
                ),
                InputAction::Select
            );
            assert_eq!(
                apply_key(
                    key(KeyCode::Char('q')),
                    &mut position,
                    &mut direction,
                    2000.0,
                    1000
                ),
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
                last_capture_ms: 0.0,
                last_encode_ms: 0.0,
                last_write_ms: 0.0,
                last_upload_ms: 0.0,
                png_bytes: 100,
            }
        }

        #[test]
        fn prefetches_early_in_the_scroll_direction() {
            let first = cached_at(0.0);
            // Do not spend cache memory on a document that has not moved yet.
            assert_eq!(
                prefetch_origin(&first, 0.0, 1000, ScrollDirection::Forward),
                None
            );
            // One key scroll is enough to schedule the next page.
            assert_eq!(
                prefetch_origin(&first, 16.0, 1000, ScrollDirection::Forward),
                Some(1000.0)
            );
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
            // Reversing near the far edge of an existing cache starts a
            // backward prefetch before the viewport leaves that cache.
            assert_eq!(
                prefetch_origin(&cached_at(1000.0), 1950.0, 1000, ScrollDirection::Backward),
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
