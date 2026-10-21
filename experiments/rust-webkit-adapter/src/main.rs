//! Objective-C WebKit bootstrap PoC, separate from doctty production code.
//! The initial milestone verifies that WKWebView can be owned and loaded by Rust.
//! Snapshot and outline parity are intentionally *not* claimed in this commit.
#![deny(unsafe_op_in_unsafe_fn)]

mod protocol;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    macos::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This isolated WebKit adapter runs on macOS only.");
}
