use super::access::{authenticate_participant, parse_share_id};
use super::{RelayError, RelayState};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const MAXIMUM_REQUEST_BYTES: usize = 16 * 1024;
const MAXIMUM_RECORDS: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PointerEventObservations {
    records: Vec<PointerEventObservation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PointerEventObservation {
    session: String,
    sequence: u32,
    client_unix_milliseconds: u64,
    hypothesis: String,
    event: String,
    action: String,
    pointer_type: String,
    pointer_id: Option<i32>,
    buttons: u16,
    captured: bool,
    active: bool,
    tool: String,
    stroke_started: bool,
    sample_count: usize,
    point_count: usize,
    socket_state: u16,
}

pub(super) async fn log_pointer_events(
    Path(share_id): Path<String>,
    State(state): State<RelayState>,
    headers: HeaderMap,
    Json(batch): Json<PointerEventObservations>,
) -> Result<StatusCode, RelayError> {
    let share_id = parse_share_id(&share_id)?;
    let share = state.share(share_id)?;
    let participant = authenticate_participant(share_id, &share, &headers)?;
    if batch.records.is_empty() || batch.records.len() > MAXIMUM_RECORDS {
        return Err(RelayError::bad_request(
            "invalid pointer debug record count",
        ));
    }
    for record in batch.records {
        if ![
            &record.session,
            &record.hypothesis,
            &record.event,
            &record.action,
            &record.pointer_type,
            &record.tool,
        ]
        .into_iter()
        .all(|value| contains_only_log_label_characters(value))
        {
            return Err(RelayError::bad_request("invalid pointer debug label"));
        }
        eprintln!(
            "page_relay_pointer_debug server_ms={} client_ms={} hypothesis={} session={} sequence={} share_id={} participant_id={} event={} action={} pointer_type={} pointer_id={:?} buttons={} captured={} active={} tool={} stroke_started={} sample_count={} point_count={} socket_state={}",
            unix_milliseconds(),
            record.client_unix_milliseconds,
            record.hypothesis,
            record.session,
            record.sequence,
            share_id,
            participant.id,
            record.event,
            record.action,
            record.pointer_type,
            record.pointer_id,
            record.buttons,
            record.captured,
            record.active,
            record.tool,
            record.stroke_started,
            record.sample_count,
            record.point_count,
            record.socket_state,
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn unix_milliseconds() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn contains_only_log_label_characters(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
