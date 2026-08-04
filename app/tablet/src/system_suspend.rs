use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

const SUSPEND_SUCCESS_COUNT_PATH: &str = "/sys/power/suspend_stats/success";
const WAKE_LOCK_PATH: &str = "/sys/power/wake_lock";
const WAKE_UNLOCK_PATH: &str = "/sys/power/wake_unlock";
const APPLICATION_WAKE_LOCK: &str = "remarque-tablet";
const TRANSITION_WAKE_LOCK: &str = "remarque.suspend.transition 5000000000";
const MAX_SUSPEND_ATTEMPTS: usize = 8;
const SUSPEND_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(6);
const SUSPEND_COUNT_POLL_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletedSuspend {
    pub successful_suspend_count_before: u64,
    pub successful_suspend_count_after: u64,
}

pub fn suspend_then_hibernate_until_woken() -> io::Result<CompletedSuspend> {
    let successful_suspends_before =
        read_successful_suspend_count(Path::new(SUSPEND_SUCCESS_COUNT_PATH))?;
    unsafe { libc::sync() };
    let mut released_wake_lock =
        ReleasedApplicationWakeLock::acquire_transition_lock_and_release_application(
            Path::new(WAKE_LOCK_PATH),
            Path::new(WAKE_UNLOCK_PATH),
            APPLICATION_WAKE_LOCK,
            TRANSITION_WAKE_LOCK,
        )?;

    for attempt in 1..=MAX_SUSPEND_ATTEMPTS {
        let status = Command::new("systemctl")
            .arg("suspend-then-hibernate")
            .status();
        let status = match status {
            Ok(status) => status,
            Err(error) => {
                released_wake_lock.restore()?;
                return Err(error);
            }
        };
        if let Some(successful_suspends_after) =
            wait_for_successful_suspend_after(successful_suspends_before)?
        {
            released_wake_lock.restore()?;
            return Ok(CompletedSuspend {
                successful_suspend_count_before: successful_suspends_before,
                successful_suspend_count_after: successful_suspends_after,
            });
        }
        log_failed_suspend_attempt(attempt, status);
    }

    released_wake_lock.restore()?;
    Err(io::Error::other(format!(
        "the kernel did not complete a suspend after {MAX_SUSPEND_ATTEMPTS} attempts"
    )))
}

fn wait_for_successful_suspend_after(previous_count: u64) -> io::Result<Option<u64>> {
    let deadline = Instant::now() + SUSPEND_CONFIRMATION_TIMEOUT;
    loop {
        let current_count = read_successful_suspend_count(Path::new(SUSPEND_SUCCESS_COUNT_PATH))?;
        if current_count > previous_count {
            return Ok(Some(current_count));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(SUSPEND_COUNT_POLL_INTERVAL);
    }
}

fn read_successful_suspend_count(path: &Path) -> io::Result<u64> {
    fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn log_failed_suspend_attempt(attempt: usize, status: ExitStatus) {
    eprintln!("tablet_suspend_not_completed attempt={attempt} systemctl_status={status}");
}

struct ReleasedApplicationWakeLock<'a> {
    wake_lock_path: &'a Path,
    application_wake_lock: &'a str,
    needs_restore: bool,
}

impl<'a> ReleasedApplicationWakeLock<'a> {
    fn acquire_transition_lock_and_release_application(
        wake_lock_path: &'a Path,
        wake_unlock_path: &'a Path,
        application_wake_lock: &'a str,
        transition_wake_lock: &'a str,
    ) -> io::Result<Self> {
        fs::write(wake_lock_path, transition_wake_lock)?;
        fs::write(wake_unlock_path, application_wake_lock)?;
        Ok(Self {
            wake_lock_path,
            application_wake_lock,
            needs_restore: true,
        })
    }

    fn restore(&mut self) -> io::Result<()> {
        if !self.needs_restore {
            return Ok(());
        }
        fs::write(self.wake_lock_path, self.application_wake_lock)?;
        self.needs_restore = false;
        Ok(())
    }
}

impl Drop for ReleasedApplicationWakeLock<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("application_wake_lock_restore_failed={error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "remarque-suspend-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn successful_suspend_count_is_parsed() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let count = directory.join("success");
        fs::write(&count, "42\n").unwrap();
        assert_eq!(read_successful_suspend_count(&count).unwrap(), 42);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn transition_lock_precedes_release_and_application_lock_is_restored() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let wake_lock = directory.join("wake_lock");
        let wake_unlock = directory.join("wake_unlock");
        fs::write(&wake_lock, "").unwrap();
        fs::write(&wake_unlock, "").unwrap();

        let mut released =
            ReleasedApplicationWakeLock::acquire_transition_lock_and_release_application(
                &wake_lock,
                &wake_unlock,
                APPLICATION_WAKE_LOCK,
                TRANSITION_WAKE_LOCK,
            )
            .unwrap();
        assert_eq!(
            fs::read_to_string(&wake_lock).unwrap(),
            TRANSITION_WAKE_LOCK
        );
        assert_eq!(
            fs::read_to_string(&wake_unlock).unwrap(),
            APPLICATION_WAKE_LOCK
        );
        released.restore().unwrap();
        assert_eq!(
            fs::read_to_string(&wake_lock).unwrap(),
            APPLICATION_WAKE_LOCK
        );

        fs::remove_dir_all(directory).unwrap();
    }
}
