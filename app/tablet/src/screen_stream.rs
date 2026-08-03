use crate::display::QuillDisplay;
use crate::screen_stream_protocol::{encode_changed_pixels, encode_full_frame};
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

const LISTEN_ADDRESS: &str = "0.0.0.0:7432";
const STREAM_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone)]
struct ScreenStreamState {
    display: Arc<QuillDisplay>,
    streaming: Arc<AtomicBool>,
}

struct StreamingLease(Arc<AtomicBool>);

impl Drop for StreamingLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub fn start_screen_stream(display: Arc<QuillDisplay>) -> io::Result<thread::JoinHandle<()>> {
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
                        streaming: Arc::new(AtomicBool::new(false)),
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

async fn screen_viewer() -> Html<&'static str> {
    Html(include_str!("../assets/screen-viewer.html"))
}

async fn upgrade_websocket(
    websocket: WebSocketUpgrade,
    State(state): State<ScreenStreamState>,
) -> impl IntoResponse {
    websocket.on_upgrade(move |socket| stream_display_changes(socket, state))
}

async fn stream_display_changes(mut socket: WebSocket, state: ScreenStreamState) {
    if state.streaming.swap(true, Ordering::AcqRel) {
        return;
    }
    let _streaming_lease = StreamingLease(Arc::clone(&state.streaming));
    let mut previous = state.display.copy_snapshot();
    if socket
        .send(Message::Binary(
            encode_full_frame(previous.width, previous.height, &previous.pixels).into(),
        ))
        .await
        .is_err()
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
                    && socket.send(Message::Binary(message.into())).await.is_err()
                {
                    return;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}
