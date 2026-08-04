use crate::battery::{BatteryReading, BatteryState};
use crate::wifi::WifiConnection;

pub(crate) fn format_device_status(
    battery: Option<BatteryReading>,
    wifi: WifiConnection,
) -> String {
    let wifi = match wifi {
        WifiConnection::Connected => "Wi-Fi connecté",
        WifiConnection::Disconnected => "Wi-Fi déconnecté",
        WifiConnection::Unavailable => "Wi-Fi indisponible",
    };
    let battery = battery.map_or_else(
        || "Batterie indisponible".to_owned(),
        |reading| {
            let charging = match reading.state {
                BatteryState::Charging => " · en charge",
                BatteryState::Full | BatteryState::Discharging | BatteryState::Unknown => "",
            };
            format!("Batterie {} %{charging}", reading.percentage)
        },
    );
    format!("{wifi}  ·  {battery}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_charge_does_not_change_displayed_status() {
        let reading = |charge_microamp_hours| BatteryReading {
            percentage: 83,
            charge_microamp_hours: Some(charge_microamp_hours),
            state: BatteryState::Discharging,
        };

        assert_eq!(
            format_device_status(Some(reading(4_800_123)), WifiConnection::Connected),
            format_device_status(Some(reading(4_799_812)), WifiConnection::Connected),
        );
    }

    #[test]
    fn formats_every_displayed_component() {
        assert_eq!(
            format_device_status(
                Some(BatteryReading {
                    percentage: 42,
                    charge_microamp_hours: Some(1),
                    state: BatteryState::Charging,
                }),
                WifiConnection::Disconnected,
            ),
            "Wi-Fi déconnecté  ·  Batterie 42 % · en charge",
        );
    }
}
