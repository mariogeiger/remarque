use remarque_document::DocumentExchange;
use remarque_tablet::display::EpaperDisplay;
use remarque_tablet::document_requests::apply_all_pending_document_requests;
use remarque_tablet::input::{PenDevice, PowerButtonDevice, TouchDevice};
use remarque_tablet::notebook::Notebook;
use remarque_tablet::screen_stream::start_screen_stream;
use remarque_tablet::sleep_cycle_measurement::SleepCycleMeasurement;
use remarque_tablet::system_suspend::suspend_then_hibernate_until_woken;
use remarque_tablet::wifi::retry_wifi_reassociation_in_background;
use signal_hook::consts::{SIGINT, SIGTERM};
use std::io;
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const POWER_BUTTON_SUPPRESSION_AFTER_RESUME: Duration = Duration::from_secs(3);
const PANEL_POST_UPDATE_DISCHARGE_TIME: Duration = Duration::from_secs(30);
const SLEEPING_POWER_BUTTON_POLL_INTERVAL: Duration = Duration::from_millis(50);
const SLEEP_CYCLE_MEASUREMENTS_FILE: &str = "sleep-cycle-measurements.jsonl";

enum PanelDischargeWait {
    ReadyToSuspend,
    WakeRequested,
    StopRequested,
}

fn poll_inputs(pen: RawFd, touch: RawFd, power_button: RawFd) -> io::Result<()> {
    let mut descriptors = [
        libc::pollfd {
            fd: pen,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: touch,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: power_button,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let result = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, 16) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

fn wait_for_panel_discharge_or_power_button(
    power_button: &mut PowerButtonDevice,
    stop: &AtomicBool,
) -> io::Result<PanelDischargeWait> {
    let ready_at = Instant::now() + PANEL_POST_UPDATE_DISCHARGE_TIME;
    while Instant::now() < ready_at {
        if stop.load(Ordering::Relaxed) {
            return Ok(PanelDischargeWait::StopRequested);
        }
        let remaining = ready_at.saturating_duration_since(Instant::now());
        let timeout = remaining
            .min(SLEEPING_POWER_BUTTON_POLL_INTERVAL)
            .as_millis()
            .max(1) as i32;
        let mut descriptor = libc::pollfd {
            fd: power_button.raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
        if power_button.drain_completed_press()? {
            return Ok(PanelDischargeWait::WakeRequested);
        }
    }
    Ok(PanelDischargeWait::ReadyToSuspend)
}

fn main() -> io::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGTERM, Arc::clone(&stop))?;
    signal_hook::flag::register(SIGINT, Arc::clone(&stop))?;

    let display = Arc::new(EpaperDisplay::open()?);
    let exchange_directory = std::env::var_os("REMARQUE_EXCHANGE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/root/remarque/data/exchange"));
    let sleep_cycle_measurements_path = exchange_directory.join(SLEEP_CYCLE_MEASUREMENTS_FILE);
    let exchange = DocumentExchange::new(exchange_directory);
    exchange.prepare()?;
    let mut notebook = Notebook::new(Arc::clone(&display), exchange.library_state_path())?;
    let _screen_stream = match start_screen_stream(display) {
        Ok(thread) => Some(thread),
        Err(error) => {
            eprintln!("screen_stream_unavailable={error}");
            None
        }
    };
    let mut pen = PenDevice::open(notebook.width(), notebook.height())?;
    let mut touch = TouchDevice::open(notebook.width(), notebook.height())?;
    let mut power_button = PowerButtonDevice::open()?;

    while !stop.load(Ordering::Relaxed) {
        poll_inputs(pen.raw_fd(), touch.raw_fd(), power_button.raw_fd())?;
        if notebook.apply_pen_frames(pen.drain()?)? {
            return Ok(());
        }
        for frame in touch.drain()? {
            if notebook.apply_touch_frame(frame)? {
                return Ok(());
            }
        }
        notebook.redraw_pending_pinch_frame()?;
        let suspend_requested = power_button.drain_completed_press()?;
        apply_all_pending_document_requests(&mut notebook, &exchange)?;
        notebook.redraw_library_if_device_status_changed()?;
        if suspend_requested {
            notebook.finish_input_sequences_and_save_state()?;
            notebook.show_sleep_screen()?;
            match wait_for_panel_discharge_or_power_button(&mut power_button, &stop)? {
                PanelDischargeWait::ReadyToSuspend => {
                    let measurement = match SleepCycleMeasurement::capture_before_sleep() {
                        Ok(measurement) => Some(measurement),
                        Err(error) => {
                            eprintln!("sleep_cycle_measurement_start_failed={error}");
                            None
                        }
                    };
                    match suspend_then_hibernate_until_woken() {
                        Ok(completed_suspend) => {
                            if let Some(measurement) = measurement
                                && let Err(error) = measurement.append_after_wake(
                                    &sleep_cycle_measurements_path,
                                    completed_suspend,
                                )
                            {
                                eprintln!("sleep_cycle_measurement_write_failed={error}");
                            }
                        }
                        Err(error) => eprintln!("tablet_suspend_failed={error}"),
                    }
                    if let Err(error) = retry_wifi_reassociation_in_background() {
                        eprintln!("wifi_reassociation_start_failed={error}");
                    }
                }
                PanelDischargeWait::WakeRequested => {}
                PanelDischargeWait::StopRequested => return Ok(()),
            }
            pen.discard_pending_events_and_reset_state()?;
            touch.discard_pending_events_and_reset_state()?;
            power_button.discard_pending_events_and_reset_state()?;
            power_button.suppress_new_presses_for(POWER_BUTTON_SUPPRESSION_AFTER_RESUME);
            notebook.redraw_active_view_with_full_refresh()?;
        }
    }
    Ok(())
}
