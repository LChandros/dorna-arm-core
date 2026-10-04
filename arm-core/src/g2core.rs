//! g2core wire codec: pure functions to build queries and parse the
//! controller's JSON responses into the shared `arm-core-types` shapes. No I/O
//! here, so every function is unit-testable against captured fixtures. The data
//! types themselves live in `arm-core-types` (shared with clients); this module
//! owns the logic that produces them.

use anyhow::{anyhow, Result};
use serde_json::Value;

// The wire types are the shared contract; re-export so the rest of the crate
// (controller, daemon) keeps referring to `g2core::StatusReport` etc.
pub use arm_core_types::{Footer, Message, StatusReport};

/// Build a read-only "get" for a g2core key: `get("fv")` => `{"fv":null}`.
/// A get never causes motion.
pub fn get(key: &str) -> String {
    format!("{{\"{key}\":null}}")
}

/// Parse one line of g2core JSON (newline already stripped) into a `Message`.
pub fn parse_line(line: &str) -> Result<Message> {
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|e| anyhow!("invalid g2core JSON {line:?}: {e}"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow!("expected a JSON object, got {line:?}"))?;

    // Asynchronous status report: top-level {"sr": {...}}.
    if let Some(sr) = obj.get("sr") {
        let report: StatusReport =
            serde_json::from_value(sr.clone()).map_err(|e| anyhow!("bad status report: {e}"))?;
        return Ok(Message::Status(report));
    }

    // Command response: {"r": {...}, "f":[...]}.
    if let Some(body) = obj.get("r") {
        let footer = obj.get("f").and_then(parse_footer);
        return Ok(Message::Response { body: body.clone(), footer });
    }

    Ok(Message::Other(value))
}

fn parse_footer(f: &Value) -> Option<Footer> {
    let arr = f.as_array()?;
    Some(Footer {
        revision: arr.first()?.as_i64()?,
        status_code: arr.get(1)?.as_i64()?,
    })
}

/// Pull a `StatusReport` out of a command-response body, for the case where the
/// host asked for status with `{"sr":null}` and g2core wrapped it as
/// `{"r":{"sr":{...}}}`. Returns None if the body has no `sr`.
pub fn status_from_response_body(body: &Value) -> Option<StatusReport> {
    let sr = body.as_object()?.get("sr")?;
    serde_json::from_value(sr.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures below are real lines captured from the arm's controller.

    #[test]
    fn get_builds_a_null_query() {
        assert_eq!(get("fv"), r#"{"fv":null}"#);
        assert_eq!(get("sr"), r#"{"sr":null}"#);
    }

    #[test]
    fn parses_firmware_version_response() {
        let m = parse_line(r#"{"r":{"fv":1.21},"f":[1,0,13]}"#).unwrap();
        match m {
            Message::Response { body, footer } => {
                assert_eq!(body["fv"].as_f64().unwrap(), 1.21);
                let f = footer.unwrap();
                assert_eq!(f.status_code, 0);
                assert!(f.is_ok());
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn parses_device_id_response() {
        let m = parse_line(r#"{"r":{"id":"0084-084a-0084-7bd"},"f":[1,0,13]}"#).unwrap();
        match m {
            Message::Response { body, .. } => {
                assert_eq!(body["id"].as_str().unwrap(), "0084-084a-0084-7bd");
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn parses_async_status_report() {
        let m = parse_line(r#"{"sr":{"stat":4}}"#).unwrap();
        match m {
            Message::Status(sr) => {
                assert_eq!(sr.stat, Some(4));
                assert_eq!(sr.posx, None); // partial report: only stat present
            }
            other => panic!("expected Status, got {other:?}"),
        }
    }

    #[test]
    fn parses_full_status_report_from_response_body() {
        // The real {"sr":null} response captured after a full home.
        let line = r#"{"r":{"sr":{"line":52,"posx":-8.2671,"posy":139.7344,"posz":912.8153,"posa":-154.78669,"posb":285.6647,"posc":0,"stat":4,"home":1,"homx":1,"homy":1,"homz":1,"homa":1,"homb":1,"vel":0,"qr":48}},"f":[1,0,13]}"#;
        let m = parse_line(line).unwrap();
        let body = match m {
            Message::Response { body, .. } => body,
            other => panic!("expected Response, got {other:?}"),
        };
        let sr = status_from_response_body(&body).expect("body should carry sr");
        assert_eq!(sr.home, Some(1));
        assert_eq!(sr.stat, Some(4));
        assert_eq!(sr.posx, Some(-8.2671));
        assert_eq!(sr.posy, Some(139.7344));
        assert_eq!(sr.qr, Some(48));
    }

    #[test]
    fn footer_status_code_flags_failure() {
        // status_code != 0 is a rejected command.
        let m = parse_line(r#"{"r":{"sr":{}},"f":[1,42,5]}"#).unwrap();
        match m {
            Message::Response { footer, .. } => {
                let f = footer.unwrap();
                assert_eq!(f.status_code, 42);
                assert!(!f.is_ok());
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse_line("not json at all").is_err());
        assert!(parse_line(r#"{"r":"#).is_err());
    }

    #[test]
    fn non_status_non_response_is_other() {
        let m = parse_line(r#"{"er":{"fb":101.03,"st":0}}"#).unwrap();
        assert!(matches!(m, Message::Other(_)));
    }
}
