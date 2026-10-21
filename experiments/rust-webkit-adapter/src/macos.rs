//! All Cocoa/WebKit calls are isolated here. The view is never sent to
//! another thread: AppKit objects are !Send and belong to the main thread.
use std::{env, path::PathBuf, time::{Duration, Instant}};

use anyhow::{Context, Result, bail};
use objc2::rc::Retained;
use objc2_app_kit::NSApplication;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSDate, NSRunLoop, NSString, NSURL};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

pub(crate) fn run() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let html = PathBuf::from(args.next().context("usage: doctty-rust-webkit-adapter HTML READ_ROOT")?);
    let root = PathBuf::from(args.next().context("usage: doctty-rust-webkit-adapter HTML READ_ROOT")?);
    if args.next().is_some() {
        bail!("too many arguments; expected HTML READ_ROOT");
    }
    let html = html.canonicalize().context("cannot find HTML file")?;
    let root = root.canonicalize().context("cannot find read access directory")?;
    if !html.is_file() || !root.is_dir() || !html.starts_with(&root) {
        bail!("HTML must be a file inside the read-access root");
    }

    // Safety: this executable's entrypoint is the macOS process main thread.
    // The NSApplication and WKWebView objects stay on it for their lifetime.
    let _app = unsafe { NSApplication::sharedApplication() };
    let configuration = WKWebViewConfiguration::new();
    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: 800.0, height: 1200.0 },
    };
    let view: Retained<WKWebView> = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(), frame, &configuration)
    };

    let url = NSURL::fileURLWithPath_isDirectory(
        &NSString::from_str(&html.to_string_lossy()),
        false,
    );
    let access = NSURL::fileURLWithPath_isDirectory(
        &NSString::from_str(&root.to_string_lossy()),
        true,
    );
    // Safety: URLs are local filesystem URLs, view stays on main thread,
    // and the access root encloses the HTML file.
    unsafe { view.loadFileURL_allowingReadAccessToURL(&url, &access) };

    // First-stage smoke check only: this proves navigation stops loading.
    // A navigation delegate + font/image readiness checks are mandatory
    // before using the existing viewer's 'ready' contract.
    let timeout = Instant::now() + Duration::from_secs(15);
    let loop_ = NSRunLoop::currentRunLoop();
    let mut ever_loading = false;
    while Instant::now() < timeout {
        if unsafe { view.isLoading() } {
            ever_loading = true;
        } else if ever_loading {
            println!("{}", serde_json::to_string(&protocol::Response::Ready {
                height: 0.0,
            })?);
            return Ok(());
        }
        let limit = NSDate::dateWithTimeIntervalSinceNow(0.02);
        loop_.runUntilDate(&limit);
    }
    bail!("WebKit did not complete initial navigation within 15s")
}
