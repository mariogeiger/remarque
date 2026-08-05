use crate::{AppliedPageOperation, PageCommand, PageSnapshot, Participant};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const PROTOCOL_VERSION: u16 = 4;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientMessage {
    Submit { command: PageCommand },
    Acknowledge { revision: u64 },
    RequestSnapshot,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        protocol_version: u16,
        participant: Participant,
        snapshot: PageSnapshot,
    },
    Applied {
        operation: AppliedPageOperation,
    },
    Snapshot {
        snapshot: PageSnapshot,
    },
    Digest {
        revision: u64,
        digest: [u8; 32],
    },
    Rejected {
        command: Option<crate::CommandId>,
        reason: String,
    },
}

#[derive(Debug)]
pub struct ProtocolError(String);

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProtocolError {}

pub fn encode_client_message(message: &ClientMessage) -> Result<Vec<u8>, ProtocolError> {
    postcard::to_allocvec(message).map_err(|error| ProtocolError(error.to_string()))
}

pub fn decode_client_message(bytes: &[u8]) -> Result<ClientMessage, ProtocolError> {
    postcard::from_bytes(bytes).map_err(|error| ProtocolError(error.to_string()))
}

pub fn encode_server_message(message: &ServerMessage) -> Result<Vec<u8>, ProtocolError> {
    postcard::to_allocvec(message).map_err(|error| ProtocolError(error.to_string()))
}

pub fn decode_server_message(bytes: &[u8]) -> Result<ServerMessage, ProtocolError> {
    postcard::from_bytes(bytes).map_err(|error| ProtocolError(error.to_string()))
}

pub fn snapshot_digest(snapshot: &PageSnapshot) -> [u8; 32] {
    let bytes = serde_json::to_vec(snapshot).expect("page snapshots are JSON serializable");
    *blake3::hash(&bytes).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PageDimensions, PageIdentity};

    #[test]
    fn snapshot_message_round_trips_as_binary() {
        let message = ServerMessage::Snapshot {
            snapshot: PageSnapshot {
                identity: PageIdentity {
                    document_id: "notebook-1".to_owned(),
                    page_index: 0,
                },
                dimensions: PageDimensions {
                    width: 1620,
                    height: 2076,
                },
                background: None,
                strokes: Vec::new(),
                active_strokes: Vec::new(),
                revision: 7,
            },
        };
        let bytes = encode_server_message(&message).unwrap();
        assert_eq!(decode_server_message(&bytes).unwrap(), message);
    }
}
