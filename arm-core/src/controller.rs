//! Controller: owns a `Transport` and speaks g2core. Every access to the arm
//! goes through here, so command formatting and safety caps live in one place.
//! The web API calls these methods. Motion is expressed in g2core axis space
//! (X=j0, Y=j1, Z=j2, A/B=wrist) for now; calibrated joint-space kinematics is
//! a later layer.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::g2core::{self, Message, StatusReport};
use crate::transport::Transport;

const READ_TIMEOUT: Duration = Duration::from_millis(400);

/// Largest single relative jog we will issue, in g2core axis units. A debug UI
/// sends small nudges; this cap stops a fat-fingered value from flinging a joint.
pub const MAX_JOG: f64 = 30.0;

/// Valid g2core axis letters for this 6-motor arm.
const AXES: &[char] = &['X', 'Y', 'Z', 'A', 'B', 'C'];

pub struct Controller<T: Transport> {
    pub(crate) transport: T,
    /// Accumulated status: g2core sends deltas, so we merge each report into a
    /// running full picture and hand that out.
    state: StatusReport,
}

impl<T: Transport> Controller<T> {
    pub fn new(transport: T) -> Self {
        Self { transport, state: StatusReport::default() }
    }

    /// Write one line and parse whatever comes back within the read window.
    fn exchange(&mut self, line: &str) -> Result<Vec<Message>> {
        self.transport.write_line(line)?;
        let raw = self.transport.read_available(READ_TIMEOUT)?;
        Ok(raw
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .filter_map(|l| g2core::parse_line(l).ok())
            .collect())
    }

    /// Read one g2core key (read-only get). Returns the value for that key.
    pub fn query(&mut self, key: &str) -> Result<Value> {
        for m in self.exchange(&g2core::get(key))? {
            if let Message::Response { body, .. } = m {
                return Ok(body.get(key).cloned().unwrap_or(body));
            }
        }
        Err(anyhow!("no response for {key}"))
    }

    /// Current status report. g2core reports are deltas, so each one is merged
    /// into the accumulated state and the full picture is returned.
    pub fn status(&mut self) -> Result<StatusReport> {
        for m in self.exchange(&g2core::get("sr"))? {
            let sr = match m {
                Message::Status(sr) => Some(sr),
                Message::Response { body, .. } => g2core::status_from_response_body(&body),
                _ => None,
            };
            if let Some(sr) = sr {
                self.state.merge(&sr);
                return Ok(self.state.clone());
            }
        }
        Err(anyhow!("no status report"))
    }

    /// Read a digital input (hall endstop) state, e.g. `input(6)` -> 0/1.
    pub fn input(&mut self, n: u8) -> Result<i64> {
        self.query(&format!("in{n}"))?
            .as_i64()
            .ok_or_else(|| anyhow!("in{n} not an integer"))
    }

    /// Read several digital inputs in a single round-trip: write all the queries
    /// first, then read once. One serial round-trip instead of N, so the sensor
    /// panel stays responsive. Returns a map of input number -> state.
    pub fn inputs(&mut self, ns: &[u8]) -> Result<BTreeMap<u8, i64>> {
        for &n in ns {
            self.transport.write_line(&g2core::get(&format!("in{n}")))?;
        }
        let raw = self.transport.read_available(READ_TIMEOUT)?;
        let mut out = BTreeMap::new();
        for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Ok(Message::Response { body, .. }) = g2core::parse_line(line) {
                if let Some(obj) = body.as_object() {
                    for (k, v) in obj {
                        if let (Some(rest), Some(val)) = (k.strip_prefix("in"), v.as_i64()) {
                            if let Ok(n) = rest.parse::<u8>() {
                                out.insert(n, val);
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Relative jog of one axis by `delta` units at `feed` units/min, capped at
    /// `MAX_JOG`. Sends G91 (relative) for the move, then restores G90 (absolute)
    /// so the rest of the system keeps its absolute frame.
    pub fn jog_axis(&mut self, axis: char, delta: f64, feed: f64) -> Result<()> {
        let axis = axis.to_ascii_uppercase();
        if !AXES.contains(&axis) {
            return Err(anyhow!("invalid axis {axis:?}"));
        }
        if !delta.is_finite() || delta.abs() > MAX_JOG {
            return Err(anyhow!("jog {delta} exceeds MAX_JOG {MAX_JOG}"));
        }
        let feed = feed.max(1.0);
        self.transport
            .write_line(&format!("G91 G1 {axis}{delta} F{feed}"))?;
        self.transport.write_line("G90")?;
        Ok(())
    }

    /// Start g2core's homing cycle for one axis (e.g. 'Y' homes J1). Use only on
    /// axes whose endstop is known good; J0 (X) is a known-bad endstop right now.
    pub fn home_axis(&mut self, axis: char) -> Result<()> {
        let axis = axis.to_ascii_uppercase();
        if !AXES.contains(&axis) {
            return Err(anyhow!("invalid axis {axis:?}"));
        }
        self.transport.write_line(&format!("G28.2 {axis}0"))
    }

    /// Immediately stop motion: feedhold then queue flush.
    pub fn stop(&mut self) -> Result<()> {
        self.transport.write_line("!")?; // feedhold
        self.transport.write_line("%")?; // flush queued moves
        Ok(())
    }

    /// Send a raw line and return the parsed responses (debug console passthrough).
    pub fn raw(&mut self, line: &str) -> Result<Vec<Message>> {
        self.exchange(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;

    fn ctrl() -> Controller<MockTransport> {
        Controller::new(MockTransport::new())
    }

    #[test]
    fn status_parses_response_wrapped_sr() {
        let mut c = ctrl();
        c.transport
            .push_response(r#"{"r":{"sr":{"posy":139.7344,"stat":4,"home":1}},"f":[1,0,13]}"#);
        let sr = c.status().unwrap();
        assert_eq!(sr.posy, Some(139.7344));
        assert_eq!(sr.home, Some(1));
    }

    #[test]
    fn status_parses_async_sr() {
        let mut c = ctrl();
        c.transport.push_response(r#"{"sr":{"stat":5,"vel":120.0}}"#);
        let sr = c.status().unwrap();
        assert_eq!(sr.stat, Some(5));
        assert_eq!(sr.vel, Some(120.0));
    }

    #[test]
    fn status_accumulates_deltas() {
        // g2core sends a full report, then deltas. status() must retain fields
        // the later (partial) report omits.
        let mut c = ctrl();
        c.transport.push_response(r#"{"r":{"sr":{"posx":-2.0,"stat":3}},"f":[1,0,13]}"#);
        c.transport.push_response(r#"{"r":{"sr":{"stat":5}},"f":[1,0,13]}"#);
        let first = c.status().unwrap();
        assert_eq!(first.posx, Some(-2.0));
        assert_eq!(first.stat, Some(3));
        let second = c.status().unwrap();
        assert_eq!(second.posx, Some(-2.0)); // retained; the delta had no posx
        assert_eq!(second.stat, Some(5)); // updated by the delta
    }

    #[test]
    fn input_reads_hall_state() {
        let mut c = ctrl();
        c.transport.push_response(r#"{"r":{"in6":1},"f":[1,0,13]}"#);
        assert_eq!(c.input(6).unwrap(), 1);
    }

    #[test]
    fn inputs_reads_multiple_in_one_round_trip() {
        let mut c = ctrl();
        // One read returns several input responses (g2core answers each query).
        c.transport.push_response(
            "{\"r\":{\"in2\":0},\"f\":[1,0,13]}\n{\"r\":{\"in6\":1},\"f\":[1,0,13]}",
        );
        let m = c.inputs(&[2, 6]).unwrap();
        assert_eq!(m.get(&2), Some(&0));
        assert_eq!(m.get(&6), Some(&1));
        // All queries were written before the single read.
        assert_eq!(c.transport.written, vec![r#"{"in2":null}"#, r#"{"in6":null}"#]);
    }

    #[test]
    fn jog_writes_relative_move_then_restores_absolute() {
        let mut c = ctrl();
        c.jog_axis('y', 5.0, 500.0).unwrap();
        assert_eq!(c.transport.written, vec!["G91 G1 Y5 F500", "G90"]);
    }

    #[test]
    fn jog_handles_negative_delta() {
        let mut c = ctrl();
        c.jog_axis('Z', -2.5, 300.0).unwrap();
        assert_eq!(c.transport.written, vec!["G91 G1 Z-2.5 F300", "G90"]);
    }

    #[test]
    fn jog_rejects_oversize_delta_without_writing() {
        let mut c = ctrl();
        assert!(c.jog_axis('Y', 100.0, 500.0).is_err());
        assert!(c.transport.written.is_empty());
    }

    #[test]
    fn jog_rejects_invalid_axis() {
        let mut c = ctrl();
        assert!(c.jog_axis('Q', 1.0, 100.0).is_err());
        assert!(c.transport.written.is_empty());
    }

    #[test]
    fn home_writes_g282_for_axis() {
        let mut c = ctrl();
        c.home_axis('y').unwrap();
        assert_eq!(c.transport.written, vec!["G28.2 Y0"]);
    }

    #[test]
    fn stop_sends_feedhold_then_flush() {
        let mut c = ctrl();
        c.stop().unwrap();
        assert_eq!(c.transport.written, vec!["!", "%"]);
    }
}
