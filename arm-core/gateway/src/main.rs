//! arm-gateway: the HTTP edge of the 3-tier split. It serves the static web
//! console(s) and bridges their calls to the arm-core driver's Unix-socket
//! protocol. Two consumption styles coexist:
//!   - JSON API (GET /state, /inputs; POST /jog,/home,/stop,/raw) — the TS
//!     console polls this; curl/other clients use it too.
//!   - SSE (GET /events) — pushes rendered HTML fragments (state + inputs) by
//!     subscribing to the driver's event stream once and fanning out. The htmx
//!     console consumes this for live updates with no client polling.
//!
//! Multi-threaded: one thread per request (so a long-lived /events stream does
//! not block other requests), plus one background "pump" holding the driver
//! subscription. Still no async runtime.
//!
//! Usage:  arm-gateway [http_addr] [socket_path] [webroot]
//!   defaults: 0.0.0.0:2600, /tmp/arm-core.sock, and no webroot (API only).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use arm_core_types::{HomeRequest, JogRequest, RawRequest};

const DEFAULT_HTTP: &str = "0.0.0.0:2600";
const DEFAULT_SOCKET: &str = "/tmp/arm-core.sock";

type Body = std::io::Cursor<Vec<u8>>;
/// SSE subscribers: each holds a channel that receives ready-to-send SSE chunks.
type SseSubs = Arc<Mutex<Vec<Sender<String>>>>;

/// One request/reply round-trip to the driver over a fresh socket connection.
fn call(socket_path: &str, cmd: Value) -> Result<Value> {
    let stream =
        UnixStream::connect(socket_path).with_context(|| format!("connecting {socket_path}"))?;
    let mut writer = stream.try_clone()?;
    writer.write_all(cmd.to_string().as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(anyhow!("driver closed the connection"));
        }
        let v: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("event").is_some() {
            continue;
        }
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(v.get("result").cloned().unwrap_or_else(|| json!({})));
        }
        return Err(anyhow!(v
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("driver error")
            .to_string()));
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn json_response(status: u16, body: &Value) -> Response<Body> {
    Response::from_data(body.to_string().into_bytes())
        .with_status_code(status)
        .with_header(header("Access-Control-Allow-Origin", "*"))
        .with_header(header("Access-Control-Allow-Methods", "GET, POST, OPTIONS"))
        .with_header(header("Access-Control-Allow-Headers", "Content-Type"))
        .with_header(header("Content-Type", "application/json"))
}

fn text_response(status: u16, msg: &str) -> Response<Body> {
    Response::from_data(msg.as_bytes().to_vec())
        .with_status_code(status)
        .with_header(header("Content-Type", "text/plain; charset=utf-8"))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") | Some("map") => "application/json",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn is_api_path(path: &str) -> bool {
    matches!(path, "/state" | "/inputs" | "/jog" | "/home" | "/stop" | "/raw")
}

/// Serve a file from the web root, with a path-traversal guard. A trailing slash
/// maps to index.html in that directory.
fn serve_static(webroot: &Option<PathBuf>, path: &str) -> Response<Body> {
    let Some(root) = webroot else {
        return text_response(404, "no web root configured");
    };
    let mut target = root.join(path.trim_start_matches('/'));
    // A directory (with or without a trailing slash) serves its index.html.
    if target.is_dir() {
        target = target.join("index.html");
    }
    let (Ok(canon_root), Ok(target)) = (root.canonicalize(), target.canonicalize()) else {
        return text_response(404, "not found");
    };
    if !target.starts_with(&canon_root) {
        return text_response(403, "forbidden");
    }
    match std::fs::read(&target) {
        Ok(bytes) => Response::from_data(bytes)
            .with_status_code(200)
            .with_header(header("Content-Type", content_type(&target))),
        Err(_) => text_response(404, "not found"),
    }
}

fn handle_api(socket: &str, method: &Method, path: &str, body: &str) -> Result<Value> {
    match (method, path) {
        (Method::Get, "/state") => call(socket, json!({ "cmd": "status" })),
        (Method::Get, "/inputs") => call(socket, json!({ "cmd": "inputs" })),
        (Method::Post, "/jog") => {
            let r: JogRequest = serde_json::from_str(body).context("invalid /jog body")?;
            call(socket, json!({ "cmd": "jog", "axis": r.axis, "delta": r.delta, "feed": r.feed }))
        }
        (Method::Post, "/home") => {
            let r: HomeRequest = serde_json::from_str(body).context("invalid /home body")?;
            call(socket, json!({ "cmd": "home", "axis": r.axis }))
        }
        (Method::Post, "/stop") => call(socket, json!({ "cmd": "stop" })),
        (Method::Post, "/raw") => {
            let r: RawRequest = serde_json::from_str(body).context("invalid /raw body")?;
            call(socket, json!({ "cmd": "raw", "line": r.line }))
        }
        _ => Err(anyhow!("not allowed: {} {}", method, path)),
    }
}

// --- HTML rendering for the SSE (htmx) console ------------------------------

fn num(d: &Value, k: &str) -> String {
    match d.get(k) {
        Some(v) if !v.is_null() => v.as_f64().map(|f| format!("{f:.3}")).unwrap_or_else(|| v.to_string()),
        _ => "—".to_string(),
    }
}

fn flag(d: &Value, k: &str) -> &'static str {
    if d.get(k).and_then(Value::as_i64) == Some(1) { "yes" } else { "no" }
}

fn render_state_html(d: &Value) -> String {
    let statname = match d.get("stat").and_then(Value::as_i64) {
        Some(1) => "ready", Some(3) => "stop", Some(4) => "end", Some(5) => "run",
        Some(6) => "hold", Some(9) => "homing", _ => "?",
    };
    let mut s = String::from("<table><tr><th>axis</th><th>pos</th><th>homed</th></tr>");
    for (ax, pk, hk) in [
        ("X", "posx", "homx"), ("Y", "posy", "homy"), ("Z", "posz", "homz"),
        ("A", "posa", "homa"), ("B", "posb", "homb"), ("C", "posc", ""),
    ] {
        let h = if hk.is_empty() { String::new() } else { flag(d, hk).to_string() };
        s.push_str(&format!("<tr><td>{ax}</td><td>{}</td><td>{h}</td></tr>", num(d, pk)));
    }
    s.push_str(&format!(
        "</table><p>stat: {statname} · all homed: {} · queue: {}</p>",
        flag(d, "home"), num(d, "qr")
    ));
    s
}

fn render_inputs_html(d: &Value) -> String {
    let mut s = String::from("<div class=\"inputs\">");
    for n in 1..=9 {
        let on = d.get(n.to_string().as_str()).and_then(Value::as_i64) == Some(1);
        let cls = if matches!(n, 2 | 3 | 4 | 5 | 6) { "in homing" } else { "in" };
        s.push_str(&format!(
            "<span class=\"{cls}\"><span class=\"dot {}\"></span> in{n}</span>",
            if on { "on" } else { "" }
        ));
    }
    s.push_str("</div>");
    s
}

/// Background pump: hold one driver subscription, render each state event to
/// HTML, also poll inputs, and fan both out to SSE subscribers as named events.
/// Reconnects if the driver restarts.
fn sse_pump(socket: String, subs: SseSubs) {
    loop {
        if let Ok(stream) = UnixStream::connect(&socket) {
            if let Ok(mut w) = stream.try_clone() {
                let _ = w.write_all(b"{\"cmd\":\"subscribe\"}\n");
                let _ = w.flush();
            }
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
                if v.get("event").and_then(Value::as_str) != Some("state") {
                    continue;
                }
                let data = v.get("data").cloned().unwrap_or(Value::Null);
                let mut chunks = Vec::new();
                // Skip partial status reports (no posx) so the display isn't
                // overwritten with placeholders; keep the last good render.
                if data.get("posx").is_some_and(|p| !p.is_null()) {
                    chunks.push(format!("event: state\ndata: {}\n\n", render_state_html(&data)));
                }
                if let Ok(inputs) = call(&socket, json!({ "cmd": "inputs" })) {
                    chunks.push(format!("event: inputs\ndata: {}\n\n", render_inputs_html(&inputs)));
                }
                if chunks.is_empty() {
                    continue;
                }
                let mut guard = subs.lock().unwrap();
                guard.retain(|tx| chunks.iter().all(|c| tx.send(c.clone()).is_ok()));
            }
        }
        thread::sleep(Duration::from_secs(2)); // driver gone/restarting; retry
    }
}

/// Stream Server-Sent Events to one client until it disconnects.
fn handle_sse(req: Request, subs: &SseSubs) {
    let (tx, rx) = mpsc::channel::<String>();
    subs.lock().unwrap().push(tx);
    let mut w = req.into_writer();
    // Connection: close (read-until-close framing) — browsers reject keep-alive
    // without Content-Length/chunked. The preamble opens the stream and sets the
    // EventSource reconnect interval.
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n: connected\n\nretry: 2000\n\n";
    if w.write_all(head.as_bytes()).is_err() || w.flush().is_err() {
        return;
    }
    for chunk in rx {
        if w.write_all(chunk.as_bytes()).is_err() || w.flush().is_err() {
            break; // client gone; the pump prunes this subscriber on next send
        }
    }
}

fn handle_request(mut req: Request, socket: &str, webroot: &Option<PathBuf>, subs: &SseSubs) {
    let method = req.method().clone();
    let path = req.url().split('?').next().unwrap_or("/").to_string();

    if method == Method::Get && path == "/events" {
        handle_sse(req, subs);
        return;
    }
    if method == Method::Options {
        let mut resp = Response::empty(204);
        for h in [
            header("Access-Control-Allow-Origin", "*"),
            header("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
            header("Access-Control-Allow-Headers", "Content-Type"),
        ] {
            resp = resp.with_header(h);
        }
        let _ = req.respond(resp);
        return;
    }

    let mut body = String::new();
    let _ = req.as_reader().read_to_string(&mut body);

    let resp = if is_api_path(&path) {
        match handle_api(socket, &method, &path, &body) {
            Ok(v) => json_response(200, &v),
            Err(e) => json_response(502, &json!({ "error": e.to_string() })),
        }
    } else if method == Method::Get {
        serve_static(webroot, &path)
    } else {
        json_response(404, &json!({ "error": format!("not found: {} {}", method, path) }))
    };
    let _ = req.respond(resp);
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let http_addr = args.next().unwrap_or_else(|| DEFAULT_HTTP.to_string());
    let socket = args.next().unwrap_or_else(|| DEFAULT_SOCKET.to_string());
    let webroot = args.next().map(PathBuf::from);

    let subs: SseSubs = Arc::new(Mutex::new(Vec::new()));
    {
        let socket = socket.clone();
        let subs = Arc::clone(&subs);
        thread::spawn(move || sse_pump(socket, subs));
    }

    let server = Arc::new(
        Server::http(http_addr.as_str())
            .map_err(|e| anyhow!("starting HTTP server on {http_addr}: {e}"))?,
    );
    match &webroot {
        Some(w) => eprintln!("arm-gateway: http://{http_addr} (web: {}) -> unix:{socket}", w.display()),
        None => eprintln!("arm-gateway: http://{http_addr} (API only) -> unix:{socket}"),
    }

    for req in server.incoming_requests() {
        let socket = socket.clone();
        let webroot = webroot.clone();
        let subs = Arc::clone(&subs);
        thread::spawn(move || handle_request(req, &socket, &webroot, &subs));
    }
    Ok(())
}
