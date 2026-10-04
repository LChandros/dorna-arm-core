//! Transport: the byte-level link to the controller, behind a trait.
//!
//! `SerialTransport` is the real serial port. `MockTransport` records what was
//! written and replays queued responses, so the controller and codec can be
//! tested through scripted exchanges with no hardware attached. This trait is
//! the single seam that keeps the rest of the crate hardware-independent.

use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

pub trait Transport {
    /// Write one command line. A trailing newline is added if missing.
    fn write_line(&mut self, line: &str) -> Result<()>;

    /// Read whatever bytes arrive within `timeout`, decoded as text. g2core
    /// responses are newline-delimited JSON; the caller frames lines.
    fn read_available(&mut self, timeout: Duration) -> Result<String>;
}

const LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// A bare read query such as `{"sr":null}` or `{"in3":null}`: no side effects.
fn is_query(line: &str) -> bool {
    line.starts_with("{\"") && line.ends_with("\":null}") && !line[2..line.len() - 7].contains('"')
}

/// Append-only wire log: every line sent to and received from the controller,
/// stamped with epoch seconds. Rotates to `<path>.1` past `LOG_MAX_BYTES`.
/// The broadcaster re-sends the same read queries every tick; an exchange made
/// only of queries is logged just when its reply differs from the last time,
/// so the log shows motion and state changes without drowning in idle polls.
struct WireLog {
    path: PathBuf,
    file: File,
    pending: Vec<String>,
    all_queries: bool,
    last_reply: HashMap<String, String>,
}

impl WireLog {
    fn open(path: PathBuf) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { path, file, pending: Vec::new(), all_queries: true, last_reply: HashMap::new() })
    }

    fn write(&mut self, dir: &str, text: &str) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let line = format!("{}.{:03} {dir} {}\n", now.as_secs(), now.subsec_millis(), text);
        let _ = self.file.write_all(line.as_bytes());
        if self.file.metadata().map_or(false, |m| m.len() > LOG_MAX_BYTES) {
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
            if let Ok(f) = OpenOptions::new().create(true).append(true).open(&self.path) {
                self.file = f;
            }
        }
    }

    fn tx(&mut self, line: &str) {
        let line = line.trim_end();
        if is_query(line) {
            self.pending.push(line.to_string());
        } else {
            // A real command: flush any held queries so ordering stays true.
            for q in std::mem::take(&mut self.pending) {
                self.write("TX", &q);
            }
            self.all_queries = false;
            self.write("TX", line);
        }
    }

    fn rx(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let queries = std::mem::take(&mut self.pending);
        let polled = std::mem::replace(&mut self.all_queries, true) && !queries.is_empty();
        if polled {
            let key = queries.join(" ");
            if self.last_reply.get(&key).map_or(false, |r| r == text) {
                return;
            }
            self.last_reply.insert(key, text.to_string());
        }
        for q in &queries {
            self.write("TX", q);
        }
        for l in text.lines() {
            self.write("RX", l.trim_end());
        }
    }
}

/// Real serial transport over `/dev/ttyACM0` (or any path) at the given baud.
pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
    log: Option<WireLog>,
}

impl SerialTransport {
    pub fn open(path: &str, baud: u32) -> Result<Self> {
        let port = serialport::new(path, baud)
            // Short per-read timeout so read_available can detect an idle gap
            // and return promptly instead of always waiting the full window.
            .timeout(Duration::from_millis(20))
            .open()
            .with_context(|| format!("opening serial port {path}"))?;
        Ok(Self { port, log: None })
    }

    /// Record all controller traffic to `path` (see `WireLog`). A log that
    /// cannot be opened is reported and skipped; it never blocks control.
    pub fn with_wire_log(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        match WireLog::open(path.clone()) {
            Ok(mut log) => {
                log.write("--", "arm-core opened controller");
                self.log = Some(log);
            }
            Err(e) => eprintln!("arm-core: wire log {} disabled: {e}", path.display()),
        }
        self
    }

    /// Discard any buffered input (e.g. a startup banner) before a fresh exchange.
    pub fn clear_input(&mut self) -> Result<()> {
        self.port.clear(serialport::ClearBuffer::Input)?;
        Ok(())
    }
}

impl Transport for SerialTransport {
    fn write_line(&mut self, line: &str) -> Result<()> {
        self.port.write_all(line.as_bytes())?;
        if !line.ends_with('\n') {
            self.port.write_all(b"\n")?;
        }
        self.port.flush()?;
        if let Some(log) = self.log.as_mut() {
            log.tx(line);
        }
        Ok(())
    }

    fn read_available(&mut self, timeout: Duration) -> Result<String> {
        // Read until the responses go quiet (an idle gap after some data) or the
        // overall timeout caps us. g2core answers in milliseconds, so this
        // returns in tens of ms rather than blocking the whole window.
        const IDLE_GAP: Duration = Duration::from_millis(40);
        let mut out = Vec::new();
        let mut chunk = [0u8; 4096];
        let start = Instant::now();
        let mut last_data: Option<Instant> = None;
        while start.elapsed() < timeout {
            match self.port.read(&mut chunk) {
                Ok(n) if n > 0 => {
                    out.extend_from_slice(&chunk[..n]);
                    last_data = Some(Instant::now());
                }
                _ => {
                    // No data this tick; if we already got some and it's gone
                    // quiet, the response is complete.
                    if last_data.map_or(false, |t| t.elapsed() >= IDLE_GAP) {
                        break;
                    }
                }
            }
        }
        let text = String::from_utf8_lossy(&out).into_owned();
        if let Some(log) = self.log.as_mut() {
            log.rx(&text);
        }
        Ok(text)
    }
}

/// In-memory transport for tests. Records every line written, and replays
/// queued responses one read at a time (FIFO). An empty queue reads as "".
#[derive(Default)]
pub struct MockTransport {
    pub written: Vec<String>,
    responses: VecDeque<String>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a response that a later `read_available` will return.
    pub fn push_response(&mut self, resp: &str) {
        self.responses.push_back(resp.to_string());
    }
}

impl Transport for MockTransport {
    fn write_line(&mut self, line: &str) -> Result<()> {
        self.written.push(line.trim_end().to_string());
        Ok(())
    }

    fn read_available(&mut self, _timeout: Duration) -> Result<String> {
        Ok(self.responses.pop_front().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_records_written_lines_without_newline() {
        let mut t = MockTransport::new();
        t.write_line("{\"fv\":null}\n").unwrap();
        t.write_line("{\"sr\":null}").unwrap();
        assert_eq!(t.written, vec!["{\"fv\":null}", "{\"sr\":null}"]);
    }

    #[test]
    fn mock_replays_queued_responses_in_order() {
        let mut t = MockTransport::new();
        t.push_response(r#"{"r":{"fv":1.21},"f":[1,0,13]}"#);
        t.push_response(r#"{"sr":{"stat":4}}"#);
        assert_eq!(
            t.read_available(Duration::from_millis(1)).unwrap(),
            r#"{"r":{"fv":1.21},"f":[1,0,13]}"#
        );
        assert_eq!(
            t.read_available(Duration::from_millis(1)).unwrap(),
            r#"{"sr":{"stat":4}}"#
        );
    }

    #[test]
    fn mock_empty_queue_reads_empty() {
        let mut t = MockTransport::new();
        assert_eq!(t.read_available(Duration::from_millis(1)).unwrap(), "");
    }

    /// A `MockTransport` is usable anywhere a `Transport` is expected.
    #[test]
    fn mock_satisfies_the_trait_object() {
        fn ping(t: &mut dyn Transport) -> Result<()> {
            t.write_line(&crate::g2core::get("fv"))
        }
        let mut t = MockTransport::new();
        ping(&mut t).unwrap();
        assert_eq!(t.written, vec![r#"{"fv":null}"#]);
    }
}
