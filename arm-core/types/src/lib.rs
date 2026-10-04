//! arm-core-types: the wire contract between the arm-core daemon and its
//! clients. Pure data shapes, no logic and no I/O, so a Rust/WASM client can
//! depend on exactly these definitions and the HTTP+JSON contract is checked at
//! compile time on both ends. The daemon's parsing/serial logic lives in
//! arm-core; this crate is only the shapes that cross the wire.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// g2core response footer: `[revision, status_code, ...]`. `status_code == 0`
/// means the command was accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Footer {
    pub revision: i64,
    pub status_code: i64,
}

impl Footer {
    pub fn is_ok(&self) -> bool {
        self.status_code == 0
    }
}

/// g2core status report (the `GET /state` body). Every field is optional because
/// g2core emits partial reports carrying only the values that changed.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct StatusReport {
    pub line: Option<i64>,
    pub posx: Option<f64>,
    pub posy: Option<f64>,
    pub posz: Option<f64>,
    pub posa: Option<f64>,
    pub posb: Option<f64>,
    pub posc: Option<f64>,
    pub vel: Option<f64>,
    /// Machine state: 1=ready, 3=stop, 4=program-end, 5=run, 6=hold, 9=homing.
    pub stat: Option<i64>,
    /// 1 once every axis is homed.
    pub home: Option<i64>,
    pub homx: Option<i64>,
    pub homy: Option<i64>,
    pub homz: Option<i64>,
    pub homa: Option<i64>,
    pub homb: Option<i64>,
    /// Queue report: free move buffers.
    pub qr: Option<i64>,
}

impl StatusReport {
    /// Merge a (possibly partial) report into this one: every field that is
    /// `Some` in `other` overwrites ours. g2core emits deltas — only changed
    /// fields — so a consumer must accumulate them to hold the full state.
    pub fn merge(&mut self, other: &StatusReport) {
        macro_rules! merge_fields {
            ($($field:ident),* $(,)?) => {
                $( if other.$field.is_some() { self.$field = other.$field; } )*
            };
        }
        merge_fields!(
            line, posx, posy, posz, posa, posb, posc, vel, stat, home, homx, homy, homz,
            homa, homb, qr,
        );
    }
}

/// One parsed line from the controller. The `POST /raw` response carries these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// A command response: the `r` body plus the optional footer.
    Response { body: Value, footer: Option<Footer> },
    /// An asynchronous status report.
    Status(StatusReport),
    /// Valid JSON we received but did not classify.
    Other(Value),
}

fn default_feed() -> f64 {
    500.0
}

/// `POST /jog` body: relative jog of one axis (capped server-side).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct JogRequest {
    pub axis: char,
    pub delta: f64,
    #[serde(default = "default_feed")]
    pub feed: f64,
}

/// `POST /home` body: start homing one axis.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct HomeRequest {
    pub axis: char,
}

/// `POST /raw` body: a raw g2core line to send verbatim. (Edge/gateway HTTP shape.)
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RawRequest {
    pub line: String,
}

// --- driver socket protocol -------------------------------------------------
//
// arm-core's driver interface is line-delimited JSON over a Unix socket. A
// client sends one `Command` object per line (optionally with an `id` for
// correlation); the driver replies with a `Response` line and may push `Event`
// lines (state, faults) to subscribers.

/// A command from a driver client. Wire form: `{"cmd":"jog","axis":"Y","delta":2}`.
/// An optional top-level `id` (handled outside this enum) is echoed in the reply.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Read the current status report.
    Status,
    /// Read hall/endstop inputs in1..in9.
    Inputs,
    /// Relative jog of one axis (capped server-side). Requires motion control.
    Jog {
        axis: char,
        delta: f64,
        #[serde(default = "default_feed")]
        feed: f64,
    },
    /// Home one axis. Requires motion control.
    Home { axis: char },
    /// Feedhold + flush. Always allowed (safety), no control needed.
    Stop,
    /// Send a raw g2core line. Requires motion control (it can move the arm).
    Raw { line: String },
    /// Subscribe this connection to pushed events (state, faults).
    Subscribe,
    /// Take motion control (exclusive). Fails if another connection holds it.
    Acquire,
    /// Release motion control.
    Release,
}

/// A reply to a `Command`. Wire form: `{"id":1,"ok":true,"result":{...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// An unsolicited push to subscribers. Wire form: `{"event":"state","data":{...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub event: String,
    pub data: Value,
}
