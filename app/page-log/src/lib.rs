mod identifier;
mod journal;
mod page;
mod protocol;

pub use identifier::{CommandId, Identifier, ParticipantId, ShareId, StrokeId};
pub use journal::{JournalError, PageJournal, Submission};
pub use page::{
    ActiveStroke, AppliedPageOperation, BackgroundAsset, BackgroundEncoding, PageCommand,
    PageDimensions, PageIdentity, PageOperation, PageSnapshot, Participant, ParticipantRole,
    SharedStroke, StrokeReplacement, SubmittedPageOperation,
};
pub use protocol::{
    ClientMessage, PROTOCOL_VERSION, ProtocolError, ServerMessage, decode_client_message,
    decode_server_message, encode_client_message, encode_server_message, snapshot_digest,
};
