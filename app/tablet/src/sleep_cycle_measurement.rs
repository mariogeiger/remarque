use crate::battery::{BatteryReading, read_battery};
use crate::system_suspend::CompletedSuspend;
use remarque_document::write_bytes_atomically;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const FORMAT_VERSION: u8 = 1;

pub struct SleepCycleMeasurement {
    started_at_unix_seconds: u64,
    started_at_boottime_milliseconds: u64,
    battery_before: Option<BatteryReading>,
}

#[derive(Debug, Serialize)]
struct CompletedSleepCycleMeasurement {
    format_version: u8,
    started_at_unix_seconds: u64,
    returned_at_unix_seconds: u64,
    elapsed_milliseconds: u64,
    battery_before: Option<BatteryReading>,
    battery_after: Option<BatteryReading>,
    battery_percentage_point_change: Option<i16>,
    battery_charge_change_microamp_hours: Option<i64>,
    successful_suspend_count_before: u64,
    successful_suspend_count_after: u64,
}

impl SleepCycleMeasurement {
    pub fn capture_before_sleep() -> io::Result<Self> {
        Ok(Self {
            started_at_unix_seconds: unix_time_seconds()?,
            started_at_boottime_milliseconds: boottime_milliseconds()?,
            battery_before: read_battery().ok(),
        })
    }

    pub fn append_after_wake(
        self,
        path: &Path,
        completed_suspend: CompletedSuspend,
    ) -> io::Result<()> {
        let completed = self.complete_at(
            unix_time_seconds()?,
            boottime_milliseconds()?,
            read_battery().ok(),
            completed_suspend,
        );
        let mut bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sleep-cycle measurements do not end at a record boundary",
            ));
        }
        serde_json::to_writer(&mut bytes, &completed).map_err(io::Error::other)?;
        bytes.push(b'\n');
        write_bytes_atomically(path, &bytes)
    }

    fn complete_at(
        self,
        returned_at_unix_seconds: u64,
        returned_at_boottime_milliseconds: u64,
        battery_after: Option<BatteryReading>,
        completed_suspend: CompletedSuspend,
    ) -> CompletedSleepCycleMeasurement {
        CompletedSleepCycleMeasurement {
            format_version: FORMAT_VERSION,
            started_at_unix_seconds: self.started_at_unix_seconds,
            returned_at_unix_seconds,
            elapsed_milliseconds: returned_at_boottime_milliseconds
                .saturating_sub(self.started_at_boottime_milliseconds),
            battery_before: self.battery_before,
            battery_after,
            battery_percentage_point_change: battery_percentage_point_change(
                self.battery_before,
                battery_after,
            ),
            battery_charge_change_microamp_hours: battery_charge_change_microamp_hours(
                self.battery_before,
                battery_after,
            ),
            successful_suspend_count_before: completed_suspend.successful_suspend_count_before,
            successful_suspend_count_after: completed_suspend.successful_suspend_count_after,
        }
    }
}

fn battery_percentage_point_change(
    before: Option<BatteryReading>,
    after: Option<BatteryReading>,
) -> Option<i16> {
    Some(i16::from(after?.percentage) - i16::from(before?.percentage))
}

fn battery_charge_change_microamp_hours(
    before: Option<BatteryReading>,
    after: Option<BatteryReading>,
) -> Option<i64> {
    Some(after?.charge_microamp_hours? - before?.charge_microamp_hours?)
}

fn unix_time_seconds() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(io::Error::other)
}

fn boottime_milliseconds() -> io::Result<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let seconds = u64::try_from(time.tv_sec)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let nanoseconds = u64::try_from(time.tv_nsec)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    seconds
        .checked_mul(1_000)
        .and_then(|milliseconds| milliseconds.checked_add(nanoseconds / 1_000_000))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "boottime overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::BatteryState;

    #[test]
    fn completed_cycle_preserves_raw_charge_and_elapsed_time() {
        let before = BatteryReading {
            percentage: 96,
            charge_microamp_hours: Some(5_500_000),
            state: BatteryState::Discharging,
        };
        let after = BatteryReading {
            percentage: 93,
            charge_microamp_hours: Some(5_320_000),
            state: BatteryState::Discharging,
        };
        let completed = SleepCycleMeasurement {
            started_at_unix_seconds: 1_000,
            started_at_boottime_milliseconds: 2_500,
            battery_before: Some(before),
        }
        .complete_at(
            29_800,
            28_802_500,
            Some(after),
            CompletedSuspend {
                successful_suspend_count_before: 8,
                successful_suspend_count_after: 10,
            },
        );

        assert_eq!(completed.elapsed_milliseconds, 28_800_000);
        assert_eq!(completed.battery_percentage_point_change, Some(-3));
        assert_eq!(
            completed.battery_charge_change_microamp_hours,
            Some(-180_000)
        );
        assert_eq!(completed.successful_suspend_count_after, 10);
    }
}
