use crate::{CommandId, ParticipantId, StrokeId};
use remarque_core::color::Color;
use remarque_core::stroke::StrokePoint;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PageIdentity {
    pub document_id: String,
    pub page_index: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PageDimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundEncoding {
    Bgra8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackgroundAsset {
    pub digest: [u8; 32],
    pub dimensions: PageDimensions,
    pub encoding: BackgroundEncoding,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantRole {
    Owner,
    Editor,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Participant {
    pub id: ParticipantId,
    pub role: ParticipantRole,
    pub color: Color,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SharedStroke {
    pub id: StrokeId,
    pub author: ParticipantId,
    pub color: Color,
    pub points: Vec<StrokePoint>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ActiveStroke {
    pub stroke: SharedStroke,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PageSnapshot {
    pub identity: PageIdentity,
    pub dimensions: PageDimensions,
    pub background: Option<BackgroundAsset>,
    pub strokes: Vec<SharedStroke>,
    pub active_strokes: Vec<ActiveStroke>,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StrokeReplacement {
    pub removed: StrokeId,
    pub fragments: Vec<SharedStroke>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmittedPageOperation {
    BeginStroke {
        stroke_id: StrokeId,
    },
    AppendStrokePoints {
        stroke_id: StrokeId,
        first_point: u32,
        points: Vec<StrokePoint>,
    },
    CommitStroke {
        stroke_id: StrokeId,
    },
    CancelStroke {
        stroke_id: StrokeId,
    },
    ReplaceStrokes {
        replacements: Vec<StrokeReplacement>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PageCommand {
    pub id: CommandId,
    pub operation: SubmittedPageOperation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageOperation {
    BeginStroke {
        stroke: SharedStroke,
    },
    AppendStrokePoints {
        stroke_id: StrokeId,
        first_point: u32,
        points: Vec<StrokePoint>,
    },
    CommitStroke {
        stroke_id: StrokeId,
    },
    CancelStroke {
        stroke_id: StrokeId,
    },
    ReplaceStrokes {
        replacements: Vec<StrokeReplacement>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AppliedPageOperation {
    pub revision: u64,
    pub command_id: CommandId,
    pub actor: ParticipantId,
    pub operation: PageOperation,
}
