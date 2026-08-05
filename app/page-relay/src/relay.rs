use crate::persistence::{load_shares, read_asset, write_asset, write_share};
use crate::settings::RelaySettings;
use access::{
    authenticate_participant, blake3_digest, choose_guest_color, decode_hex, encode_hex,
    guest_session_protocol, parse_share_id, random_bytes, require_active, require_owner_token,
    unix_seconds,
};
use axum::body::Bytes;
use axum::extract::DefaultBodyLimit;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use constant_time_eq::constant_time_eq;
use futures_util::{SinkExt, StreamExt};
use remarque_core::color::Color;
use remarque_page_log::{
    ClientMessage, CommandId, PROTOCOL_VERSION, PageCommand, PageJournal, PageOperation,
    PageSnapshot, Participant, ParticipantId, ParticipantRole, ServerMessage, ShareId, StrokeId,
    SubmittedPageOperation, decode_client_message, encode_server_message, snapshot_digest,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tower_http::services::ServeDir;

const SHARE_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
const MAXIMUM_WEBSOCKET_MESSAGE_BYTES: usize = 128 * 1024;
const BROADCAST_CAPACITY: usize = 256;
const MAXIMUM_ASSET_BYTES: usize = 64 * 1024 * 1024;
const MAXIMUM_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
const MAXIMUM_PARTICIPANTS_PER_SHARE: usize = 64;
const MAXIMUM_MESSAGES_PER_SECOND: u32 = 240;
const GUEST_SESSION_PROTOCOL_PREFIX: &str = "remarque.session.";
const GUEST_COLORS: [Color; 8] = [
    Color::Blue,
    Color::Red,
    Color::Green,
    Color::Cyan,
    Color::Magenta,
    Color::Yellow,
    Color::Orange,
    Color::Gray,
];

mod access;

#[derive(Clone)]
struct RelayState {
    settings: RelaySettings,
    shares: Arc<RwLock<BTreeMap<ShareId, Arc<Share>>>>,
}

struct Share {
    stored: Mutex<StoredShare>,
    messages: broadcast::Sender<Vec<u8>>,
    access: watch::Sender<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredShare {
    pub id: ShareId,
    pub expires_at_unix_seconds: u64,
    guest_secret_digest: [u8; 32],
    owner: Participant,
    participants: Vec<Participant>,
    sessions: Vec<StoredSession>,
    journal: PageJournal,
    #[serde(default)]
    revoked: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredSession {
    token_digest: [u8; 32],
    participant_id: ParticipantId,
}

enum ActiveStrokeChange {
    Began(StrokeId),
    Finished(StrokeId),
}

#[derive(Deserialize)]
struct CreateShareRequest {
    snapshot: PageSnapshot,
}

#[derive(Serialize)]
struct CreateShareResponse {
    share_id: String,
    guest_url: String,
    owner_token: String,
    expires_at_unix_seconds: u64,
}

#[derive(Serialize)]
struct ShareStatusResponse {
    revision: u64,
    committed_strokes: usize,
    active_strokes: usize,
    revoked: bool,
    expires_at_unix_seconds: u64,
}

#[derive(Deserialize)]
struct RedeemShareRequest {
    secret: String,
    session_token: Option<String>,
}

#[derive(Serialize)]
struct RedeemShareResponse {
    participant: Participant,
    session_token: String,
    expires_at_unix_seconds: u64,
}

#[derive(Debug)]
struct RelayError {
    status: StatusCode,
    message: String,
}

impl RelayError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "authentication failed".to_owned(),
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "share was not found".to_owned(),
        }
    }

    fn too_many_requests(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for RelayError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

pub async fn serve(settings: RelaySettings) -> io::Result<()> {
    let state = RelayState::load(settings)?;
    let listener = tokio::net::TcpListener::bind(state.settings.listen_address).await?;
    axum::serve(listener, page_relay_router(state))
        .await
        .map_err(io::Error::other)
}

fn page_relay_router(state: RelayState) -> Router {
    Router::new()
        .route("/", get(viewer_page))
        .route("/share/{share_id}", get(viewer_page))
        .route("/viewer.js", get(viewer_script))
        .route("/health", get(health))
        .route(
            "/api/shares",
            post(create_share).layer(DefaultBodyLimit::max(MAXIMUM_SNAPSHOT_BYTES)),
        )
        .route(
            "/api/assets/{digest}",
            axum::routing::put(upload_asset).layer(DefaultBodyLimit::max(MAXIMUM_ASSET_BYTES)),
        )
        .route("/api/shares/{share_id}/redeem", post(redeem_share))
        .route("/api/shares/{share_id}/status", get(share_status))
        .route(
            "/api/shares/{share_id}/assets/{digest}",
            get(download_asset),
        )
        .route("/api/shares/{share_id}/ws", get(upgrade_websocket))
        .route("/api/shares/{share_id}", delete(revoke_share))
        .nest_service("/wasm", ServeDir::new(&state.settings.viewer_directory))
        .with_state(state)
}

impl RelayState {
    fn load(settings: RelaySettings) -> io::Result<Self> {
        let directory = settings.data_directory.join("shares");
        let mut shares = BTreeMap::new();
        for mut stored in load_shares(&directory)? {
            let id = stored.id;
            if let Err(error) = stored.journal.revalidate() {
                eprintln!("page_relay_share_validation_failed share_id={id} error={error}");
                continue;
            }
            match finalize_orphaned_strokes(&mut stored) {
                Ok(true) => {
                    if let Err(error) = write_share(&directory, &stored) {
                        eprintln!(
                            "page_relay_share_repair_persist_failed share_id={id} error={error}"
                        );
                        continue;
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    eprintln!("page_relay_share_repair_failed share_id={id} error={error}");
                    continue;
                }
            }
            let (messages, _) = broadcast::channel(BROADCAST_CAPACITY);
            let (access, _) = watch::channel(!stored.revoked);
            shares.insert(
                id,
                Arc::new(Share {
                    stored: Mutex::new(stored),
                    messages,
                    access,
                }),
            );
        }
        Ok(Self {
            settings,
            shares: Arc::new(RwLock::new(shares)),
        })
    }

    fn share(&self, id: ShareId) -> Result<Arc<Share>, RelayError> {
        self.shares
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .cloned()
            .ok_or_else(RelayError::not_found)
    }

    fn persist(&self, share: &StoredShare) -> Result<(), RelayError> {
        write_share(&self.settings.data_directory.join("shares"), share)
            .map_err(RelayError::internal)
    }
}

async fn health() -> &'static str {
    "ok\n"
}

async fn share_status(
    Path(share_id): Path<String>,
    State(state): State<RelayState>,
    headers: HeaderMap,
) -> Result<Json<ShareStatusResponse>, RelayError> {
    require_owner_token(&state.settings, &headers)?;
    let share = state.share(parse_share_id(&share_id)?)?;
    let stored = share
        .stored
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let snapshot = stored.journal.snapshot();
    Ok(Json(ShareStatusResponse {
        revision: snapshot.revision,
        committed_strokes: snapshot.strokes.len(),
        active_strokes: snapshot.active_strokes.len(),
        revoked: stored.revoked,
        expires_at_unix_seconds: stored.expires_at_unix_seconds,
    }))
}

async fn viewer_page(State(state): State<RelayState>) -> Result<Response, RelayError> {
    let page = read_viewer_text(&state, "index.html").await?;
    Ok((
        [
            (CONTENT_TYPE, "text/html; charset=utf-8"),
            (CACHE_CONTROL, "no-store"),
            (REFERRER_POLICY, "no-referrer"),
            (
                CONTENT_SECURITY_POLICY,
                "default-src 'self'; connect-src 'self'; img-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
            ),
        ],
        page,
    )
        .into_response())
}

async fn viewer_script(State(state): State<RelayState>) -> Result<Response, RelayError> {
    let script = read_viewer_text(&state, "viewer.js").await?;
    Ok((
        [
            (CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (CACHE_CONTROL, "no-store"),
            (REFERRER_POLICY, "no-referrer"),
        ],
        script,
    )
        .into_response())
}

async fn read_viewer_text(state: &RelayState, name: &str) -> Result<String, RelayError> {
    tokio::fs::read_to_string(state.settings.viewer_directory.join("current").join(name))
        .await
        .map_err(RelayError::internal)
}

async fn create_share(
    State(state): State<RelayState>,
    headers: HeaderMap,
    Json(request): Json<CreateShareRequest>,
) -> Result<Json<CreateShareResponse>, RelayError> {
    require_owner_token(&state.settings, &headers)?;
    let share_id = ShareId::from_bytes(random_bytes()?);
    let owner = Participant {
        id: ParticipantId::from_bytes(random_bytes()?),
        role: ParticipantRole::Owner,
        color: Color::Black,
    };
    let mut snapshot = request.snapshot;
    for stroke in &mut snapshot.strokes {
        stroke.author = owner.id;
    }
    for active in &mut snapshot.active_strokes {
        active.stroke.author = owner.id;
    }
    let journal = PageJournal::from_snapshot(snapshot)
        .map_err(|error| RelayError::bad_request(error.to_string()))?;
    if let Some(background) = &journal.snapshot().background {
        let bytes = read_asset(
            &state.settings.data_directory.join("assets"),
            &background.digest,
        )
        .map_err(|_| RelayError::bad_request("page background has not been uploaded"))?;
        let expected = usize::try_from(background.dimensions.width)
            .ok()
            .and_then(|width| {
                usize::try_from(background.dimensions.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4));
        if expected != Some(bytes.len()) {
            return Err(RelayError::bad_request(
                "page background byte count does not match its dimensions",
            ));
        }
    }
    let guest_secret = random_bytes::<32>()?;
    let owner_token = random_bytes::<32>()?;
    let expires_at_unix_seconds = unix_seconds()
        .checked_add(SHARE_LIFETIME.as_secs())
        .ok_or_else(|| RelayError::internal("share expiration overflow"))?;
    let stored = StoredShare {
        id: share_id,
        expires_at_unix_seconds,
        guest_secret_digest: blake3_digest(&guest_secret),
        owner,
        participants: vec![owner],
        sessions: vec![StoredSession {
            token_digest: blake3_digest(&owner_token),
            participant_id: owner.id,
        }],
        journal,
        revoked: false,
    };
    state.persist(&stored)?;
    let (messages, _) = broadcast::channel(BROADCAST_CAPACITY);
    let (access, _) = watch::channel(true);
    state
        .shares
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            share_id,
            Arc::new(Share {
                stored: Mutex::new(stored),
                messages,
                access,
            }),
        );
    Ok(Json(CreateShareResponse {
        share_id: share_id.to_string(),
        guest_url: format!(
            "{}/#{}.{}",
            state.settings.public_origin,
            share_id,
            encode_hex(&guest_secret)
        ),
        owner_token: encode_hex(&owner_token),
        expires_at_unix_seconds,
    }))
}

async fn upload_asset(
    Path(digest_text): Path<String>,
    State(state): State<RelayState>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<StatusCode, RelayError> {
    require_owner_token(&state.settings, &headers)?;
    let expected = decode_hex::<32>(&digest_text).map_err(RelayError::bad_request)?;
    if blake3_digest(&bytes) != expected {
        return Err(RelayError::bad_request(
            "asset digest does not match its content",
        ));
    }
    write_asset(
        &state.settings.data_directory.join("assets"),
        &expected,
        &bytes,
    )
    .map_err(RelayError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn download_asset(
    Path((share_id, digest_text)): Path<(String, String)>,
    State(state): State<RelayState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, RelayError> {
    let share_id = parse_share_id(&share_id)?;
    let share = state.share(share_id)?;
    authenticate_participant(share_id, &share, &headers)?;
    let requested = decode_hex::<32>(&digest_text).map_err(RelayError::bad_request)?;
    let expected = share
        .stored
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .journal
        .snapshot()
        .background
        .as_ref()
        .map(|background| background.digest)
        .ok_or_else(RelayError::not_found)?;
    if !constant_time_eq(&requested, &expected) {
        return Err(RelayError::not_found());
    }
    let bytes =
        read_asset(&state.settings.data_directory.join("assets"), &requested).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                RelayError::not_found()
            } else {
                RelayError::internal(error)
            }
        })?;
    Ok((
        [
            (CONTENT_TYPE, "application/x-remarque-bgra"),
            (CACHE_CONTROL, "private, max-age=86400, immutable"),
            (REFERRER_POLICY, "no-referrer"),
        ],
        bytes,
    ))
}

async fn redeem_share(
    Path(share_id): Path<String>,
    State(state): State<RelayState>,
    Json(request): Json<RedeemShareRequest>,
) -> Result<Json<RedeemShareResponse>, RelayError> {
    let share_id = parse_share_id(&share_id)?;
    let share = state.share(share_id)?;
    let secret = decode_hex::<32>(&request.secret).map_err(RelayError::bad_request)?;
    let mut stored = share
        .stored
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    require_active(&stored)?;
    if !constant_time_eq(&blake3_digest(&secret), &stored.guest_secret_digest) {
        return Err(RelayError::unauthorized());
    }
    if let Some(session_token) = request.session_token
        && let Ok(token) = decode_hex::<32>(&session_token)
        && let Some(session) = stored
            .sessions
            .iter()
            .find(|session| constant_time_eq(&session.token_digest, &blake3_digest(&token)))
        && let Some(participant) = stored
            .participants
            .iter()
            .copied()
            .find(|participant| participant.id == session.participant_id)
    {
        return Ok(Json(RedeemShareResponse {
            participant,
            session_token,
            expires_at_unix_seconds: stored.expires_at_unix_seconds,
        }));
    }
    if stored.participants.len() >= MAXIMUM_PARTICIPANTS_PER_SHARE {
        return Err(RelayError::too_many_requests(
            "share participant limit has been reached",
        ));
    }
    let participant = Participant {
        id: ParticipantId::from_bytes(random_bytes()?),
        role: ParticipantRole::Editor,
        color: choose_guest_color(&stored.participants)?,
    };
    let token = random_bytes::<32>()?;
    stored.participants.push(participant);
    stored.sessions.push(StoredSession {
        token_digest: blake3_digest(&token),
        participant_id: participant.id,
    });
    state.persist(&stored)?;
    let response = RedeemShareResponse {
        participant,
        session_token: encode_hex(&token),
        expires_at_unix_seconds: stored.expires_at_unix_seconds,
    };
    Ok(Json(response))
}

async fn revoke_share(
    Path(share_id): Path<String>,
    State(state): State<RelayState>,
    headers: HeaderMap,
) -> Result<StatusCode, RelayError> {
    require_owner_token(&state.settings, &headers)?;
    let share_id = parse_share_id(&share_id)?;
    let share = state.share(share_id)?;
    {
        let mut stored = share
            .stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        stored.revoked = true;
        state.persist(&stored)?;
    }
    let message = encode_server_message(&ServerMessage::Rejected {
        command: None,
        reason: "share was revoked".to_owned(),
    })
    .map_err(RelayError::internal)?;
    let _ = share.messages.send(message);
    share.access.send_replace(false);
    Ok(StatusCode::NO_CONTENT)
}

async fn upgrade_websocket(
    Path(share_id): Path<String>,
    State(state): State<RelayState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<impl IntoResponse, RelayError> {
    let share_id = parse_share_id(&share_id)?;
    let share = state.share(share_id)?;
    let guest_protocol = guest_session_protocol(&headers).map(str::to_owned);
    let participant = authenticate_participant(share_id, &share, &headers)?;
    let websocket = if let Some(protocol) = guest_protocol {
        websocket.protocols([protocol])
    } else {
        websocket
    };
    Ok(websocket
        .max_message_size(MAXIMUM_WEBSOCKET_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve_participant(socket, state, share, participant)))
}

async fn serve_participant(
    socket: WebSocket,
    state: RelayState,
    share: Arc<Share>,
    participant: Participant,
) {
    let mut access = share.access.subscribe();
    let mut expiration_check = tokio::time::interval(Duration::from_secs(30));
    let mut message_window_started = tokio::time::Instant::now();
    let mut messages_in_window = 0u32;
    let mut connection_strokes = BTreeSet::new();
    let (welcome, mut messages) = {
        let stored = share
            .stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if require_active(&stored).is_err() {
            return;
        }
        (
            ServerMessage::Welcome {
                protocol_version: PROTOCOL_VERSION,
                participant,
                snapshot: stored.journal.snapshot().clone(),
            },
            share.messages.subscribe(),
        )
    };
    let Ok(bytes) = encode_server_message(&welcome) else {
        return;
    };
    let (mut outgoing, mut incoming) = socket.split();
    if outgoing.send(Message::Binary(bytes.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            incoming_message = incoming.next() => {
                let Some(Ok(message)) = incoming_message else { break; };
                match message {
                    Message::Binary(bytes) => {
                        if message_window_started.elapsed() >= Duration::from_secs(1) {
                            message_window_started = tokio::time::Instant::now();
                            messages_in_window = 0;
                        }
                        messages_in_window += 1;
                        if messages_in_window > MAXIMUM_MESSAGES_PER_SECOND {
                            let _ = send_rejection(&mut outgoing, None, "message rate limit exceeded".to_owned()).await;
                            break;
                        }
                        match apply_client_message(&state, &share, participant, &mut outgoing, &bytes).await {
                            Ok(Some(ActiveStrokeChange::Began(stroke_id))) => {
                                connection_strokes.insert(stroke_id);
                            }
                            Ok(Some(ActiveStrokeChange::Finished(stroke_id))) => {
                                connection_strokes.remove(&stroke_id);
                            }
                            Ok(None) => {}
                            Err(()) => break,
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(bytes) => {
                        if outgoing.send(Message::Pong(bytes)).await.is_err() { break; }
                    }
                    Message::Text(_) | Message::Pong(_) => {}
                }
            }
            broadcast = messages.recv() => {
                match broadcast {
                    Ok(bytes) => {
                        if outgoing.send(Message::Binary(bytes.into())).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let snapshot = share.stored.lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .journal.snapshot().clone();
                        let Ok(bytes) = encode_server_message(&ServerMessage::Snapshot { snapshot }) else { break; };
                        if outgoing.send(Message::Binary(bytes.into())).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = access.changed() => break,
            _ = expiration_check.tick() => {
                let active = {
                    let stored = share.stored.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    require_active(&stored).is_ok()
                };
                if !active { break; }
            }
        }
    }
    if let Err(error) =
        finalize_connection_strokes(&state, &share, participant, &connection_strokes)
    {
        eprintln!("page_relay_disconnect_cleanup_failed={}", error.message);
    }
}

async fn apply_client_message(
    state: &RelayState,
    share: &Share,
    participant: Participant,
    outgoing: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    bytes: &[u8],
) -> Result<Option<ActiveStrokeChange>, ()> {
    let message = match decode_client_message(bytes) {
        Ok(message) => message,
        Err(error) => {
            send_rejection(outgoing, None, error.to_string()).await?;
            return Ok(None);
        }
    };
    match message {
        ClientMessage::Submit { command } => {
            let command_id = command.id;
            let durable = matches!(
                &command.operation,
                SubmittedPageOperation::CommitStroke { .. }
                    | SubmittedPageOperation::ReplaceStrokes { .. }
            );
            let result = {
                let mut stored = share
                    .stored
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Err(error) = require_active(&stored) {
                    Err(error.message)
                } else {
                    let previous = durable.then(|| stored.clone());
                    match stored.journal.submit(participant, command) {
                        Ok(submission) => {
                            if durable
                                && submission.newly_applied
                                && let Err(error) = state.persist(&stored)
                            {
                                *stored = previous.expect("durable submissions preserve state");
                                Err(error.message)
                            } else {
                                (|| -> Result<_, String> {
                                    let active_stroke_change = match &submission.operation.operation
                                    {
                                        PageOperation::BeginStroke { stroke } => {
                                            Some(ActiveStrokeChange::Began(stroke.id))
                                        }
                                        PageOperation::CommitStroke { stroke_id }
                                        | PageOperation::CancelStroke { stroke_id } => {
                                            Some(ActiveStrokeChange::Finished(*stroke_id))
                                        }
                                        PageOperation::AppendStrokePoints { .. }
                                        | PageOperation::ReplaceStrokes { .. } => None,
                                    };
                                    let bytes = encode_server_message(&ServerMessage::Applied {
                                        operation: submission.operation,
                                    })
                                    .map_err(|error| error.to_string())?;
                                    if submission.newly_applied {
                                        let _ = share.messages.send(bytes);
                                        if durable {
                                            let revision = stored.journal.snapshot().revision;
                                            let digest = snapshot_digest(stored.journal.snapshot());
                                            let bytes =
                                                encode_server_message(&ServerMessage::Digest {
                                                    revision,
                                                    digest,
                                                })
                                                .map_err(|error| error.to_string())?;
                                            let _ = share.messages.send(bytes);
                                        }
                                        Ok((active_stroke_change, None))
                                    } else {
                                        Ok((active_stroke_change, Some(bytes)))
                                    }
                                })()
                            }
                        }
                        Err(error) => Err(error.to_string()),
                    }
                }
            };
            let active_stroke_change = match result {
                Ok((active_stroke_change, duplicate_response)) => {
                    if let Some(bytes) = duplicate_response
                        && outgoing.send(Message::Binary(bytes.into())).await.is_err()
                    {
                        return Err(());
                    }
                    active_stroke_change
                }
                Err(reason) => {
                    send_rejection(outgoing, Some(command_id), reason).await?;
                    return Ok(None);
                }
            };
            return Ok(active_stroke_change);
        }
        ClientMessage::Acknowledge { .. } => {}
        ClientMessage::RequestSnapshot => {
            let snapshot = share
                .stored
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .journal
                .snapshot()
                .clone();
            let bytes =
                encode_server_message(&ServerMessage::Snapshot { snapshot }).map_err(|_| ())?;
            outgoing
                .send(Message::Binary(bytes.into()))
                .await
                .map_err(|_| ())?;
        }
    }
    Ok(None)
}

fn finalize_connection_strokes(
    state: &RelayState,
    share: &Share,
    participant: Participant,
    stroke_ids: &BTreeSet<StrokeId>,
) -> Result<(), RelayError> {
    if stroke_ids.is_empty() {
        return Ok(());
    }
    {
        let mut stored = share
            .stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        require_active(&stored)?;
        let mut updated = stored.clone();
        let mut operations = Vec::new();
        for stroke_id in stroke_ids {
            let Some(active) = updated
                .journal
                .snapshot()
                .active_strokes
                .iter()
                .find(|active| {
                    active.stroke.id == *stroke_id && active.stroke.author == participant.id
                })
            else {
                continue;
            };
            let operation = if active.stroke.points.is_empty() {
                SubmittedPageOperation::CancelStroke {
                    stroke_id: *stroke_id,
                }
            } else {
                SubmittedPageOperation::CommitStroke {
                    stroke_id: *stroke_id,
                }
            };
            let submission = updated
                .journal
                .submit(
                    participant,
                    PageCommand {
                        id: CommandId::from_bytes(random_bytes()?),
                        operation,
                    },
                )
                .map_err(RelayError::internal)?;
            operations.push(submission.operation);
        }
        if operations.is_empty() {
            return Ok(());
        }
        let applied_messages = operations
            .into_iter()
            .map(|operation| encode_server_message(&ServerMessage::Applied { operation }))
            .collect::<Result<Vec<_>, _>>()
            .map_err(RelayError::internal)?;
        let revision = updated.journal.snapshot().revision;
        let digest = snapshot_digest(updated.journal.snapshot());
        let digest_message = encode_server_message(&ServerMessage::Digest { revision, digest })
            .map_err(RelayError::internal)?;
        state.persist(&updated)?;
        *stored = updated;
        for bytes in applied_messages {
            let _ = share.messages.send(bytes);
        }
        let _ = share.messages.send(digest_message);
    }
    Ok(())
}

fn finalize_orphaned_strokes(stored: &mut StoredShare) -> io::Result<bool> {
    let orphaned = stored
        .journal
        .snapshot()
        .active_strokes
        .iter()
        .map(|active| {
            (
                active.stroke.id,
                active.stroke.author,
                active.stroke.points.is_empty(),
            )
        })
        .collect::<Vec<_>>();
    for (stroke_id, author, empty) in &orphaned {
        let participant = stored
            .participants
            .iter()
            .copied()
            .find(|participant| participant.id == *author)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "active stroke author is not a share participant",
                )
            })?;
        let mut command_id = [0; 16];
        getrandom::fill(&mut command_id).map_err(io::Error::other)?;
        stored
            .journal
            .submit(
                participant,
                PageCommand {
                    id: CommandId::from_bytes(command_id),
                    operation: if *empty {
                        SubmittedPageOperation::CancelStroke {
                            stroke_id: *stroke_id,
                        }
                    } else {
                        SubmittedPageOperation::CommitStroke {
                            stroke_id: *stroke_id,
                        }
                    },
                },
            )
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    }
    Ok(!orphaned.is_empty())
}

async fn send_rejection(
    outgoing: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    command: Option<remarque_page_log::CommandId>,
    reason: String,
) -> Result<(), ()> {
    let bytes =
        encode_server_message(&ServerMessage::Rejected { command, reason }).map_err(|_| ())?;
    outgoing
        .send(Message::Binary(bytes.into()))
        .await
        .map_err(|_| ())
}

#[cfg(test)]
include!("relay_tests.rs");
