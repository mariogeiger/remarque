use crate::display::EpaperDisplay;
use crate::screen_stream_protocol::{encode_changed_pixels, encode_full_frame};
use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::header::CACHE_CONTROL;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use serde::Deserialize;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

const LISTEN_ADDRESS: &str = "0.0.0.0:7432";
const STREAM_INTERVAL: Duration = Duration::from_millis(100);
const VIEWER_REPLACED_CLOSE_CODE: u16 = 4000;

#[derive(Clone)]
struct ScreenStreamState {
    display: Arc<EpaperDisplay>,
    connection_generation: watch::Sender<u64>,
    viewer_session: u64,
    next_viewer_generation: Arc<AtomicU64>,
    active_viewer_generation: Arc<AtomicU64>,
}

#[derive(Deserialize)]
struct ViewerQuery {
    session: u64,
    viewer: u64,
}

pub fn start_screen_stream(display: Arc<EpaperDisplay>) -> io::Result<thread::JoinHandle<()>> {
    let listener = std::net::TcpListener::bind(LISTEN_ADDRESS)?;
    listener.set_nonblocking(true)?;
    thread::Builder::new()
        .name("screen-stream".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("screen_stream_stopped={error}");
                    return;
                }
            };
            let result = runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener)?;
                let router = Router::new()
                    .route("/", get(screen_viewer))
                    .route("/ws/3", get(upgrade_websocket))
                    .with_state(ScreenStreamState {
                        display,
                        connection_generation: watch::channel(0).0,
                        viewer_session: current_time_nanoseconds(),
                        next_viewer_generation: Arc::new(AtomicU64::new(0)),
                        active_viewer_generation: Arc::new(AtomicU64::new(0)),
                    });
                axum::serve(listener, router)
                    .await
                    .map_err(io::Error::other)
            });
            if let Err(error) = result {
                eprintln!("screen_stream_stopped={error}");
            }
        })
        .map_err(io::Error::other)
}

async fn screen_viewer(State(state): State<ScreenStreamState>) -> impl IntoResponse {
    let viewer_generation = state
        .next_viewer_generation
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    let html = include_str!("../assets/screen-viewer.html")
        .replace(
            "__REMARQUE_VIEWER_SESSION__",
            &state.viewer_session.to_string(),
        )
        .replace(
            "__REMARQUE_VIEWER_GENERATION__",
            &viewer_generation.to_string(),
        );
    ([(CACHE_CONTROL, "no-store")], Html(html))
}

async fn upgrade_websocket(
    websocket: WebSocketUpgrade,
    Query(query): Query<ViewerQuery>,
    State(state): State<ScreenStreamState>,
) -> impl IntoResponse {
    websocket.on_upgrade(move |socket| stream_display_changes(socket, state, query))
}

async fn stream_display_changes(
    mut socket: WebSocket,
    state: ScreenStreamState,
    viewer: ViewerQuery,
) {
    if viewer.session != state.viewer_session
        || !claim_viewer_generation(&state.active_viewer_generation, viewer.viewer)
    {
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: VIEWER_REPLACED_CLOSE_CODE,
                reason: "replaced by a newer viewer".into(),
            })))
            .await;
        return;
    }
    let generation = (*state.connection_generation.borrow()).wrapping_add(1);
    state.connection_generation.send_replace(generation);
    let mut connection_generation = state.connection_generation.subscribe();
    let mut previous = state.display.copy_snapshot();
    if !send_until_replaced(
        &mut socket,
        Message::Binary(
            encode_full_frame(previous.width, previous.height, &previous.pixels).into(),
        ),
        &mut connection_generation,
    )
    .await
    {
        return;
    }

    let mut interval = tokio::time::interval(STREAM_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if state.display.generation() == previous.generation {
                    continue;
                }
                let current = state.display.copy_snapshot();
                let message = if previous.width == current.width && previous.height == current.height {
                    encode_changed_pixels(
                        &previous.pixels,
                        &current.pixels,
                        current.width,
                        current.height,
                    )
                } else {
                    Some(encode_full_frame(current.width, current.height, &current.pixels))
                };
                previous = current;
                if let Some(message) = message
                    && !send_until_replaced(
                        &mut socket,
                        Message::Binary(message.into()),
                        &mut connection_generation,
                    ).await
                {
                    return;
                }
            }
            _ = connection_generation.changed() => return,
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn send_until_replaced(
    socket: &mut WebSocket,
    message: Message,
    connection_generation: &mut watch::Receiver<u64>,
) -> bool {
    tokio::select! {
        result = socket.send(message) => result.is_ok(),
        _ = connection_generation.changed() => false,
    }
}

fn claim_viewer_generation(active: &AtomicU64, candidate: u64) -> bool {
    let mut current = active.load(Ordering::Acquire);
    loop {
        if candidate < current {
            return false;
        }
        if candidate == current {
            return true;
        }
        match active.compare_exchange_weak(current, candidate, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

fn current_time_nanoseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
