//! Opt-in JSON Lines tracing for the benchmark harness (`DOCTTY_BENCH_TRACE_FILE`).

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const TRACE_FILE_ENV: &str = "DOCTTY_BENCH_TRACE_FILE";

static SINK: OnceLock<Option<Mutex<File>>> = OnceLock::new();
static PROC: OnceLock<&'static str> = OnceLock::new();
static FAILED: AtomicBool = AtomicBool::new(false);

pub fn init(proc_name: &'static str) {
    let _ = PROC.set(proc_name);
}

fn sink() -> Option<&'static Mutex<File>> {
    SINK.get_or_init(|| open_sink(std::env::var_os(TRACE_FILE_ENV).as_deref()).map(Mutex::new))
        .as_ref()
}

pub fn enabled() -> bool {
    sink().is_some() && !FAILED.load(Ordering::Relaxed)
}

/// `fields` must be a JSON object; `Null` means no fields.
pub fn event(name: &str, fields: Value) {
    let Some(sink) = sink() else { return };
    if FAILED.load(Ordering::Relaxed) {
        return;
    }
    let proc = PROC.get().copied().unwrap_or("unknown");
    let Ok(mut file) = sink.lock() else { return };
    if let Err(err) = write_event(&mut *file, proc, now_us(), name, fields)
        && !FAILED.swap(true, Ordering::Relaxed)
    {
        eprintln!("doctty: bench trace disabled after write error: {err}");
    }
}

/// UNIX epoch microseconds.
pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
}

fn write_event(
    out: &mut impl Write,
    proc: &str,
    t_us: u64,
    name: &str,
    fields: Value,
) -> io::Result<()> {
    let mut object = match fields {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => serde_json::Map::from_iter([("value".to_owned(), other)]),
    };
    object.insert("t_us".to_owned(), t_us.into());
    object.insert("proc".to_owned(), proc.into());
    object.insert("ev".to_owned(), name.into());
    let mut line = serde_json::to_vec(&Value::Object(object)).map_err(io::Error::from)?;
    line.push(b'\n');
    out.write_all(&line)
}

fn open_sink(path: Option<&OsStr>) -> Option<File> {
    let path = path.filter(|p| !p.is_empty())?;
    let disable = |reason: &dyn std::fmt::Display| {
        eprintln!(
            "doctty: bench trace disabled, cannot open {}: {reason}",
            path.to_string_lossy()
        );
    };
    // Pipes and devices can block a synchronous write (or the open itself, for a FIFO).
    if std::fs::metadata(path).is_ok_and(|m| !m.file_type().is_file()) {
        disable(&"not a regular file");
        return None;
    }
    match File::options().create(true).append(true).open(path) {
        Ok(file) if file.metadata().is_ok_and(|m| m.file_type().is_file()) => Some(file),
        Ok(_) => {
            disable(&"not a regular file");
            None
        }
        Err(err) => {
            disable(&err);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use serde_json::json;
    use std::process::Command;
    use std::time::Instant;

    fn render(fields: Value) -> Result<Value> {
        let mut buf = Vec::new();
        write_event(&mut buf, "cli", 1_700_000_000_123_456, "probe", fields)?;
        assert_eq!(buf.iter().filter(|&&b| b == b'\n').count(), 1);
        assert_eq!(buf.last(), Some(&b'\n'));
        Ok(serde_json::from_slice(&buf)?)
    }

    #[test]
    fn open_sink_returns_none_and_creates_no_file_when_path_is_unset_or_empty() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(open_sink(None).is_none());
        assert!(open_sink(Some(OsStr::new(""))).is_none());
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn write_event_emits_reserved_keys_and_merges_fields_at_top_level() -> Result<()> {
        let line = render(json!({"bytes": 4096, "label": "x"}))?;
        assert_eq!(
            line,
            json!({"t_us": 1_700_000_000_123_456u64, "proc": "cli", "ev": "probe", "bytes": 4096, "label": "x"})
        );
        Ok(())
    }

    #[test]
    fn write_event_with_null_fields_emits_only_reserved_keys() -> Result<()> {
        let line = render(Value::Null)?;
        assert_eq!(
            line,
            json!({"t_us": 1_700_000_000_123_456u64, "proc": "cli", "ev": "probe"})
        );
        Ok(())
    }

    #[test]
    fn write_event_keeps_reserved_values_when_fields_collide() -> Result<()> {
        let line = render(json!({"t_us": 7, "proc": "evil", "ev": "evil", "keep": 3}))?;
        assert_eq!(
            line,
            json!({"t_us": 1_700_000_000_123_456u64, "proc": "cli", "ev": "probe", "keep": 3})
        );
        Ok(())
    }

    #[test]
    fn write_event_stores_non_object_fields_under_value() -> Result<()> {
        assert_eq!(render(json!([1, 2]))?["value"], json!([1, 2]));
        assert_eq!(render(json!(42))?["value"], json!(42));
        Ok(())
    }

    #[test]
    fn open_sink_returns_none_for_directory_and_missing_parent() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(open_sink(Some(dir.path().as_os_str())).is_none());
        let missing = dir.path().join("no-such-dir").join("trace.jsonl");
        assert!(open_sink(Some(missing.as_os_str())).is_none());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn open_sink_returns_none_for_existing_non_regular_file() {
        assert!(open_sink(Some(OsStr::new("/dev/null"))).is_none());
    }

    #[test]
    fn concurrent_child_writer() -> Result<()> {
        if std::env::var_os("DOCTTY_BENCH_TRACE_CHILD").is_none() {
            return Ok(());
        }
        init("cli");
        for i in 0..500 {
            event("child", json!({"i": i}));
        }
        Ok(())
    }

    #[test]
    fn two_processes_appending_to_one_file_produce_only_whole_lines() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("trace.jsonl");
        let exe = std::env::current_exe()?;
        let spawn = || -> Result<std::process::Child> {
            Ok(Command::new(&exe)
                .args([
                    "--exact",
                    "support::bench_trace::tests::concurrent_child_writer",
                ])
                .env("DOCTTY_BENCH_TRACE_CHILD", "1")
                .env(TRACE_FILE_ENV, &path)
                .spawn()?)
        };
        let mut children = [spawn()?, spawn()?];
        for child in &mut children {
            assert!(child.wait()?.success());
        }
        let text = std::fs::read_to_string(&path)?;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1000);
        for line in lines {
            let v: Value = serde_json::from_str(line)?;
            assert_eq!(v["ev"], "child");
        }
        Ok(())
    }

    #[test]
    #[ignore = "overhead measurement, run with --release --nocapture"]
    fn bench_trace_overhead() -> Result<()> {
        const N: u32 = 100_000;
        assert!(!enabled(), "unset {TRACE_FILE_ENV} before measuring");
        let start = Instant::now();
        for i in 0..N {
            event("probe", json!({"i": i}));
        }
        println!(
            "disabled: {} ns/event",
            start.elapsed().as_nanos() / u128::from(N)
        );

        let dir = tempfile::tempdir()?;
        let file = open_sink(Some(dir.path().join("t.jsonl").as_os_str()))
            .ok_or_else(|| anyhow::anyhow!("cannot open temp trace file"))?;
        let sink = Mutex::new(file);
        let start = Instant::now();
        for i in 0..N {
            let mut out = sink.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            write_event(&mut *out, "cli", now_us(), "probe", json!({"i": i}))?;
        }
        println!(
            "enabled: {} ns/event",
            start.elapsed().as_nanos() / u128::from(N)
        );
        Ok(())
    }
}
