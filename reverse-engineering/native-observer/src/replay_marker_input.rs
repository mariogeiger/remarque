use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::mem::size_of;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

const EVENT_BYTES: usize = 24;
const EVENT_SYNCHRONIZE: u16 = 0;
const EVENT_KEY: u16 = 1;
const EVENT_ABSOLUTE: u16 = 3;
const BUTTON_TOOL_PEN: u16 = 320;
const BUTTON_TOOL_RUBBER: u16 = 321;
const BUTTON_TOUCH: u16 = 330;
const BUTTON_STYLUS: u16 = 331;
const BUTTON_STYLUS_2: u16 = 332;
const AXIS_X: u16 = 0;
const AXIS_Y: u16 = 1;
const AXIS_PRESSURE: u16 = 24;
const AXIS_DISTANCE: u16 = 25;
const AXIS_TILT_X: u16 = 26;
const AXIS_TILT_Y: u16 = 27;
const BUS_INTEL_ISHTP: u16 = 0x1c;
const ABSOLUTE_AXIS_COUNT: usize = 64;
const UINPUT_NAME_BYTES: usize = 80;
const UI_CREATE_DEVICE: libc::Ioctl = 0x5501;
const UI_DESTROY_DEVICE: libc::Ioctl = 0x5502;
const UI_ENABLE_EVENT_TYPE: libc::Ioctl = 0x4004_5564;
const UI_ENABLE_KEY: libc::Ioctl = 0x4004_5565;
const UI_ENABLE_ABSOLUTE_AXIS: libc::Ioctl = 0x4004_5567;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputIdentifier {
    bus_type: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UserInputDevice {
    name: [u8; UINPUT_NAME_BYTES],
    identifier: InputIdentifier,
    maximum_force_feedback_effects: u32,
    absolute_maximum: [i32; ABSOLUTE_AXIS_COUNT],
    absolute_minimum: [i32; ABSOLUTE_AXIS_COUNT],
    absolute_fuzz: [i32; ABSOLUTE_AXIS_COUNT],
    absolute_flat: [i32; ABSOLUTE_AXIS_COUNT],
}

impl Default for UserInputDevice {
    fn default() -> Self {
        Self {
            name: [0; UINPUT_NAME_BYTES],
            identifier: InputIdentifier::default(),
            maximum_force_feedback_effects: 0,
            absolute_maximum: [0; ABSOLUTE_AXIS_COUNT],
            absolute_minimum: [0; ABSOLUTE_AXIS_COUNT],
            absolute_fuzz: [0; ABSOLUTE_AXIS_COUNT],
            absolute_flat: [0; ABSOLUTE_AXIS_COUNT],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    time: libc::timeval,
    event_type: u16,
    code: u16,
    value: i32,
}

#[derive(Clone, Copy)]
struct RecordedEvent {
    time: Duration,
    event_type: u16,
    code: u16,
    value: i32,
}

struct InjectedEvent {
    source_event: Option<usize>,
    source_offset: Duration,
    event: RecordedEvent,
    before_write_ns: u64,
    after_write_ns: u64,
}

struct VirtualInputDevice {
    file: std::fs::File,
}

impl Drop for VirtualInputDevice {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.file.as_raw_fd(), UI_DESTROY_DEVICE);
        }
    }
}

fn main() {
    if let Err(error) = replay_marker_input() {
        eprintln!("replay_error={error}");
        std::process::exit(1);
    }
}

fn replay_marker_input() -> io::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let path = arguments.next().ok_or_else(usage_error)?;
    let first_event = parse_usize(arguments.next(), "FIRST_EVENT")?;
    let end_event = parse_usize(arguments.next(), "END_EVENT")?;
    let start_delay = parse_u64(arguments.next(), "START_DELAY_MS")?;
    let output_path = arguments.next().ok_or_else(usage_error)?;
    let start_trigger = arguments.next().map(std::path::PathBuf::from);
    let end_trigger = arguments.next().map(std::path::PathBuf::from);
    if arguments.next().is_some() || first_event >= end_event {
        return Err(usage_error());
    }
    if start_trigger.as_ref().is_some_and(|path| path.exists())
        || end_trigger.as_ref().is_some_and(|path| path.exists())
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "replay trigger already exists",
        ));
    }

    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)?;

    let events = read_recorded_events(Path::new(&path))?;
    let selected = events.get(first_event..end_event).ok_or_else(usage_error)?;
    let first_time = selected[0].time;
    let duration = selected.last().unwrap().time.saturating_sub(first_time);
    if duration > Duration::from_secs(30) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "selected replay exceeds 30 seconds",
        ));
    }

    let mut device = create_virtual_marker()?;
    println!(
        "device_ready=Elan marker input replay events={} duration_ms={:.3}",
        selected.len(),
        duration.as_secs_f64() * 1000.0
    );
    io::stdout().flush()?;
    thread::sleep(Duration::from_millis(start_delay));
    if let Some(path) = &start_trigger {
        wait_for_path_to_exist(path);
    }

    let replay_started = Instant::now();
    let mut injected = Vec::with_capacity(selected.len() + 7);
    for (selected_index, event) in selected.iter().enumerate() {
        let target = event.time.saturating_sub(first_time);
        thread::sleep(target.saturating_sub(replay_started.elapsed()));
        injected.push(device.write_event(Some(first_event + selected_index), target, *event)?);
    }
    injected.extend(device.release_all()?);
    write_injection_log(
        output,
        first_event,
        end_event,
        start_delay,
        start_trigger.is_some(),
        end_trigger.is_some(),
        first_time,
        duration,
        &injected,
    )?;
    println!("replay_complete_events={}", selected.len());
    io::stdout().flush()?;
    if let Some(path) = &end_trigger {
        wait_for_path_to_exist(path);
    }
    Ok(())
}

fn usage_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: replay-marker-input TRACE FIRST_EVENT END_EVENT START_DELAY_MS OUTPUT_JSONL [START_TRIGGER [END_TRIGGER]]",
    )
}

fn wait_for_path_to_exist(path: &Path) {
    while !path.exists() {
        thread::sleep(Duration::from_millis(5));
    }
}

fn parse_usize(value: Option<String>, name: &str) -> io::Result<usize> {
    value
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("invalid {name}")))
}

fn parse_u64(value: Option<String>, name: &str) -> io::Result<u64> {
    value
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("invalid {name}")))
}

fn read_recorded_events(path: &Path) -> io::Result<Vec<RecordedEvent>> {
    let bytes = fs::read(path)?;
    if bytes.is_empty() || bytes.len() % EVENT_BYTES != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "trace is not a nonempty sequence of 24-byte input_event records",
        ));
    }
    bytes
        .chunks_exact(EVENT_BYTES)
        .map(|record| {
            let seconds = i64::from_ne_bytes(record[0..8].try_into().unwrap());
            let microseconds = i64::from_ne_bytes(record[8..16].try_into().unwrap());
            if seconds < 0 || !(0..1_000_000).contains(&microseconds) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid input_event timestamp",
                ));
            }
            let event = RecordedEvent {
                time: Duration::new(seconds as u64, microseconds as u32 * 1000),
                event_type: u16::from_ne_bytes(record[16..18].try_into().unwrap()),
                code: u16::from_ne_bytes(record[18..20].try_into().unwrap()),
                value: i32::from_ne_bytes(record[20..24].try_into().unwrap()),
            };
            validate_event(event)?;
            Ok(event)
        })
        .collect()
}

fn validate_event(event: RecordedEvent) -> io::Result<()> {
    let valid = match (event.event_type, event.code) {
        (EVENT_SYNCHRONIZE, 0) => event.value == 0,
        (
            EVENT_KEY,
            BUTTON_TOOL_PEN | BUTTON_TOOL_RUBBER | BUTTON_TOUCH | BUTTON_STYLUS | BUTTON_STYLUS_2,
        ) => (0..=1).contains(&event.value),
        (EVENT_ABSOLUTE, AXIS_X) => (0..=11180).contains(&event.value),
        (EVENT_ABSOLUTE, AXIS_Y) => (0..=15340).contains(&event.value),
        (EVENT_ABSOLUTE, AXIS_PRESSURE) => (0..=4096).contains(&event.value),
        (EVENT_ABSOLUTE, AXIS_DISTANCE) => (0..=65535).contains(&event.value),
        (EVENT_ABSOLUTE, AXIS_TILT_X | AXIS_TILT_Y) => (-9000..=9000).contains(&event.value),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported input event type={} code={} value={}",
                event.event_type, event.code, event.value
            ),
        ))
    }
}

fn enable(fd: RawFd, request: libc::Ioctl, value: u16) -> io::Result<()> {
    if unsafe { libc::ioctl(fd, request, libc::c_int::from(value)) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn create_virtual_marker() -> io::Result<VirtualInputDevice> {
    let mut file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open("/dev/uinput")?;
    let fd = file.as_raw_fd();
    for event_type in [EVENT_KEY, EVENT_ABSOLUTE] {
        enable(fd, UI_ENABLE_EVENT_TYPE, event_type)?;
    }
    for key in [
        BUTTON_TOOL_PEN,
        BUTTON_TOOL_RUBBER,
        BUTTON_TOUCH,
        BUTTON_STYLUS,
        BUTTON_STYLUS_2,
    ] {
        enable(fd, UI_ENABLE_KEY, key)?;
    }
    for axis in [
        AXIS_X,
        AXIS_Y,
        AXIS_PRESSURE,
        AXIS_DISTANCE,
        AXIS_TILT_X,
        AXIS_TILT_Y,
    ] {
        enable(fd, UI_ENABLE_ABSOLUTE_AXIS, axis)?;
    }

    let mut descriptor = UserInputDevice::default();
    let name = b"Elan marker input replay";
    descriptor.name[..name.len()].copy_from_slice(name);
    descriptor.identifier.bus_type = BUS_INTEL_ISHTP;
    descriptor.absolute_maximum[usize::from(AXIS_X)] = 11180;
    descriptor.absolute_maximum[usize::from(AXIS_Y)] = 15340;
    descriptor.absolute_maximum[usize::from(AXIS_PRESSURE)] = 4096;
    descriptor.absolute_maximum[usize::from(AXIS_DISTANCE)] = 65535;
    descriptor.absolute_minimum[usize::from(AXIS_TILT_X)] = -9000;
    descriptor.absolute_maximum[usize::from(AXIS_TILT_X)] = 9000;
    descriptor.absolute_minimum[usize::from(AXIS_TILT_Y)] = -9000;
    descriptor.absolute_maximum[usize::from(AXIS_TILT_Y)] = 9000;
    let descriptor_bytes = unsafe {
        std::slice::from_raw_parts(
            (&descriptor as *const UserInputDevice).cast::<u8>(),
            size_of::<UserInputDevice>(),
        )
    };
    file.write_all(descriptor_bytes)?;
    if unsafe { libc::ioctl(fd, UI_CREATE_DEVICE) } < 0 {
        return Err(io::Error::last_os_error());
    }
    thread::sleep(Duration::from_millis(500));
    Ok(VirtualInputDevice { file })
}

impl VirtualInputDevice {
    fn write_event(
        &mut self,
        source_event: Option<usize>,
        source_offset: Duration,
        recorded: RecordedEvent,
    ) -> io::Result<InjectedEvent> {
        let event = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            event_type: recorded.event_type,
            code: recorded.code,
            value: recorded.value,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&event as *const InputEvent).cast::<u8>(),
                size_of::<InputEvent>(),
            )
        };
        let before_write_ns = monotonic_nanoseconds()?;
        self.file.write_all(bytes)?;
        let after_write_ns = monotonic_nanoseconds()?;
        Ok(InjectedEvent {
            source_event,
            source_offset,
            event: recorded,
            before_write_ns,
            after_write_ns,
        })
    }

    fn release_all(&mut self) -> io::Result<Vec<InjectedEvent>> {
        let release_events = [
            (EVENT_ABSOLUTE, AXIS_PRESSURE, 0),
            (EVENT_KEY, BUTTON_TOUCH, 0),
            (EVENT_KEY, BUTTON_STYLUS, 0),
            (EVENT_KEY, BUTTON_STYLUS_2, 0),
            (EVENT_KEY, BUTTON_TOOL_PEN, 0),
            (EVENT_KEY, BUTTON_TOOL_RUBBER, 0),
            (EVENT_SYNCHRONIZE, 0, 0),
        ];
        let mut injected = Vec::with_capacity(release_events.len());
        for (event_type, code, value) in release_events {
            let source_offset = Duration::ZERO;
            let event = RecordedEvent {
                time: Duration::ZERO,
                event_type,
                code,
                value,
            };
            injected.push(self.write_event(None, source_offset, event)?);
        }
        Ok(injected)
    }
}

fn monotonic_nanoseconds() -> io::Result<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64)
}

fn write_injection_log(
    output: File,
    first_event: usize,
    end_event: usize,
    start_delay_ms: u64,
    waited_for_start_trigger: bool,
    waited_for_end_trigger: bool,
    first_time: Duration,
    duration: Duration,
    events: &[InjectedEvent],
) -> io::Result<()> {
    let mut output = BufWriter::new(output);
    writeln!(
        output,
        "{{\"kind\":\"metadata\",\"first_event\":{first_event},\"end_event\":{end_event},\"start_delay_ms\":{start_delay_ms},\"waited_for_start_trigger\":{waited_for_start_trigger},\"waited_for_end_trigger\":{waited_for_end_trigger},\"source_first_us\":{},\"source_duration_us\":{}}}",
        first_time.as_micros(),
        duration.as_micros()
    )?;
    for (sequence, injected) in events.iter().enumerate() {
        let source_event = injected
            .source_event
            .map_or_else(|| "null".to_owned(), |index| index.to_string());
        writeln!(
            output,
            "{{\"kind\":\"event\",\"sequence\":{sequence},\"source_event\":{source_event},\"source_offset_us\":{},\"before_write_ns\":{},\"after_write_ns\":{},\"type\":{},\"code\":{},\"value\":{}}}",
            injected.source_offset.as_micros(),
            injected.before_write_ns,
            injected.after_write_ns,
            injected.event.event_type,
            injected.event.code,
            injected.event.value
        )?;
    }
    output.flush()
}
