use super::{GUEST_COLORS, GUEST_SESSION_PROTOCOL_PREFIX, RelayError, Share, StoredShare};
use crate::settings::RelaySettings;
use axum::http::header::{AUTHORIZATION, COOKIE, SEC_WEBSOCKET_PROTOCOL};
use axum::http::{HeaderMap, StatusCode};
use constant_time_eq::constant_time_eq;
use remarque_core::color::Color;
use remarque_page_log::{Participant, ShareId};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn authenticate_participant(
    share_id: ShareId,
    share: &Share,
    headers: &HeaderMap,
) -> Result<Participant, RelayError> {
    let token = bearer_token(headers)
        .map(str::to_owned)
        .or_else(|| {
            guest_session_protocol(headers)
                .and_then(|protocol| protocol.strip_prefix(GUEST_SESSION_PROTOCOL_PREFIX))
                .map(str::to_owned)
        })
        .or_else(|| cookie_value(headers, &session_cookie_name(share_id)))
        .ok_or_else(RelayError::unauthorized)?;
    let token = decode_hex::<32>(&token).map_err(|_| RelayError::unauthorized())?;
    let stored = share
        .stored
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    require_active(&stored)?;
    let session = stored
        .sessions
        .iter()
        .find(|session| constant_time_eq(&session.token_digest, &blake3_digest(&token)))
        .ok_or_else(RelayError::unauthorized)?;
    stored
        .participants
        .iter()
        .copied()
        .find(|participant| participant.id == session.participant_id)
        .ok_or_else(RelayError::unauthorized)
}

pub(super) fn require_owner_token(
    settings: &RelaySettings,
    headers: &HeaderMap,
) -> Result<(), RelayError> {
    let token = bearer_token(headers).ok_or_else(RelayError::unauthorized)?;
    if constant_time_eq(token.as_bytes(), settings.owner_token.as_bytes()) {
        Ok(())
    } else {
        Err(RelayError::unauthorized())
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

pub(super) fn guest_session_protocol(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|header| header.split(','))
        .map(str::trim)
        .find(|protocol| protocol.starts_with(GUEST_SESSION_PROTOCOL_PREFIX))
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(COOKIE).iter().find_map(|header| {
        header.to_str().ok()?.split(';').find_map(|cookie| {
            let (cookie_name, value) = cookie.trim().split_once('=')?;
            (cookie_name == name).then(|| value.to_owned())
        })
    })
}

pub(super) fn require_active(share: &StoredShare) -> Result<(), RelayError> {
    if share.revoked {
        Err(RelayError {
            status: StatusCode::GONE,
            message: "share was revoked".to_owned(),
        })
    } else if share.expires_at_unix_seconds <= unix_seconds() {
        Err(RelayError {
            status: StatusCode::GONE,
            message: "share has expired".to_owned(),
        })
    } else {
        Ok(())
    }
}

pub(super) fn choose_guest_color(participants: &[Participant]) -> Result<Color, RelayError> {
    let unused = GUEST_COLORS
        .into_iter()
        .filter(|color| {
            !participants
                .iter()
                .any(|participant| participant.color == *color)
        })
        .collect::<Vec<_>>();
    let candidates = if unused.is_empty() {
        GUEST_COLORS.as_slice()
    } else {
        unused.as_slice()
    };
    let random = u64::from_le_bytes(random_bytes()?);
    Ok(candidates[random as usize % candidates.len()])
}

pub(super) fn parse_share_id(text: &str) -> Result<ShareId, RelayError> {
    ShareId::from_str(text).map_err(RelayError::bad_request)
}

fn session_cookie_name(share_id: ShareId) -> String {
    format!("remarque_{share_id}")
}

pub(super) fn random_bytes<const LENGTH: usize>() -> Result<[u8; LENGTH], RelayError> {
    let mut bytes = [0; LENGTH];
    getrandom::fill(&mut bytes).map_err(RelayError::internal)?;
    Ok(bytes)
}

pub(super) fn blake3_digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

pub(super) fn encode_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(text, "{byte:02x}").expect("writing to a string cannot fail");
    }
    text
}

pub(super) fn decode_hex<const LENGTH: usize>(text: &str) -> Result<[u8; LENGTH], &'static str> {
    if text.len() != LENGTH * 2 {
        return Err("secret has the wrong length");
    }
    let mut bytes = [0; LENGTH];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&text[start..start + 2], 16)
            .map_err(|_| "secret is not hexadecimal")?;
    }
    Ok(bytes)
}

pub(super) fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
