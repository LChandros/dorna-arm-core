//! arm-core driver daemon: owns the g2core controller and serves a
//! line-delimited JSON protocol over a Unix domain socket. Programmatic
//! consumers connect here (the HTTP gateway, CLI tools, a future vision/plating
//! service); the browser-facing HTTP+JSON lives in a separate gateway.
//!
//! Protocol — one JSON object per line:
//!   client -> {"id":1,"cmd":"jog","axis":"Y","delta":2}
//!   server -> {"id":1,"ok":true,"result":{...}}      (reply)
//!   server -> {"event":"state","data":{...}}         (push, to subscribers)
//!
//! Commands: status, inputs, jog, home, stop, raw, subscribe, acquire, release.
//! Motion commands (jog/home/raw) require motion control, auto-acquired if free
//! and refused if another connection holds it; stop is always allowed.
//!
//! Usage:  arm-core [socket_path] [serial_path]
//!   defaults: /tmp/arm-core.sock and the stable by-id TinyG path.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;

use arm_core::controller::Controller;
use arm_core::protocol::{dispatch, err_line, needs_control, ok_line, parse_request};
use arm_core::transport::SerialTransport;
use arm_core_types::{Command, Event};

const DEFAULT_SOCKET: &str = "/tmp/arm-core.sock";
const DEFAULT_SERIAL: &str = "/dev/serial/by-id/usb-Synthetos_TinyG_v2_0084-084a-0084-7bd6-if00";
const BAUD: u32 = 115_200;
const WIRE_LOG: &str = "/home/intern/arm-core/logs/serial.log";
const STATE_INTERVAL: Duration = Duration::from_millis(500);

type Ctrl = Arc<Mutex<Controller<SerialTransport>>>;
type Subscribers = Arc<Mutex<Vec<(u64, Sender<String>)>>>;
type Control = Arc<Mutex<Option<u64>>>;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let socket_path = args.next().unwrap_or_else(|| DEFAULT_SOCKET.to_string());
    let serial = args.next().unwrap_or_else(|| DEFAULT_SERIAL.to_string());

    eprintln!("arm-core: opening controller at {serial}");
    let mut transport =
        SerialTransport::open(&serial, BAUD)
            .with_context(|| format!("opening {serial}"))?
            .with_wire_log(WIRE_LOG);
    transport.clear_input().ok();
    let ctrl: Ctrl = Arc::new(Mutex::new(Controller::new(transport)));

    let _ = std::fs::remove_file(&socket_path); // a stale socket would block bind
    let listener =
        UnixListener::bind(&socket_path).with_context(|| format!("binding {socket_path}"))?;
    eprintln!("arm-core: listening on unix:{socket_path}");

    let subscribers: Subscribers = Arc::new(Mutex::new(Vec::new()));
    let control: Control = Arc::new(Mutex::new(None));
    let conn_ids = AtomicU64::new(1);

    // State broadcaster: poll status and push to subscribers.
    {
        let ctrl = Arc::clone(&ctrl);
        let subscribers = Arc::clone(&subscribers);
        thread::spawn(move || broadcaster(ctrl, subscribers));
    }

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let conn_id = conn_ids.fetch_add(1, Ordering::Relaxed);
        let ctrl = Arc::clone(&ctrl);
        let subscribers = Arc::clone(&subscribers);
        let control = Arc::clone(&control);
        thread::spawn(move || {
            if let Err(e) = handle(conn_id, stream, &ctrl, &subscribers, &control) {
                eprintln!("arm-core: conn {conn_id} ended: {e}");
            }
            // Tear down this connection's footprint.
            subscribers.lock().unwrap().retain(|(id, _)| *id != conn_id);
            let mut held = control.lock().unwrap();
            if *held == Some(conn_id) {
                *held = None;
            }
        });
    }
    Ok(())
}

/// Poll the controller's status on an interval and push a `state` event to every
/// subscriber. Dead subscribers (closed connections) are pruned on send failure.
fn broadcaster(ctrl: Ctrl, subscribers: Subscribers) {
    loop {
        thread::sleep(STATE_INTERVAL);
        let line = {
            let mut c = ctrl.lock().unwrap();
            match c.status() {
                Ok(sr) => serde_json::to_string(&Event {
                    event: "state".into(),
                    data: serde_json::to_value(sr).unwrap_or(json!(null)),
                })
                .ok(),
                Err(_) => None,
            }
        };
        let Some(line) = line else { continue };
        subscribers
            .lock()
            .unwrap()
            .retain(|(_, tx)| tx.send(line.clone()).is_ok());
    }
}

/// Serve one connection: a writer thread drains outbound lines to the socket
/// while this thread reads requests and produces replies.
fn handle(
    conn_id: u64,
    stream: UnixStream,
    ctrl: &Ctrl,
    subscribers: &Subscribers,
    control: &Control,
) -> Result<()> {
    let mut writer_stream = stream.try_clone()?;
    let (tx_out, rx_out) = mpsc::channel::<String>();

    let writer = thread::spawn(move || {
        for line in rx_out {
            if writer_stream.write_all(line.as_bytes()).is_err()
                || writer_stream.write_all(b"\n").is_err()
            {
                break;
            }
            let _ = writer_stream.flush();
        }
    });

    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match parse_request(&line) {
            Ok((id, cmd)) => process(conn_id, id, &cmd, ctrl, subscribers, control, &tx_out),
            Err(e) => err_line(None, e.to_string()),
        };
        if tx_out.send(reply).is_err() {
            break;
        }
    }

    drop(tx_out);
    let _ = writer.join();
    Ok(())
}

/// Handle one command and return the reply line. Session commands manage
/// connection state; motion commands are gated on the control lock.
fn process(
    conn_id: u64,
    id: Option<u64>,
    cmd: &Command,
    ctrl: &Ctrl,
    subscribers: &Subscribers,
    control: &Control,
    tx_out: &Sender<String>,
) -> String {
    match cmd {
        Command::Subscribe => {
            subscribers.lock().unwrap().push((conn_id, tx_out.clone()));
            ok_line(id, json!({ "subscribed": true }))
        }
        Command::Acquire => {
            let mut held = control.lock().unwrap();
            if held.is_none() || *held == Some(conn_id) {
                *held = Some(conn_id);
                ok_line(id, json!({ "control": true }))
            } else {
                err_line(id, format!("motion control held by connection {}", held.unwrap()))
            }
        }
        Command::Release => {
            let mut held = control.lock().unwrap();
            if *held == Some(conn_id) {
                *held = None;
            }
            ok_line(id, json!({ "control": false }))
        }
        _ => {
            // Controller command. Gate motion on the control lock (auto-acquire
            // if free); reads and stop pass through.
            if needs_control(cmd) {
                let mut held = control.lock().unwrap();
                if held.is_none() {
                    *held = Some(conn_id);
                }
                let holder = *held;
                if holder != Some(conn_id) {
                    return err_line(
                        id,
                        format!("motion control held by connection {}", holder.unwrap()),
                    );
                }
            }
            let result = dispatch(&mut ctrl.lock().unwrap(), cmd);
            match result {
                Ok(v) => ok_line(id, v),
                Err(e) => err_line(id, e.to_string()),
            }
        }
    }
}
