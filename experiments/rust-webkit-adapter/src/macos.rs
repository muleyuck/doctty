//! All Cocoa/WebKit calls are isolated here. The view is never sent to
//! another thread: AppKit objects are !Send and belong to the main thread.
use std::{
    cell::RefCell,
    env,
    io::{self, BufRead},
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::protocol;
use anyhow::{Context, Result, bail};
use block2::RcBlock;
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSApplication, NSBitmapImageFileType, NSBitmapImageRep, NSImage};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSDate, NSDictionary, NSError, NSRunLoop, NSString, NSURL};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView, WKWebViewConfiguration};

pub(crate) fn run() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let html = PathBuf::from(
        args.next()
            .context("usage: doctty-rust-webkit-adapter HTML READ_ROOT")?,
    );
    let root = PathBuf::from(
        args.next()
            .context("usage: doctty-rust-webkit-adapter HTML READ_ROOT")?,
    );
    let output = args.next().map(PathBuf::from);
    let width = args
        .next()
        .map(|v| v.to_string_lossy().parse::<f64>())
        .transpose()
        .context("invalid viewport width")?;
    let height = args
        .next()
        .map(|v| v.to_string_lossy().parse::<f64>())
        .transpose()
        .context("invalid viewport height")?;
    if args.next().is_some()
        || output.is_some() != width.is_some()
        || output.is_some() != height.is_some()
    {
        bail!("usage: doctty-rust-webkit-adapter HTML READ_ROOT [OUTPUT_PNG WIDTH HEIGHT]");
    }
    let viewport_width = width.unwrap_or(800.0);
    let viewport_height = height.unwrap_or(600.0);
    if !viewport_width.is_finite()
        || !viewport_height.is_finite()
        || viewport_width < 1.0
        || viewport_height < 1.0
        || viewport_width > 16384.0
        || viewport_height > 16384.0
    {
        bail!("viewport dimensions must be between 1 and 16384");
    }
    let html = html.canonicalize().context("cannot find HTML file")?;
    let root = root
        .canonicalize()
        .context("cannot find read access directory")?;
    if !html.is_file() || !root.is_dir() || !html.starts_with(&root) {
        bail!("HTML must be a file inside the read-access root");
    }

    // Safety: this executable's entrypoint is the macOS process main thread.
    // The NSApplication and WKWebView objects stay on it for their lifetime.
    let mtm = MainThreadMarker::new().context("WebKit adapter must run on main thread")?;
    let _app = NSApplication::sharedApplication(mtm);
    let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: viewport_width,
            height: viewport_height * 2.0,
        },
    };
    let view: Retained<WKWebView> = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &configuration)
    };

    let url =
        NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&html.to_string_lossy()), false);
    let access =
        NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&root.to_string_lossy()), true);
    // Safety: URLs are local filesystem URLs, view stays on main thread,
    // and the access root encloses the HTML file.
    unsafe { view.loadFileURL_allowingReadAccessToURL(&url, &access) };

    // Navigation may finish before the first isLoading() observation, so
    // do not use an ever-loading flag as the completion criterion. Instead,
    // ask the loaded document for the same layout/fonts/image state as Swift.
    let ready_by = Instant::now() + Duration::from_secs(15);
    let mut attempt = 0;
    loop {
        if Instant::now() >= ready_by {
            bail!("WebKit document was not ready within 15s");
        }
        let status = match evaluate_json(&view, READY_SCRIPT, Duration::from_secs(3)) {
            Ok(status) => status,
            Err(_) if Instant::now() < ready_by => {
                // The local navigation may not have committed yet. Unlike
                // isLoading(), this retry does not miss a fast load transition.
                NSRunLoop::currentRunLoop()
                    .runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
                continue;
            }
            Err(error) => return Err(error),
        };
        // A fresh WKWebView may evaluate JS against about:blank before
        // loadFileURL has committed. Do not treat its viewport height as
        // proof that the requested local HTML document has loaded.
        if let Some(height) = ready_document_height(&status, attempt) {
            emit(&protocol::Response::Ready { height })?;
            return serve_commands(
                &view,
                height,
                output.as_deref(),
                viewport_width,
                viewport_height,
            );
        }
        attempt += 1;
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
    }
}

const READY_SCRIPT: &str = r#"JSON.stringify((() => ({
  height: Math.max(
    document.documentElement.scrollHeight,
    document.body ? document.body.scrollHeight : 0,
    document.documentElement.offsetHeight,
    window.innerHeight
  ),
  // about:blank is immediately scriptable but is not the requested file.
  fileLoaded: location.protocol === "file:" && document.readyState === "complete",
  fontsReady: !document.fonts || document.fonts.status === "loaded",
  imagesReady: Array.from(document.images).every(img => img.complete)
}))())"#;

fn ready_document_height(status: &serde_json::Value, attempt: usize) -> Option<f64> {
    let height = status["height"].as_f64()?;
    let file_loaded = status["fileLoaded"].as_bool().unwrap_or(false);
    let fonts_ready = status["fontsReady"].as_bool().unwrap_or(false);
    let images_ready = status["imagesReady"].as_bool().unwrap_or(false);
    (height.is_finite()
        && height > 0.0
        && file_loaded
        && ((fonts_ready && images_ready) || attempt >= 40))
        .then_some(height)
}

#[cfg(test)]
mod readiness_tests {
    use super::ready_document_height;
    use serde_json::json;

    #[test]
    fn blank_page_must_not_emit_ready_even_with_positive_height() {
        let about_blank = json!({
            "height": 800.0,
            "fileLoaded": false,
            "fontsReady": true,
            "imagesReady": true
        });
        assert_eq!(ready_document_height(&about_blank, 0), None);
        assert_eq!(ready_document_height(&about_blank, 100), None);
    }

    #[test]
    fn file_must_be_loaded_and_resources_settled() {
        let loading = json!({
            "height": 3500.0,
            "fileLoaded": true,
            "fontsReady": false,
            "imagesReady": true
        });
        assert_eq!(ready_document_height(&loading, 0), None);
        assert_eq!(ready_document_height(&loading, 40), Some(3500.0));
        let ready = json!({
            "height": 3500.0,
            "fileLoaded": true,
            "fontsReady": true,
            "imagesReady": true
        });
        assert_eq!(ready_document_height(&ready, 0), Some(3500.0));
        assert_eq!(ready_document_height(&json!({"height": 0.0}), 40), None);
    }
}

// WebKit promises to call the completion handler on the main thread.
// The run loop is pumped on that thread; no Objective-C object moves to
// a Rust worker. Evaluate a JSON *string* to keep the FFI type explicit.
fn evaluate_json(
    webview: &WKWebView,
    javascript: &str,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let result: Rc<RefCell<Option<Result<String>>>> = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&result);
    let callback = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        let finished = if !error.is_null() {
            Err(anyhow::anyhow!("WKWebView JavaScript evaluation failed"))
        } else if value.is_null() {
            Err(anyhow::anyhow!("WKWebView JavaScript produced no result"))
        } else {
            // Safety: READY_SCRIPT always returns a JSON.stringify string;
            // WebKit bridges JavaScript strings to Foundation NSString.
            // The callback's object remains alive throughout this call.
            let text = unsafe { &*value.cast::<NSString>() };
            Ok(text.to_string())
        };
        *slot.borrow_mut() = Some(finished);
    });
    // Safety: the block has an owned heap lifetime and matches the
    // WKWebView API signature. The view and run loop stay on the main thread.
    unsafe {
        webview
            .evaluateJavaScript_completionHandler(&NSString::from_str(javascript), Some(&callback));
    }
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(finished) = result.borrow_mut().take() {
            return Ok(serde_json::from_str(&finished?)?);
        }
        if Instant::now() >= deadline {
            bail!("JavaScript callback timed out after {timeout:?}");
        }
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
}

fn emit(response: &protocol::Response) -> Result<()> {
    println!("{}", serde_json::to_string(response)?);
    Ok(())
}

// Read stdin off the AppKit main thread. WKWebView and all JavaScript
// evaluation remain on the main thread and must keep pumping NSRunLoop.
fn serve_commands(
    view: &WKWebView,
    document_height: f64,
    output: Option<&Path>,
    width: f64,
    height: f64,
) -> Result<()> {
    let started = Instant::now();
    let (tx, rx) = mpsc::channel::<String>();
    let mut snapshot_requests = 0_u64;
    let mut snapshot_completed = 0_u64;
    let mut last_snapshot_at: Option<Instant> = None;
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });

    loop {
        match rx.try_recv() {
            Ok(line) => {
                let command = match serde_json::from_str::<protocol::Command>(&line) {
                    Ok(command) => command,
                    Err(error) => {
                        emit(&protocol::Response::Error {
                            message: format!("invalid command: {error}"),
                        })?;
                        continue;
                    }
                };
                match command {
                    protocol::Command::Dimensions => {
                        emit(&protocol::Response::Dimensions {
                            height: document_height,
                        })?;
                    }
                    protocol::Command::Metrics => {
                        emit(&protocol::Response::Metrics {
                            requests: snapshot_requests,
                            completed: snapshot_completed,
                            uptime_sec: started.elapsed().as_secs_f64(),
                            seconds_since_snapshot: last_snapshot_at
                                .map_or(-1.0, |at| at.elapsed().as_secs_f64()),
                        })?;
                    }
                    protocol::Command::Quit => return Ok(()),
                    protocol::Command::Snapshot { y } => {
                        snapshot_requests += 1;
                        last_snapshot_at = Some(Instant::now());
                        let result = output
                            .context("snapshot requires OUTPUT_PNG WIDTH HEIGHT")
                            .and_then(|path| {
                                capture_snapshot(view, path, y, document_height, width, height)
                            });
                        match result {
                            Ok(response) => {
                                snapshot_completed += 1;
                                emit(&response)?;
                            }
                            Err(error) => emit(&protocol::Response::Error {
                                message: error.to_string(),
                            })?,
                        }
                    }
                    protocol::Command::Resize { .. }
                    | protocol::Command::Outline
                    | protocol::Command::Select => {
                        emit(&protocol::Response::Error {
                            message: "command not yet implemented in Rust adapter".into(),
                        })?;
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            Err(mpsc::TryRecvError::Empty) => {
                NSRunLoop::currentRunLoop()
                    .runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
            }
        }
    }
}

// A viewport is two terminal heights tall, matching the Swift helper.
fn capture_snapshot(
    view: &WKWebView,
    output: &Path,
    y: f64,
    document_height: f64,
    width: f64,
    height: f64,
) -> Result<protocol::Response> {
    if !y.is_finite() {
        bail!("snapshot y must be finite");
    }
    let requested_y = y.max(0.0).min((document_height - height * 2.0).max(0.0));
    let started = Instant::now();
    let script = format!(
        "JSON.stringify((() => {{ window.scrollTo(0, {requested_y}); return window.scrollY; }})())"
    );
    let actual_y = evaluate_json(view, &script, Duration::from_secs(5))?
        .as_f64()
        .context("invalid scroll position")?;

    let mtm = MainThreadMarker::new().context("snapshot requires main thread")?;
    let config = unsafe { WKSnapshotConfiguration::new(mtm) };
    unsafe {
        config.setRect(CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width,
                height: height * 2.0,
            },
        });
        config.setAfterScreenUpdates(true);
    }

    let capture_started = Instant::now();
    let result: Rc<RefCell<Option<Result<Retained<NSImage>>>>> = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&result);
    let callback = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        let finished = if !error.is_null() {
            Err(anyhow::anyhow!("WebKit snapshot failed"))
        } else if image.is_null() {
            Err(anyhow::anyhow!("WebKit snapshot returned no image"))
        } else {
            // SAFETY: WKWebView supplies a live NSImage during the callback;
            // retain before leaving the callback.
            unsafe { Retained::retain(image) }.context("could not retain image")
        };
        *slot.borrow_mut() = Some(finished);
    });
    unsafe {
        view.takeSnapshotWithConfiguration_completionHandler(Some(&config), &callback);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let image = loop {
        if let Some(done) = result.borrow_mut().take() {
            break done?;
        }
        if Instant::now() >= deadline {
            bail!("WebKit snapshot timed out");
        }
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    };
    let capture_ms = capture_started.elapsed().as_secs_f64() * 1000.0;
    let encode_started = Instant::now();
    let tiff = image
        .TIFFRepresentation()
        .context("snapshot has no TIFF data")?;
    let bitmap =
        NSBitmapImageRep::imageRepWithData(&tiff).context("could not decode snapshot bitmap")?;
    let properties = NSDictionary::new();
    // Safety: bitmap and the empty immutable properties dictionary are valid
    // Objective-C objects retained for the duration of PNG conversion.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    }
    .context("could not encode viewport PNG")?;
    let bytes = png.to_vec();
    let png_bytes = bytes.len();
    let encode_ms = encode_started.elapsed().as_secs_f64() * 1000.0;
    let write_started = Instant::now();
    std::fs::write(output, &bytes).context("could not write viewport PNG")?;
    let write_ms = write_started.elapsed().as_secs_f64() * 1000.0;
    Ok(protocol::Response::Snapshot {
        requested_y,
        actual_y,
        height: document_height,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        capture_ms,
        encode_ms,
        write_ms,
        png_bytes,
    })
}
