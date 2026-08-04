use std::io;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const REASSOCIATION_ATTEMPTS: usize = 10;
const REASSOCIATION_INTERVAL: Duration = Duration::from_secs(3);

pub fn retry_wifi_reassociation_in_background() -> io::Result<()> {
    thread::Builder::new()
        .name("wifi-reassociate".to_owned())
        .spawn(|| {
            for _ in 0..REASSOCIATION_ATTEMPTS {
                if wifi_is_connected() {
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

fn wifi_is_connected() -> bool {
    Command::new("wpa_cli")
        .args(["-i", "wlan0", "status"])
        .output()
        .is_ok_and(|output| {
            output.status.success() && status_reports_completed_connection(&output.stdout)
        })
}

fn status_reports_completed_connection(status: &[u8]) -> bool {
    status
        .split(|byte| *byte == b'\n')
        .any(|line| line == b"wpa_state=COMPLETED")
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
        assert!(status_reports_completed_connection(
            b"ssid=private\nwpa_state=COMPLETED\nip_address=192.0.2.1\n"
        ));
        assert!(!status_reports_completed_connection(
            b"wpa_state=DISCONNECTED\n"
        ));
    }
}
