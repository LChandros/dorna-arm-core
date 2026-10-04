//! Driver protocol: parse client request lines and dispatch controller-level
//! commands. Pure logic, no sockets, so it's unit-testable with the mock.
//! Session commands (subscribe/acquire/release) affect connection state, not the
//! arm, so the server handles those; `dispatch` covers the controller commands.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use arm_core_types::{Command, Response};

use crate::controller::Controller;
use crate::transport::Transport;

const ALL_INPUTS: &[u8] = &[1, 2, 3, 4, 5, 6, 7, 8, 9];

/// Parse one request line into its optional correlation `id` and the `Command`.
/// The `id` field is stripped before decoding the command so unit commands like
/// `{"cmd":"status","id":1}` parse cleanly.
pub fn parse_request(line: &str) -> Result<(Option<u64>, Command)> {
    let mut v: Value =
        serde_json::from_str(line.trim()).map_err(|e| anyhow!("invalid request JSON: {e}"))?;
    let id = v.get("id").and_then(Value::as_u64);
    if let Some(obj) = v.as_object_mut() {
        obj.remove("id");
    }
    let command: Command =
        serde_json::from_value(v).map_err(|e| anyhow!("unknown or malformed command: {e}"))?;
    Ok((id, command))
}

/// Whether a command moves the arm (and therefore needs motion control). `Stop`
/// is intentionally NOT mutating-gated: anyone may stop the arm.
pub fn needs_control(cmd: &Command) -> bool {
    matches!(cmd, Command::Jog { .. } | Command::Home { .. } | Command::Raw { .. })
}

/// Execute a controller-level command. Session commands are rejected here (the
/// server handles them); everything else maps to a `Controller` call.
pub fn dispatch<T: Transport>(ctrl: &mut Controller<T>, cmd: &Command) -> Result<Value> {
    match cmd {
        Command::Status => Ok(serde_json::to_value(ctrl.status()?)?),
        Command::Inputs => Ok(serde_json::to_value(ctrl.inputs(ALL_INPUTS)?)?),
        Command::Jog { axis, delta, feed } => {
            ctrl.jog_axis(*axis, *delta, *feed)?;
            Ok(json!({ "ok": true }))
        }
        Command::Home { axis } => {
            ctrl.home_axis(*axis)?;
            Ok(json!({ "ok": true }))
        }
        Command::Stop => {
            ctrl.stop()?;
            Ok(json!({ "ok": true }))
        }
        Command::Raw { line } => Ok(json!({ "responses": ctrl.raw(line)? })),
        Command::Subscribe | Command::Acquire | Command::Release => {
            Err(anyhow!("session command must be handled by the server"))
        }
    }
}

/// Serialize a success reply line (the writer adds the newline).
pub fn ok_line(id: Option<u64>, result: Value) -> String {
    serde_json::to_string(&Response { id, ok: true, result: Some(result), error: None })
        .expect("Response serializes")
}

/// Serialize an error reply line.
pub fn err_line(id: Option<u64>, error: String) -> String {
    serde_json::to_string(&Response { id, ok: false, result: None, error: Some(error) })
        .expect("Response serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;

    fn ctrl() -> Controller<MockTransport> {
        Controller::new(MockTransport::new())
    }

    #[test]
    fn parses_command_with_id() {
        let (id, cmd) = parse_request(r#"{"id":7,"cmd":"jog","axis":"Y","delta":2}"#).unwrap();
        assert_eq!(id, Some(7));
        assert_eq!(cmd, Command::Jog { axis: 'Y', delta: 2.0, feed: 500.0 });
    }

    #[test]
    fn parses_unit_command_with_id() {
        // The extra `id` must not break a unit command.
        let (id, cmd) = parse_request(r#"{"cmd":"status","id":3}"#).unwrap();
        assert_eq!(id, Some(3));
        assert_eq!(cmd, Command::Status);
    }

    #[test]
    fn parses_command_without_id() {
        let (id, cmd) = parse_request(r#"{"cmd":"stop"}"#).unwrap();
        assert_eq!(id, None);
        assert_eq!(cmd, Command::Stop);
    }

    #[test]
    fn rejects_unknown_command() {
        assert!(parse_request(r#"{"cmd":"fly"}"#).is_err());
        assert!(parse_request("not json").is_err());
    }

    #[test]
    fn control_gating_classification() {
        assert!(needs_control(&Command::Jog { axis: 'Y', delta: 1.0, feed: 500.0 }));
        assert!(needs_control(&Command::Home { axis: 'Y' }));
        assert!(needs_control(&Command::Raw { line: "G0 Y1".into() }));
        assert!(!needs_control(&Command::Stop));
        assert!(!needs_control(&Command::Status));
        assert!(!needs_control(&Command::Inputs));
    }

    #[test]
    fn dispatch_status_returns_report() {
        let mut c = ctrl();
        c.transport
            .push_response(r#"{"r":{"sr":{"stat":1,"home":0}},"f":[1,0,13]}"#);
        let v = dispatch(&mut c, &Command::Status).unwrap();
        assert_eq!(v["stat"], serde_json::json!(1));
    }

    #[test]
    fn dispatch_jog_emits_gcode() {
        let mut c = ctrl();
        dispatch(&mut c, &Command::Jog { axis: 'Y', delta: 2.0, feed: 300.0 }).unwrap();
        assert_eq!(c.transport.written, vec!["G91 G1 Y2 F300", "G90"]);
    }

    #[test]
    fn dispatch_rejects_session_commands() {
        let mut c = ctrl();
        assert!(dispatch(&mut c, &Command::Subscribe).is_err());
    }

    #[test]
    fn reply_lines_serialize() {
        assert_eq!(ok_line(Some(1), json!({"ok":true})), r#"{"id":1,"ok":true,"result":{"ok":true}}"#);
        assert_eq!(err_line(None, "nope".into()), r#"{"ok":false,"error":"nope"}"#);
    }
}
