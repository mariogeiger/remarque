use serde::Serialize;
use std::fs;
use std::io;
use std::path::Path;

const BATTERY_PATH: &str = "/sys/class/power_supply/max1726x_battery";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryState {
    Charging,
    Discharging,
    Full,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BatteryReading {
    pub percentage: u8,
    pub charge_microamp_hours: Option<i64>,
    pub state: BatteryState,
}

pub fn read_battery() -> io::Result<BatteryReading> {
    read_battery_at(Path::new(BATTERY_PATH))
}

fn read_battery_at(path: &Path) -> io::Result<BatteryReading> {
    let percentage = read_number::<u8>(&path.join("capacity"))?;
    if percentage > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "battery capacity exceeds 100 percent",
        ));
    }
    let charge_microamp_hours = match read_number::<i64>(&path.join("charge_now")) {
        Ok(charge) => Some(charge),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let state = match fs::read_to_string(path.join("status"))?.trim() {
        "Charging" => BatteryState::Charging,
        "Discharging" => BatteryState::Discharging,
        "Full" => BatteryState::Full,
        _ => BatteryState::Unknown,
    };
    Ok(BatteryReading {
        percentage,
        charge_microamp_hours,
        state,
    })
}

fn read_number<T>(path: &Path) -> io::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "remarque-battery-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn reads_percentage_charge_and_state() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("capacity"), "83\n").unwrap();
        fs::write(directory.join("charge_now"), "4800123\n").unwrap();
        fs::write(directory.join("status"), "Discharging\n").unwrap();

        assert_eq!(
            read_battery_at(&directory).unwrap(),
            BatteryReading {
                percentage: 83,
                charge_microamp_hours: Some(4_800_123),
                state: BatteryState::Discharging,
            }
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_an_impossible_percentage() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("capacity"), "101\n").unwrap();
        fs::write(directory.join("status"), "Unknown\n").unwrap();

        assert_eq!(
            read_battery_at(&directory).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
