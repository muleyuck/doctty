//! Versioned, renderer-independent JSON contract.
//! The legacy Swift protocol remains authoritative until full parity is proven.
use serde::{Deserialize, Serialize};

// Staged for the next milestone: this bootstrap does not yet consume stdin.
#[allow(dead_code)]
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[allow(dead_code)]
pub enum Command {
    Dimensions,
    Resize { width: u32, height: u32 },
    Snapshot { y: f64 },
    Outline,
    Select,
    Metrics,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Response {
    Ready { height: f64 },
    Dimensions { height: f64 },
    Resized { height: f64 },
    Outline {
        background: String,
        items: Vec<serde_json::Value>,
    },
    Selection {
        text: String,
    },
    Metrics {
        requests: u64,
        completed: u64,
        #[serde(rename = "uptimeSec")]
        uptime_sec: f64,
        #[serde(rename = "secondsSinceSnapshot")]
        seconds_since_snapshot: f64,
    },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_legacy_swift_commands() {
        assert_eq!(
            serde_json::from_str::<Command>(r#"{"type":"snapshot","y":25.5}"#).unwrap(),
            Command::Snapshot { y: 25.5 }
        );
        assert_eq!(
            serde_json::from_str::<Command>(r#"{"type":"resize","width":900,"height":600}"#)
                .unwrap(),
            Command::Resize {
                width: 900,
                height: 600
            }
        );
    }

    #[test]
    fn deserialize_line_protocol_commands() {
        for (json, expected) in [
            (r#"{"type":"dimensions"}"#, Command::Dimensions),
            (r#"{"type":"outline"}"#, Command::Outline),
            (r#"{"type":"select"}"#, Command::Select),
            (r#"{"type":"metrics"}"#, Command::Metrics),
            (r#"{"type":"quit"}"#, Command::Quit),
        ] {
            assert_eq!(serde_json::from_str::<Command>(json).unwrap(), expected);
        }
        assert!(serde_json::from_str::<Command>(r#"{"type":"bogus"}"#).is_err());
    }

    #[test]
    fn outline_and_metrics_match_swift_json_shapes() {
        let outline = serde_json::to_value(Response::Outline {
            background: "#ffffff".into(),
            items: vec![serde_json::json!({"y": 42, "text": "Title"})],
        })
        .unwrap();
        assert_eq!(outline["kind"], "outline");
        assert_eq!(outline["items"][0]["y"], 42);
        assert_eq!(outline["background"], "#ffffff");

        let metrics = serde_json::to_value(Response::Metrics {
            requests: 0,
            completed: 0,
            uptime_sec: 3.5,
            seconds_since_snapshot: -1.0,
        })
        .unwrap();
        assert_eq!(metrics["kind"], "metrics");
        assert_eq!(metrics["uptimeSec"], 3.5);
        assert_eq!(metrics["secondsSinceSnapshot"], -1.0);
        assert!(metrics.get("uptime_sec").is_none());
    }

    #[test]
    fn render_errors_are_machine_readable() {
        let value = serde_json::to_string(&Response::Error {
            message: "load failed".into(),
        })
        .unwrap();
        assert!(value.contains(r#""kind":"error""#));
    }
}
