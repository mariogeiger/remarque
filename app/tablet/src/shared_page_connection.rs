use futures_util::{SinkExt, StreamExt};
use remarque_page_log::{
    ClientMessage, ServerMessage, decode_server_message, encode_client_message,
};
use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

const MAXIMUM_RECONNECT_DELAY: Duration = Duration::from_secs(30);

pub enum SharedPageEvent {
    Connected,
    Disconnected(String),
    Message(ServerMessage),
}

pub struct SharedPageConnection {
    outgoing: UnboundedSender<ClientMessage>,
    incoming: mpsc::Receiver<SharedPageEvent>,
}

impl SharedPageConnection {
    pub fn connect(websocket_url: String, owner_token: String) -> io::Result<Self> {
        let (outgoing, outgoing_receiver) = tokio::sync::mpsc::unbounded_channel();
        let (incoming_sender, incoming) = mpsc::channel();
        thread::Builder::new()
            .name("shared-page-connection".to_owned())
            .spawn(move || {
                run_shared_page_connection_thread(
                    websocket_url,
                    owner_token,
                    outgoing_receiver,
                    incoming_sender,
                )
            })?;
        Ok(Self { outgoing, incoming })
    }

    pub fn send(&self, message: ClientMessage) -> io::Result<()> {
        self.outgoing.send(message).map_err(|_| {
            io::Error::new(io::ErrorKind::BrokenPipe, "shared page connection stopped")
        })
    }

    pub fn drain(&self) -> Vec<SharedPageEvent> {
        self.incoming.try_iter().collect()
    }
}

fn run_shared_page_connection_thread(
    websocket_url: String,
    owner_token: String,
    outgoing: UnboundedReceiver<ClientMessage>,
    incoming: mpsc::Sender<SharedPageEvent>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = incoming.send(SharedPageEvent::Disconnected(error.to_string()));
            return;
        }
    };
    runtime.block_on(maintain_shared_page_connection(
        websocket_url,
        owner_token,
        outgoing,
        incoming,
    ));
}

async fn maintain_shared_page_connection(
    websocket_url: String,
    owner_token: String,
    mut outgoing: UnboundedReceiver<ClientMessage>,
    incoming: mpsc::Sender<SharedPageEvent>,
) {
    let mut retry_delay = Duration::from_secs(1);
    loop {
        let result = exchange_messages_until_disconnected(
            &websocket_url,
            &owner_token,
            &mut outgoing,
            &incoming,
        )
        .await;
        let (reason, connected) = match result {
            Ok(()) => ("connection closed".to_owned(), true),
            Err(error) => (error.to_string(), false),
        };
        if connected {
            retry_delay = Duration::from_secs(1);
        }
        if incoming
            .send(SharedPageEvent::Disconnected(reason))
            .is_err()
        {
            return;
        }
        tokio::time::sleep(retry_delay).await;
        if !connected {
            retry_delay = (retry_delay * 2).min(MAXIMUM_RECONNECT_DELAY);
        }
    }
}

async fn exchange_messages_until_disconnected(
    websocket_url: &str,
    owner_token: &str,
    outgoing: &mut UnboundedReceiver<ClientMessage>,
    incoming: &mpsc::Sender<SharedPageEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut request = websocket_url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {owner_token}"))?,
    );
    let (socket, _) = connect_async(request).await?;
    incoming.send(SharedPageEvent::Connected)?;
    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
            message = outgoing.recv() => {
                let Some(message) = message else { return Ok(()); };
                let bytes = encode_client_message(&message)?;
                sender.send(Message::Binary(bytes.into())).await?;
            }
            message = receiver.next() => {
                let Some(message) = message else { return Ok(()); };
                match message? {
                    Message::Binary(bytes) => {
                        incoming.send(SharedPageEvent::Message(decode_server_message(&bytes)?))?;
                    }
                    Message::Ping(bytes) => sender.send(Message::Pong(bytes)).await?,
                    Message::Close(_) => return Ok(()),
                    Message::Text(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
        }
    }
}
