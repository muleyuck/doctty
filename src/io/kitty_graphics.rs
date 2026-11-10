#[cfg(unix)]
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

const SUPPORT_QUERY_IMAGE_ID: u32 = 31;

/// Kitty graphics query followed by a primary device attributes request.
/// Every terminal answers the latter, so its reply bounds the wait.
#[must_use]
pub fn encode_support_query() -> String {
    format!("\u{1b}_Gi={SUPPORT_QUERY_IMAGE_ID},s=1,v=1,a=q,t=d,f=24;AAAA\u{1b}\\\u{1b}[c")
}

/// Returns `None` until the device attributes reply is complete.
#[must_use]
pub fn parse_support_reply(reply: &[u8]) -> Option<bool> {
    let attributes_start = find_device_attributes_reply(reply)?;
    let ok = format!("\u{1b}_Gi={SUPPORT_QUERY_IMAGE_ID};OK\u{1b}\\");
    Some(
        reply[..attributes_start]
            .windows(ok.len())
            .any(|window| window == ok.as_bytes()),
    )
}

fn find_device_attributes_reply(reply: &[u8]) -> Option<usize> {
    (0..reply.len()).find(|&start| {
        let Some(rest) = reply[start..].strip_prefix(b"\x1b[?") else {
            return false;
        };
        let parameters = rest
            .iter()
            .take_while(|byte| byte.is_ascii_digit() || **byte == b';')
            .count();
        rest.get(parameters) == Some(&b'c')
    })
}

#[cfg(unix)]
const SUPPORT_QUERY_TIMEOUT: Duration = Duration::from_secs(1);

/// Asks the terminal on stdin/stdout whether it accepts Kitty graphics.
/// Multiplexers such as Herdr answer for their panes, so the terminal name
/// is not needed.
pub fn probe_kitty_graphics_support() -> Result<bool> {
    enable_raw_mode()?;
    let supported = query_kitty_graphics_support();
    disable_raw_mode()?;
    supported
}

#[cfg(unix)]
fn query_kitty_graphics_support() -> Result<bool> {
    use rustix::{
        event::{PollFd, PollFlags, Timespec, poll},
        io::Errno,
    };
    use std::os::fd::AsFd;

    let mut stdout = io::stdout();
    stdout.write_all(encode_support_query().as_bytes())?;
    stdout.flush()?;

    let stdin = io::stdin();
    let stdin = stdin.as_fd();
    let deadline = Instant::now() + SUPPORT_QUERY_TIMEOUT;
    let mut reply = Vec::new();
    let mut buffer = [0_u8; 256];
    loop {
        if let Some(supported) = parse_support_reply(&reply) {
            return Ok(supported);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        let timeout = Timespec::try_from(remaining)?;
        let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => return Ok(false),
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
        // Read the fd directly: std's buffered stdin could keep bytes that
        // belong to the viewer's first key events.
        let read = match rustix::io::read(stdin, &mut buffer) {
            Ok(read) => read,
            Err(Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        };
        if read == 0 {
            return Ok(false);
        }
        reply.extend_from_slice(&buffer[..read]);
    }
}

#[cfg(not(unix))]
fn query_kitty_graphics_support() -> Result<bool> {
    Ok(false)
}
