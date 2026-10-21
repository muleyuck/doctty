//! All Cocoa/WebKit calls are isolated here. The view is never sent to
//! another thread: AppKit objects are !Send and belong to the main thread.
use std::{
    cell::RefCell,
    env,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use crate::protocol;
use anyhow::{Context, Result, bail};
use block2::RcBlock;
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained, runtime::AnyObject};
use objc2_app_kit::NSApplication;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSDate, NSError, NSRunLoop, NSString, NSURL};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

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
    if args.next().is_some() {
        bail!("too many arguments; expected HTML READ_ROOT");
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
            width: 800.0,
            height: 1200.0,
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
                NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
                continue;
            }
            Err(error) => return Err(error),
        };
        let height = status["height"]
            .as_f64()
            .context("invalid document height")?;
        let fonts_ready = status["fontsReady"].as_bool().unwrap_or(true);
        let images_ready = status["imagesReady"].as_bool().unwrap_or(true);
        if height.is_finite() && height > 0.0 && ((fonts_ready && images_ready) || attempt >= 40) {
            println!(
                "{}",
                serde_json::to_string(&protocol::Response::Ready { height })?
            );
            return Ok(());
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
  fontsReady: !document.fonts || document.fonts.status === "loaded",
  imagesReady: Array.from(document.images).every(img => img.complete)
}))())"#;

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
