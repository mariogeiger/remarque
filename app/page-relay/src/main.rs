mod persistence;
mod relay;
mod settings;

use relay::serve;
use settings::RelaySettings;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixListener;

#[tokio::main]
async fn main() {
    let result = match RelaySettings::from_environment() {
        Ok(settings) => listen_and_serve(settings).await,
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("remarque_page_relay_failed={error}");
        std::process::exit(1);
    }
}

/// Serves the relay on the socket systemd passes it (socket activation),
/// else on `REMARQUE_RELAY_LISTEN`.
async fn listen_and_serve(settings: RelaySettings) -> io::Result<()> {
    match inherited_listener() {
        Some(socket) => {
            socket.set_nonblocking(true)?;
            let listener = tokio::net::UnixListener::from_std(socket)?;
            eprintln!("remarque_page_relay_listening=systemd-socket");
            serve(settings, listener).await
        }
        None => {
            let listener = tokio::net::TcpListener::bind(settings.listen_address).await?;
            eprintln!("remarque_page_relay_listening={}", settings.listen_address);
            serve(settings, listener).await
        }
    }
}

/// The one listening socket systemd passes this process, by the protocol of
/// `sd_listen_fds`: `LISTEN_PID` names it, `LISTEN_FDS` counts the sockets,
/// which begin at descriptor 3. It comes back as a duplicate that closes on
/// exec, and descriptor 3 is closed, so no child could inherit the listener.
fn inherited_listener() -> Option<UnixListener> {
    let ours = std::env::var("LISTEN_PID").ok()?.parse::<u32>().ok()? == std::process::id();
    let one = std::env::var("LISTEN_FDS").ok()? == "1";
    if !(ours && one) {
        return None;
    }
    // SAFETY: by the protocol, descriptor 3 is a listening socket this process owns.
    let passed = unsafe { OwnedFd::from_raw_fd(3) };
    Some(UnixListener::from(
        passed.try_clone().expect("the socket can be duplicated"),
    ))
}
