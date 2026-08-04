use serde::Serialize;
use std::io;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const REASSOCIATION_ATTEMPTS: usize = 10;
const REASSOCIATION_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WifiConnection {
    Connected,
    Disconnected,
    Unavailable,
}

pub fn read_wifi_connection() -> WifiConnection {
    match Command::new("wpa_cli")
        .args(["-i", "wlan0", "status"])
        .output()
    {
        Ok(output) if output.status.success() => wifi_connection_from_status(&output.stdout),
        _ => WifiConnection::Unavailable,
    }
}

pub fn retry_wifi_reassociation_in_background() -> io::Result<()> {
    thread::Builder::new()
        .name("wifi-reassociate".to_owned())
        .spawn(|| {
            for _ in 0..REASSOCIATION_ATTEMPTS {
                if read_wifi_connection() == WifiConnection::Connected {
                    return;
                }
                run_wpa_cli(&["enable_network", "all"]);
                run_wpa_cli(&["reassociate"]);
                thread::sleep(REASSOCIATION_INTERVAL);
            }
            eprintln!("wifi_reassociation_exhausted");
        })?;
    Ok(())
}

fn wifi_connection_from_status(status: &[u8]) -> WifiConnection {
    if status
        .split(|byte| *byte == b'\n')
        .any(|line| line == b"wpa_state=COMPLETED")
    {
        WifiConnection::Connected
    } else {
        WifiConnection::Disconnected
    }
}

fn run_wpa_cli(arguments: &[&str]) {
    let _ = Command::new("wpa_cli")
        .args(["-i", "wlan0"])
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_connection_requires_the_exact_status_field() {
        assert_eq!(
            wifi_connection_from_status(
                b"ssid=private\nwpa_state=COMPLETED\nip_address=192.0.2.1\n"
            ),
            WifiConnection::Connected
        );
        assert_eq!(
            wifi_connection_from_status(b"wpa_state=DISCONNECTED\n"),
            WifiConnection::Disconnected
        );
    }
}
