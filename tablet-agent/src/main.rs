use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const SUPPORTED_FIRMWARE: &str = "3.27.3.0";
const DISPLAY_WIDTH: usize = 1620;
const DISPLAY_HEIGHT: usize = 2160;
const DISPLAY_STRIDE: usize = 6528;
const FRAME_SIZE: usize = DISPLAY_STRIDE * DISPLAY_HEIGHT;
const LISTEN_ADDRESS: &str = "0.0.0.0:7432";
const ACTIVE_INTERVAL: Duration = Duration::from_millis(100);
const ACTIVE_TAIL: Duration = Duration::from_millis(800);
const IDLE_INTERVAL: Duration = Duration::from_secs(5);
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TILE_SIZE: usize = 64;
const INPUT_DEVICES: [&str; 2] = ["/dev/input/event2", "/dev/input/event3"];
const PROTOCOL_MAGIC: &[u8; 4] = b"RMKS";
const PROTOCOL_VERSION: u8 = 2;
const MESSAGE_FULL_FRAME: u8 = 1;
const MESSAGE_DELTA_FRAME: u8 = 2;
const HEADER_SIZE: usize = 16;

// goMarkableStream uses this slightly smaller size to identify the allocation.
const FRAME_ALLOCATION_THRESHOLD: u32 = 1632 * 2154 * 4;
const MAX_ALLOCATION_HEADER_STEPS: usize = 4096;
const MAX_REASONABLE_ALLOCATION: u32 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
struct Framebuffer {
    pid: u32,
    address: u64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = start().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn start() -> io::Result<()> {
    let firmware = read_firmware_version()?;
    if firmware != SUPPORTED_FIRMWARE {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("firmware {firmware} is not supported"),
        ));
    }

    serve(locate_framebuffer()?).await
}

fn read_firmware_version() -> io::Result<String> {
    let os_release = fs::read_to_string("/etc/os-release")?;
    parse_firmware_version(&os_release).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "IMG_VERSION is missing from /etc/os-release",
        )
    })
}

fn parse_firmware_version(os_release: &str) -> Option<String> {
    os_release.lines().find_map(|line| {
        line.strip_prefix("IMG_VERSION=")
            .map(|value| value.trim_matches('"').to_owned())
    })
}

fn locate_framebuffer() -> io::Result<Framebuffer> {
    let pid = find_xochitl()?;
    let maps = fs::read_to_string(format!("/proc/{pid}/maps"))?;
    let drm_end = last_drm_mapping_end(&maps).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "xochitl has no /dev/dri/card0 mapping",
        )
    })?;

    let memory = File::open(format!("/proc/{pid}/mem"))?;
    let (address, allocation_size) = follow_allocation_headers(&memory, drm_end)?;

    if allocation_size < FRAME_SIZE as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "candidate allocation is {allocation_size} bytes, expected at least {FRAME_SIZE}"
            ),
        ));
    }

    Ok(Framebuffer { pid, address })
}

fn find_xochitl() -> io::Result<u32> {
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };

        let comm_path = entry.path().join("comm");
        if fs::read_to_string(comm_path)
            .map(|comm| comm.trim() == "xochitl")
            .unwrap_or(false)
        {
            return Ok(pid);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "xochitl was not found",
    ))
}

fn last_drm_mapping_end(maps: &str) -> Option<u64> {
    maps.lines()
        .filter(|line| line.contains("/dev/dri/card0"))
        .fold(None, |_, line| {
            line.split_whitespace()
                .next()
                .and_then(|range| range.split_once('-'))
                .and_then(|(_, end)| u64::from_str_radix(end, 16).ok())
        })
}

fn follow_allocation_headers(memory: &File, start: u64) -> io::Result<(u64, u32)> {
    let mut offset = 0_u64;
    let mut allocation_size = 2_u32;

    for _ in 0..MAX_ALLOCATION_HEADER_STEPS {
        if allocation_size >= FRAME_ALLOCATION_THRESHOLD {
            return Ok((start + offset, allocation_size));
        }

        if !(2..=MAX_REASONABLE_ALLOCATION).contains(&allocation_size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid allocation header size {allocation_size}"),
            ));
        }

        offset = offset
            .checked_add(u64::from(allocation_size - 2))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "allocation offset overflow")
            })?;

        let mut header = [0_u8; 4];
        memory.read_exact_at(&mut header, start + offset + 8)?;
        allocation_size = u32::from_le_bytes(header);
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "framebuffer allocation was not found within the step limit",
    ))
}

impl Framebuffer {
    fn open(&self) -> io::Result<File> {
        File::open(format!("/proc/{}/mem", self.pid))
    }

    fn read(&self, memory: &File, destination: &mut [u8]) -> io::Result<()> {
        memory.read_exact_at(destination, self.address)
    }
}

#[derive(Clone)]
struct AppState {
    framebuffer: Framebuffer,
    streaming: Arc<AtomicBool>,
}

async fn serve(framebuffer: Framebuffer) -> io::Result<()> {
    let listener = tokio::net::TcpListener::bind(LISTEN_ADDRESS).await?;
    let app = Router::new()
        .route("/", get(index))
        .route("/ws/2", get(websocket))
        .with_state(AppState {
            framebuffer,
            streaming: Arc::new(AtomicBool::new(false)),
        });

    println!("listening={LISTEN_ADDRESS}");
    axum::serve(listener, app).await.map_err(io::Error::other)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("viewer.html"))
}

async fn websocket(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}

async fn handle_websocket(mut socket: WebSocket, state: AppState) {
    if state.streaming.swap(true, Ordering::AcqRel) {
        return;
    }

    let (message_tx, mut message_rx) = mpsc::channel::<Vec<u8>>(1);
    let streaming = state.streaming;
    std::thread::spawn(move || {
        if let Err(error) = produce_messages(&state.framebuffer, message_tx) {
            eprintln!("capture_stopped={error}");
        }
        streaming.store(false, Ordering::Release);
    });

    loop {
        tokio::select! {
            message = message_rx.recv() => {
                let Some(message) = message else {
                    break;
                };
                if socket.send(Message::Binary(message.into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

fn produce_messages(
    framebuffer: &Framebuffer,
    message_tx: mpsc::Sender<Vec<u8>>,
) -> io::Result<()> {
    let memory = framebuffer.open()?;
    let mut inputs = open_input_devices()?;
    let mut previous = vec![0_u8; FRAME_SIZE];
    let mut current = vec![0_u8; FRAME_SIZE];
    let mut active_until = Instant::now();
    let mut last_capture = Instant::now();

    framebuffer.read(&memory, &mut previous)?;
    send_message(
        &message_tx,
        encode_message(MESSAGE_FULL_FRAME, 0, &previous),
    )?;

    loop {
        if message_tx.is_closed() {
            return Ok(());
        }

        let now = Instant::now();
        if input_activity(&mut inputs)? {
            active_until = now + ACTIVE_TAIL;
        }

        let interval = if now < active_until {
            ACTIVE_INTERVAL
        } else {
            IDLE_INTERVAL
        };
        if last_capture.elapsed() < interval {
            std::thread::sleep(INPUT_POLL_INTERVAL);
            continue;
        }

        last_capture = Instant::now();
        framebuffer.read(&memory, &mut current)?;

        let (delta, tile_count) = build_delta(&previous, &current);

        if tile_count > 0 {
            if delta.len() > FRAME_SIZE / 2 {
                send_message(&message_tx, encode_message(MESSAGE_FULL_FRAME, 0, &current))?;
            } else {
                send_message(
                    &message_tx,
                    encode_message(MESSAGE_DELTA_FRAME, tile_count, &delta),
                )?;
            }
        }

        std::mem::swap(&mut previous, &mut current);
    }
}

fn open_input_devices() -> io::Result<[File; 2]> {
    let open = |path| {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
    };
    Ok([open(INPUT_DEVICES[0])?, open(INPUT_DEVICES[1])?])
}

fn input_activity(devices: &mut [File; 2]) -> io::Result<bool> {
    let mut buffer = [0_u8; 512];
    let mut active = false;

    for (index, device) in devices.iter_mut().enumerate() {
        loop {
            match device.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => {
                    active |= buffer[..size].chunks_exact(24).any(|event| {
                        let event_type = u16::from_ne_bytes([event[16], event[17]]);
                        let code = u16::from_ne_bytes([event[18], event[19]]);
                        let value =
                            i32::from_ne_bytes([event[20], event[21], event[22], event[23]]);
                        if index == 0 {
                            (event_type == 3 && code == 24 && value > 0)
                                || (event_type == 1 && code == 330)
                        } else {
                            event_type == 1 || event_type == 3
                        }
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }

    Ok(active)
}

fn encode_message(message_type: u8, tile_count: u32, payload: &[u8]) -> Vec<u8> {
    let header = encode_header(message_type, tile_count, payload.len());
    let mut message = Vec::with_capacity(HEADER_SIZE + payload.len());
    message.extend_from_slice(&header);
    message.extend_from_slice(payload);
    message
}

fn send_message(message_tx: &mpsc::Sender<Vec<u8>>, message: Vec<u8>) -> io::Result<()> {
    message_tx
        .blocking_send(message)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "websocket disconnected"))
}

fn encode_header(message_type: u8, tile_count: u32, payload_size: usize) -> [u8; HEADER_SIZE] {
    let mut header = [0_u8; HEADER_SIZE];
    header[0..4].copy_from_slice(PROTOCOL_MAGIC);
    header[4] = PROTOCOL_VERSION;
    header[5] = message_type;
    header[8..12].copy_from_slice(&(payload_size as u32).to_le_bytes());
    header[12..16].copy_from_slice(&tile_count.to_le_bytes());
    header
}

fn build_delta(previous: &[u8], current: &[u8]) -> (Vec<u8>, u32) {
    let mut payload = Vec::new();
    let mut tile_count = 0_u32;

    for y in (0..DISPLAY_HEIGHT).step_by(TILE_SIZE) {
        let tile_height = TILE_SIZE.min(DISPLAY_HEIGHT - y);
        for x in (0..DISPLAY_WIDTH).step_by(TILE_SIZE) {
            let tile_width = TILE_SIZE.min(DISPLAY_WIDTH - x);
            let row_bytes = tile_width * 4;
            let changed = (0..tile_height).any(|row| {
                let offset = (y + row) * DISPLAY_STRIDE + x * 4;
                previous[offset..offset + row_bytes] != current[offset..offset + row_bytes]
            });

            if !changed {
                continue;
            }

            payload.extend_from_slice(&(x as u16).to_le_bytes());
            payload.extend_from_slice(&(y as u16).to_le_bytes());
            payload.extend_from_slice(&(tile_width as u16).to_le_bytes());
            payload.extend_from_slice(&(tile_height as u16).to_le_bytes());

            for row in 0..tile_height {
                let offset = (y + row) * DISPLAY_STRIDE + x * 4;
                payload.extend_from_slice(&current[offset..offset + row_bytes]);
            }
            tile_count += 1;
        }
    }

    (payload, tile_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_firmware() {
        let input = "ID=codex\nIMG_VERSION=\"3.27.3.0\"\n";
        assert_eq!(parse_firmware_version(input).as_deref(), Some("3.27.3.0"));
    }

    #[test]
    fn takes_end_of_last_matching_mapping() {
        let maps = concat!(
            "1000-2000 rw-s 00000000 00:06 1 /dev/dri/card0\n",
            "2000-3000 rw-p 00000000 00:00 0\n",
            "4000-5abc rw-s 00000000 00:06 1 /dev/dri/card0\n",
        );
        assert_eq!(last_drm_mapping_end(maps), Some(0x5abc));
    }

    #[test]
    fn serializes_protocol_header() {
        let header = encode_header(MESSAGE_DELTA_FRAME, 3, 99);
        assert_eq!(&header[0..4], b"RMKS");
        assert_eq!(header[4], 2);
        assert_eq!(header[5], MESSAGE_DELTA_FRAME);
        assert_eq!(u32::from_le_bytes(header[8..12].try_into().unwrap()), 99);
        assert_eq!(u32::from_le_bytes(header[12..16].try_into().unwrap()), 3);
    }

    #[test]
    fn sends_only_changed_tiles() {
        let previous = vec![0_u8; FRAME_SIZE];
        let mut current = previous.clone();
        current[2 * DISPLAY_STRIDE + 3 * 4] = 255;

        let (delta, tile_count) = build_delta(&previous, &current);
        assert_eq!(tile_count, 1);
        assert_eq!(&delta[0..8], &[0, 0, 0, 0, 64, 0, 64, 0]);
        assert_eq!(delta.len(), 8 + 64 * 64 * 4);
    }
}
