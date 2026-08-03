use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const TILE_ID: &str = "35487897-6da5-4a73-a723-71303d8da640";
const NATIVE_CONFIG: &str = "/home/root/.config/remarkable/xochitl.conf";
const NATIVE_STARTUP_SETTLE_TIME: Duration = Duration::from_secs(3);

fn line_opens_remarque_tile(line: &str) -> bool {
    line.contains("EntityOpen::open") && line.contains(TILE_ID)
}

fn service_is_active(service: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", service])
        .status()
        .is_ok_and(|status| status.success())
}

fn start_remarque() -> io::Result<()> {
    let status = Command::new("systemctl")
        .args(["start", "remarque-tablet.service"])
        .status()?;
    if status.success() {
        if let Err(error) = clear_native_document_recovery_marker(Path::new(NATIVE_CONFIG)) {
            eprintln!("could not clear native document recovery marker: {error}");
        }
        Ok(())
    } else {
        Err(io::Error::other("could not start remarque-tablet.service"))
    }
}

fn clear_native_document_recovery_marker(path: &Path) -> io::Result<()> {
    let config = fs::read_to_string(path)?;
    let cleared = clear_native_last_open_value(&config);
    if cleared == config {
        return Ok(());
    }
    let temporary = path.with_extension("conf.remarque");
    fs::write(&temporary, cleared)?;
    fs::rename(temporary, path)
}

fn clear_native_last_open_value(config: &str) -> String {
    let mut cleared = String::with_capacity(config.len());
    for line in config.split_inclusive('\n') {
        if let Some(newline) = line.strip_prefix("LastOpen=") {
            if newline == "\n" {
                cleared.push_str(line);
            } else {
                cleared.push_str("LastOpen=\n");
            }
        } else {
            cleared.push_str(line);
        }
    }
    cleared
}

fn wait_for_native_app() {
    while service_is_active("remarque-tablet.service") || !service_is_active("xochitl.service") {
        thread::sleep(Duration::from_millis(500));
    }
    thread::sleep(NATIVE_STARTUP_SETTLE_TIME);
}

fn follow_native_app_open_events() -> io::Result<()> {
    let mut journal = Command::new("journalctl")
        .args([
            "--follow",
            "--lines=0",
            "--unit=xochitl.service",
            "--output=cat",
        ])
        .stdout(Stdio::piped())
        .spawn()?;
    let output = journal
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("journal output is unavailable"))?;
    for line in BufReader::new(output).lines() {
        let line = line?;
        if !line_opens_remarque_tile(&line) {
            continue;
        }
        eprintln!("native Remarque tile opened");
        if let Err(error) = start_remarque() {
            eprintln!("{error}");
            continue;
        }
        wait_for_native_app();
    }
    let _ = journal.wait();
    Ok(())
}

fn main() -> io::Result<()> {
    loop {
        if let Err(error) = follow_native_app_open_events() {
            eprintln!("could not follow native app events: {error}");
        }
        thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_the_remarque_tile_open_event() {
        let open = format!("rm.library.ext.open EntityOpen::open: EntityId{{{TILE_ID}}}");
        assert!(line_opens_remarque_tile(&open));
        assert!(!line_opens_remarque_tile(&format!(
            "rm.docworker worker on {TILE_ID} now running"
        )));
        assert!(!line_opens_remarque_tile(
            "rm.library.ext.open EntityOpen::open: EntityId{another-document}"
        ));
    }

    #[test]
    fn clears_only_the_native_last_open_document() {
        let config = "[General]\nLastOpen=document-id\nLightSleepEnabled=true\n";
        assert_eq!(
            clear_native_last_open_value(config),
            "[General]\nLastOpen=\nLightSleepEnabled=true\n"
        );
    }
}
