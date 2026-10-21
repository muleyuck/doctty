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
    mut document_height: f64,
    output: Option<&Path>,
    mut width: f64,
    mut height: f64,
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
                    protocol::Command::Outline => {
                        let result = evaluate_json(view, OUTLINE_SCRIPT, Duration::from_secs(10))
                            .and_then(outline_response);
                        match result {
                            Ok(response) => emit(&response)?,
                            Err(error) => emit(&protocol::Response::Error {
                                message: error.to_string(),
                            })?,
                        }
                    }
                    protocol::Command::Resize {
                        width: next_width,
                        height: next_height,
                    } => match resize_viewport(view, next_width, next_height) {
                        Ok(next_document_height) => {
                            width = f64::from(next_width);
                            height = f64::from(next_height);
                            document_height = next_document_height;
                            emit(&protocol::Response::Resized {
                                height: document_height,
                            })?;
                        }
                        Err(error) => emit(&protocol::Response::Error {
                            message: error.to_string(),
                        })?,
                    },
                    protocol::Command::Select => {
                        let result = evaluate_json(view, SELECT_SCRIPT, Duration::from_secs(5))
                            .and_then(selection_response);
                        match result {
                            Ok(response) => emit(&response)?,
                            Err(error) => emit(&protocol::Response::Error {
                                message: error.to_string(),
                            })?,
                        }
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

// Resize the same WKWebView instead of replacing its WebKit process.
// The caller commits new geometry only after DOM metrics succeed.
fn resize_viewport(view: &WKWebView, width: u32, height: u32) -> Result<f64> {
    let (width, height) = validate_resize_dimensions(width, height)?;
    let previous_frame = view.frame();
    view.setFrame(CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width,
            height: height * 2.0,
        },
    });
    view.setNeedsLayout(true);
    view.layoutSubtreeIfNeeded();

    // Allow WebKit to apply the resized viewport before reading new DOM metrics.
    NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    let document_height = evaluate_json(view, HEIGHT_SCRIPT, Duration::from_secs(5))
        .and_then(|value| value.as_f64().context("invalid resized document height"))
        .and_then(|value| {
            if value.is_finite() && value > 0.0 {
                Ok(value)
            } else {
                bail!("resized document height must be positive and finite");
            }
        });
    if document_height.is_err() {
        // Keep the previous dimensions usable after a failed resize response.
        view.setFrame(previous_frame);
        view.setNeedsLayout(true);
        view.layoutSubtreeIfNeeded();
    }
    document_height
}

const HEIGHT_SCRIPT: &str = r#"JSON.stringify(Math.max(
  document.documentElement.scrollHeight,
  document.body ? document.body.scrollHeight : 0,
  document.documentElement.offsetHeight,
  window.innerHeight
))"#;

fn validate_resize_dimensions(width: u32, height: u32) -> Result<(f64, f64)> {
    if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        bail!("viewport dimensions must be between 1 and 16384");
    }
    Ok((f64::from(width), f64::from(height)))
}

// Transform the JavaScript response to the same JSON-line shape as Swift.
// The preview still runs in the terminal, not in this WebKit subprocess.
fn outline_response(value: serde_json::Value) -> Result<protocol::Response> {
    let background = value["background"]
        .as_str()
        .context("outline is missing a background color")?
        .to_owned();
    let items = value["items"]
        .as_array()
        .context("outline is missing its items array")?
        .clone();
    if items.len() > 12_000 {
        bail!("WebKit returned too many outline items");
    }
    Ok(protocol::Response::Outline { background, items })
}

// Match Swift's intentional DOM selection / OSC52 clipboard bridge.
// This is not pointer-driven mouse selection. The JS returns a JSON string,
// just like the existing evaluate_json contract requires.
const SELECT_SCRIPT: &str = r#"JSON.stringify((() => {
  const node = document.querySelector("article p, p, h1, body");
  if (!node) return "";
  const range = document.createRange();
  range.selectNodeContents(node);
  const selection = window.getSelection();
  selection.removeAllRanges();
  selection.addRange(range);
  return selection.toString().slice(0, 4096);
})())"#;

fn selection_response(value: serde_json::Value) -> Result<protocol::Response> {
    let text = value
        .as_str()
        .context("DOM selection must be a JSON string")?
        .to_owned();
    Ok(protocol::Response::Selection { text })
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

// Keep the DOM block selection, text prefixes and style fields aligned with
// the working Swift hybrid preview until visual equivalence is demonstrated.
const OUTLINE_SCRIPT: &str = r#"JSON.stringify(
(() => {
  const root = document.querySelector('article, .markdown-body') || document.body;
  const elements = Array.from(
    root.querySelectorAll('h1,h2,h3,h4,h5,h6,p,li,pre,table,img')
  );
  const stride = Math.max(1, Math.ceil(elements.length / 12000));
  const items = [];
  for (let i = 0; i < elements.length; i += stride) {
    const el = elements[i];
    // Parent list/code/table blocks already contain child content.
    const parent = el.parentElement;
    if (parent && parent.closest('li,pre,table')) continue;
    const rect = el.getBoundingClientRect();
    if (rect.height < 1 || rect.width < 1) continue;
    const tag = el.tagName.toLowerCase();
    let text = tag === 'img'
      ? '[image] ' + (el.getAttribute('alt') || '')
      : (el.textContent || '');
    text = text.replace(/\s+/g, ' ').trim().slice(0, 160);
    if (!text) continue;
    if (/^h[1-6]$/.test(tag)) {
      text = '#'.repeat(Number(tag[1])) + ' ' + text;
    } else if (tag === 'li') {
      text = '• ' + text;
    } else if (tag === 'pre') {
      text = '[code] ' + text;
    } else if (tag === 'table') {
      text = '[table] ' + text;
    }
    const style = window.getComputedStyle(el);
    const normalizedColor = value => {
      if (value.startsWith('rgba(') &&
          Number(value.slice(value.lastIndexOf(',') + 1).replace(')', '').trim()) === 0) {
        return '';
      }
      const match = value.match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)/);
      return match ? '#' + [match[1], match[2], match[3]]
        .map(n => Number(n).toString(16).padStart(2, '0')).join('') : '';
    };
    items.push({
      y: Math.round(rect.top + window.scrollY),
      x: Math.round(rect.left + window.scrollX),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      text,
      tag,
      fontSize: parseFloat(style.fontSize) || 16,
      lineHeight: parseFloat(style.lineHeight) || 24,
      weight: parseInt(style.fontWeight, 10) || 400,
      color: normalizedColor(style.color),
      background: normalizedColor(style.backgroundColor)
    });
  }
  return {
    background: (() => {
      const background = window.getComputedStyle(document.body).backgroundColor;
      const match = background.match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)/);
      return match ? '#' + [match[1], match[2], match[3]]
        .map(n => Number(n).toString(16).padStart(2, '0')).join('') : '#ffffff';
    })(),
    items
  };
})()
)"#;

#[cfg(test)]
mod outline_tests {
    use super::outline_response;
    use serde_json::json;

    #[test]
    fn preserves_dom_coordinates_and_style_keys() {
        let response = outline_response(json!({
            "background": "#0d1117",
            "items": [{
                "y": 40, "x": 12, "width": 480, "height": 50,
                "text": "# Heading", "tag": "h1", "fontSize": 28,
                "lineHeight": 32, "weight": 700, "color": "#f0f6fc",
                "background": ""
            }]
        }))
        .unwrap();
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["kind"], "outline");
        assert_eq!(json["background"], "#0d1117");
        assert_eq!(json["items"][0]["y"], 40);
        assert_eq!(json["items"][0]["fontSize"], 28);
        assert_eq!(json["items"][0]["text"], "# Heading");
    }

    #[test]
    fn rejects_incomplete_outline_payloads() {
        assert!(outline_response(json!({"background": "#fff"})).is_err());
        assert!(outline_response(json!({"items": []})).is_err());
    }
}

#[cfg(test)]
mod resize_tests {
    use super::validate_resize_dimensions;

    #[test]
    fn validates_dimensions_before_touching_webkit() {
        assert_eq!(
            validate_resize_dimensions(480, 320).unwrap(),
            (480.0, 320.0)
        );
        assert!(validate_resize_dimensions(0, 320).is_err());
        assert!(validate_resize_dimensions(480, 0).is_err());
        assert!(validate_resize_dimensions(16_385, 320).is_err());
        assert!(validate_resize_dimensions(480, 16_385).is_err());
    }
}

#[cfg(test)]
mod selection_tests {
    use super::selection_response;
    use serde_json::json;

    #[test]
    fn encodes_swift_selection_response() {
        let response = selection_response(json!("Rust WebKit snapshot smoke")).unwrap();
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["kind"], "selection");
        assert_eq!(json["text"], "Rust WebKit snapshot smoke");
    }

    #[test]
    fn rejects_wrong_selection_payload() {
        assert!(selection_response(json!({"text": "invalid"})).is_err());
        assert!(selection_response(serde_json::Value::Null).is_err());
    }
}
