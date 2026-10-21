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
            Command::Resize { width: 900, height: 600 }
        );
    }

    #[test]
    fn render_errors_are_machine_readable() {
        let value = serde_json::to_string(&Response::Error {
            message: "load failed".into()
        }).unwrap();
        assert!(value.contains(r#""kind":"error""#));
    }
}
