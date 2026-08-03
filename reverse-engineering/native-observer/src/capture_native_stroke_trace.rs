use remarque_native_observer::xochitl_framebuffer::{
    BYTE_COUNT as FRAME_BYTE_COUNT, HEIGHT as IMAGE_HEIGHT, XochitlFramebuffer,
};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

const BEGIN_PEN_LINE: u64 = 0x0084_1ea0;
const LINE_RENDER_RETURN: u64 = 0x0084_43c8;
const FINISH_PEN_LINE: u64 = 0x0084_3eb0;
const FINISH_RIBBON: u64 = 0x00f0_89e0;
const FILL_TRIANGLE: u64 = 0x00f1_69e0;
const PREPARE_UPDATE: u64 = 0x00b9_be80;
const IMAGE_WIDTH: usize = 1620;
const EVENT_BYTES: usize = 24;
const EVENTS_PER_READ: usize = 128;
const AARCH64_BREAKPOINT: u32 = 0xd420_0000;
const WAIT_FOR_ALL_THREADS: i32 = 0x4000_0000;
const NOTE_GENERAL_REGISTERS: usize = 1;
const NOTE_FLOAT_REGISTERS: usize = 2;
const PTRACE_GET_REGISTER_SET: libc::c_uint = 0x4204;
const PTRACE_SET_REGISTER_SET: libc::c_uint = 0x4205;
const PTRACE_READ_TEXT: i32 = 1;
const PTRACE_WRITE_TEXT: i32 = 4;
const PTRACE_CONTINUE: i32 = 7;
const PTRACE_SINGLE_STEP: i32 = 9;
const PTRACE_ATTACH_THREAD: i32 = 16;
const PTRACE_DETACH_THREAD: i32 = 17;
const EVIOCSCLOCKID: libc::c_ulong = 0x4004_45a0;

static SIGNAL_REQUESTED_STOP: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Aarch64Registers {
    registers: [u64; 31],
    stack_pointer: u64,
    program_counter: u64,
    processor_state: u64,
}

#[repr(C, align(16))]
struct Aarch64FloatRegisters {
    bytes: [u8; 528],
}

impl Default for Aarch64FloatRegisters {
    fn default() -> Self {
        Self { bytes: [0; 528] }
    }
}

impl Aarch64FloatRegisters {
    fn scalar(&self, register: usize) -> f32 {
        let offset = register * 16;
        f32::from_le_bytes(self.bytes[offset..offset + 4].try_into().unwrap())
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum BreakpointKind {
    BeginLine,
    LinePoint,
    FinishLine,
    FinishRibbon,
    Triangle,
    DisplayUpdate,
}

impl BreakpointKind {
    fn address(self) -> u64 {
        match self {
            Self::BeginLine => BEGIN_PEN_LINE,
            Self::LinePoint => LINE_RENDER_RETURN,
            Self::FinishLine => FINISH_PEN_LINE,
            Self::FinishRibbon => FINISH_RIBBON,
            Self::Triangle => FILL_TRIANGLE,
            Self::DisplayUpdate => PREPARE_UPDATE,
        }
    }
}

#[derive(Clone, Copy)]
struct Breakpoint {
    kind: BreakpointKind,
    address: u64,
    aligned_address: u64,
    original_word: u64,
    breakpoint_word: u64,
}

#[derive(Clone, Copy)]
struct PenSample {
    x: f32,
    y: f32,
    pressure: f32,
    tilt_x: f32,
    tilt_y: f32,
}

#[derive(Clone, Copy)]
struct StoredPoint {
    x: f32,
    y: f32,
    two_segment_distance_quarters: u16,
    width_quarters: u16,
    direction: u8,
    pressure: u8,
}

#[derive(Clone, Copy)]
struct TriangleRecord {
    vertices: [[f32; 2]; 3],
    coverage_coordinates: [[f32; 2]; 3],
    image: u64,
    stride: usize,
    color: u32,
}

struct DrawingSurfaceCapture {
    address: u64,
    stride: usize,
    before: Vec<u8>,
}

fn main() {
    if let Err(error) = capture_stroke_trace() {
        eprintln!("capture_error={error}");
        std::process::exit(1);
    }
}

fn capture_stroke_trace() -> io::Result<()> {
    let directory = parse_directory_argument()?;
    fs::create_dir_all(&directory)?;
    install_signal_handlers();

    let process_id = find_process_named("xochitl")?;
    require_supported_xochitl_mapping(process_id)?;
    let framebuffer = XochitlFramebuffer::locate_for_process(process_id as u32)?;
    let memory = File::open(format!("/proc/{process_id}/mem"))?;
    let mut frame_before = vec![0; FRAME_BYTE_COUNT];
    framebuffer.read(&memory, &mut frame_before)?;
    fs::write(directory.join("framebuffer-before.bgra"), &frame_before)?;

    let capture_input = Arc::new(AtomicBool::new(true));
    let input_thread = record_raw_pen_input(directory.clone(), capture_input.clone())?;
    let mut threads = attach_threads(process_id)?;
    let breakpoints = install_breakpoints(threads[0])?;
    let mut events = BufWriter::new(File::create(directory.join("events.jsonl"))?);
    write_metadata(&directory, process_id, &breakpoints)?;
    for &thread_id in &threads {
        continue_thread(thread_id, 0)?;
    }

    println!("capture_ready={}", directory.display());
    io::stdout().flush()?;
    let mut sequence = 0_u64;
    let mut drawing_surface = None;
    let capture_result = capture_events(
        process_id,
        &memory,
        &mut threads,
        &breakpoints,
        &mut events,
        &mut sequence,
        &mut drawing_surface,
    );

    capture_input.store(false, Ordering::Release);
    let input_result = input_thread
        .join()
        .map_err(|_| io::Error::other("raw input recorder panicked"))?;
    let stop_result = stop_threads(&threads);
    if stop_result.is_ok() {
        let mut frame_after = vec![0; FRAME_BYTE_COUNT];
        framebuffer.read(&memory, &mut frame_after)?;
        fs::write(directory.join("framebuffer-after.bgra"), frame_after)?;
        if let Some(surface) = drawing_surface {
            save_drawing_surface(&directory, &memory, surface)?;
        }
    }
    let detach_result = restore_breakpoints_and_detach(&threads, &breakpoints);
    events.flush()?;
    capture_result
        .and(input_result)
        .and(stop_result)
        .and(detach_result)?;
    println!("capture_complete_events={sequence}");
    Ok(())
}

fn parse_directory_argument() -> io::Result<PathBuf> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments.next().map(PathBuf::from).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: capture-native-stroke-trace OUTPUT-DIRECTORY",
        )
    })?;
    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: capture-native-stroke-trace OUTPUT-DIRECTORY",
        ));
    }
    Ok(directory)
}

fn install_signal_handlers() {
    unsafe extern "C" fn request_stop(_: libc::c_int) {
        SIGNAL_REQUESTED_STOP.store(true, Ordering::Release);
    }
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = request_stop as *const () as libc::sighandler_t;
        action.sa_flags = 0;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
    }
}

fn capture_events(
    process_id: i32,
    memory: &File,
    threads: &mut Vec<i32>,
    breakpoints: &[Breakpoint],
    events: &mut impl Write,
    sequence: &mut u64,
    drawing_surface: &mut Option<DrawingSurfaceCapture>,
) -> io::Result<()> {
    while !SIGNAL_REQUESTED_STOP.load(Ordering::Acquire) {
        let mut status = 0;
        let thread_id = unsafe { libc::waitpid(-1, &mut status, WAIT_FOR_ALL_THREADS) };
        if thread_id < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
            threads.retain(|candidate| *candidate != thread_id);
            if thread_id == process_id {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "xochitl exited during capture",
                ));
            }
            continue;
        }
        if !libc::WIFSTOPPED(status) {
            continue;
        }

        let signal = libc::WSTOPSIG(status);
        let mut registers = read_general_registers(thread_id)?;
        let hit = breakpoints.iter().find(|breakpoint| {
            signal == libc::SIGTRAP
                && [breakpoint.address, breakpoint.address + 4].contains(&registers.program_counter)
        });
        let Some(breakpoint) = hit else {
            continue_thread(thread_id, signal)?;
            continue;
        };

        let timestamp = monotonic_nanoseconds()?;
        write_event(
            events,
            *sequence,
            timestamp,
            thread_id,
            breakpoint.kind,
            memory,
            &registers,
            drawing_surface,
        )?;
        *sequence += 1;
        events.flush()?;

        poke_word(
            thread_id,
            breakpoint.aligned_address,
            breakpoint.original_word,
        )?;
        registers.program_counter = breakpoint.address;
        write_general_registers(thread_id, &mut registers)?;
        ptrace_without_data(PTRACE_SINGLE_STEP, thread_id, 0)?;
        wait_for_thread(thread_id)?;
        poke_word(
            thread_id,
            breakpoint.aligned_address,
            breakpoint.breakpoint_word,
        )?;
        continue_thread(thread_id, 0)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_event(
    output: &mut impl Write,
    sequence: u64,
    timestamp: u64,
    thread_id: i32,
    kind: BreakpointKind,
    memory: &File,
    registers: &Aarch64Registers,
    drawing_surface: &mut Option<DrawingSurfaceCapture>,
) -> io::Result<()> {
    write!(
        output,
        "{{\"sequence\":{sequence},\"monotonic_ns\":{timestamp},\"thread_id\":{thread_id},"
    )?;
    match kind {
        BreakpointKind::BeginLine => {
            let sample = read_pen_sample(memory, registers.registers[1])?;
            write!(
                output,
                "\"kind\":\"begin_line\",\"line_input\":\"0x{:016x}\",",
                registers.registers[0]
            )?;
            write_pen_sample(output, "sample", sample)?;
        }
        BreakpointKind::LinePoint => {
            write_line_point(output, memory, registers.registers[19])?;
        }
        BreakpointKind::FinishLine => {
            let line_input = registers.registers[0];
            let point_count = read_u64(memory, line_input + 0x2f0)?;
            write!(
                output,
                "\"kind\":\"finish_line\",\"line_input\":\"0x{line_input:016x}\",\"point_count\":{point_count}"
            )?;
        }
        BreakpointKind::FinishRibbon => {
            write!(
                output,
                "\"kind\":\"finish_ribbon\",\"ribbon_state\":\"0x{:016x}\"",
                registers.registers[0]
            )?;
        }
        BreakpointKind::Triangle => {
            let record = read_triangle(memory, thread_id, registers)?;
            if drawing_surface.is_none() {
                let before = read_drawing_surface(memory, record.image, record.stride)?;
                *drawing_surface = Some(DrawingSurfaceCapture {
                    address: record.image,
                    stride: record.stride,
                    before,
                });
            }
            write_triangle(output, record)?;
        }
        BreakpointKind::DisplayUpdate => {
            let words = read_i32_words::<8>(memory, registers.registers[0])?;
            write!(
                output,
                "\"kind\":\"display_update\",\"request\":\"0x{:016x}\",\"words_i32\":{:?}",
                registers.registers[0], words
            )?;
        }
    }
    writeln!(output, "}}")
}

fn write_line_point(output: &mut impl Write, memory: &File, line_input: u64) -> io::Result<()> {
    let state = line_input + 0x290;
    let sample = read_pen_sample(memory, state + 0x10)?;
    let previous_sample = read_pen_sample(memory, state + 0x24)?;
    let renderer = read_u64(memory, line_input + 0x370)?;
    let scene_to_view_scale = read_f32(memory, renderer + 0x60)?;
    let pipeline = read_u64(memory, renderer + 0x30)?;
    let stroke_state = read_u64(memory, pipeline + 8)?;
    let points = read_u64(memory, state + 0x58)?;
    let point_count = read_u64(memory, state + 0x60)?;
    if point_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "line renderer returned without storing a point",
        ));
    }
    let point = read_stored_point(memory, points + (point_count - 1) * 14)?;
    write!(
        output,
        concat!(
            "\"kind\":\"line_point\",\"line_input\":\"0x{:016x}\",",
            "\"point_index\":{},\"scene_to_view_scale\":{},",
            "\"view\":{{\"x\":{},\"y\":{}}},",
            "\"render_point\":{{\"x\":{},\"y\":{},\"width\":{}}},"
        ),
        line_input,
        point_count - 1,
        scene_to_view_scale,
        read_f64(memory, line_input + 0x348)?,
        read_f64(memory, line_input + 0x350)?,
        read_f32(memory, stroke_state + 0x48)?,
        read_f32(memory, stroke_state + 0x4c)?,
        read_f32(memory, stroke_state + 4)?,
    )?;
    write_pen_sample(output, "sample", sample)?;
    write!(output, ",")?;
    write_pen_sample(output, "previous_sample", previous_sample)?;
    write!(output, ",")?;
    write_stored_point(output, "native_point", point)?;
    if point_count == 2 {
        let first_point = read_stored_point(memory, points)?;
        write!(output, ",")?;
        write_stored_point(output, "first_native_point", first_point)?;
    }
    Ok(())
}

fn write_pen_sample(output: &mut impl Write, name: &str, sample: PenSample) -> io::Result<()> {
    write!(
        output,
        concat!(
            "\"{}\":{{\"x\":{},\"y\":{},\"pressure\":{},",
            "\"tilt_x\":{},\"tilt_y\":{}}}"
        ),
        name, sample.x, sample.y, sample.pressure, sample.tilt_x, sample.tilt_y,
    )
}

fn write_stored_point(output: &mut impl Write, name: &str, point: StoredPoint) -> io::Result<()> {
    write!(
        output,
        concat!(
            "\"{}\":{{\"x\":{},\"y\":{},",
            "\"two_segment_distance_quarters\":{},\"width_quarters\":{},",
            "\"direction\":{},\"pressure\":{}}}"
        ),
        name,
        point.x,
        point.y,
        point.two_segment_distance_quarters,
        point.width_quarters,
        point.direction,
        point.pressure,
    )
}

fn read_triangle(
    memory: &File,
    thread_id: i32,
    registers: &Aarch64Registers,
) -> io::Result<TriangleRecord> {
    let renderer = registers.registers[0];
    let mut bytes = [0; 24];
    memory.read_exact_at(&mut bytes, registers.registers[1])?;
    let coordinate =
        |index: usize| f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap());
    let floats = read_float_registers(thread_id)?;
    Ok(TriangleRecord {
        vertices: [
            [coordinate(0), coordinate(1)],
            [coordinate(2), coordinate(3)],
            [coordinate(4), coordinate(5)],
        ],
        coverage_coordinates: [
            [floats.scalar(0), floats.scalar(1)],
            [floats.scalar(2), floats.scalar(3)],
            [floats.scalar(4), floats.scalar(5)],
        ],
        image: read_u64(memory, renderer + 8)?,
        stride: read_u64(memory, renderer + 16)? as usize,
        color: read_u32(memory, renderer + 24)?,
    })
}

fn write_triangle(output: &mut impl Write, record: TriangleRecord) -> io::Result<()> {
    write!(
        output,
        concat!(
            "\"kind\":\"triangle\",",
            "\"vertices\":[[{:.9},{:.9}],[{:.9},{:.9}],[{:.9},{:.9}]],",
            "\"coverage_coordinates\":[[{:.9},{:.9}],[{:.9},{:.9}],[{:.9},{:.9}]],",
            "\"image\":\"0x{:016x}\",\"stride\":{},\"color\":\"0x{:08x}\""
        ),
        record.vertices[0][0],
        record.vertices[0][1],
        record.vertices[1][0],
        record.vertices[1][1],
        record.vertices[2][0],
        record.vertices[2][1],
        record.coverage_coordinates[0][0],
        record.coverage_coordinates[0][1],
        record.coverage_coordinates[1][0],
        record.coverage_coordinates[1][1],
        record.coverage_coordinates[2][0],
        record.coverage_coordinates[2][1],
        record.image,
        record.stride,
        record.color,
    )
}

fn record_raw_pen_input(
    directory: PathBuf,
    capture: Arc<AtomicBool>,
) -> io::Result<thread::JoinHandle<io::Result<()>>> {
    let input = open_marker_input()?;
    Ok(thread::spawn(move || {
        let mut output = BufWriter::new(File::create(directory.join("raw-pen-events.jsonl"))?);
        let mut bytes = [0_u8; EVENT_BYTES * EVENTS_PER_READ];
        while capture.load(Ordering::Acquire) {
            match (&input).read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => {
                    let received = monotonic_nanoseconds()?;
                    for event in bytes[..count].chunks_exact(EVENT_BYTES) {
                        writeln!(
                            output,
                            concat!(
                                "{{\"kernel_seconds\":{},\"kernel_microseconds\":{},",
                                "\"received_monotonic_ns\":{},\"type\":{},\"code\":{},\"value\":{}}}"
                            ),
                            i64::from_ne_bytes(event[0..8].try_into().unwrap()),
                            i64::from_ne_bytes(event[8..16].try_into().unwrap()),
                            received,
                            u16::from_ne_bytes(event[16..18].try_into().unwrap()),
                            u16::from_ne_bytes(event[18..20].try_into().unwrap()),
                            i32::from_ne_bytes(event[20..24].try_into().unwrap()),
                        )?;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        output.flush()
    }))
}

fn open_marker_input() -> io::Result<File> {
    for index in 0..16 {
        let Ok(name) = fs::read_to_string(format!("/sys/class/input/event{index}/device/name"))
        else {
            continue;
        };
        if !name.to_lowercase().contains("marker") {
            continue;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(format!("/dev/input/event{index}"))?;
        let clock_id = libc::CLOCK_MONOTONIC;
        let result =
            unsafe { libc::syscall(libc::SYS_ioctl, file.as_raw_fd(), EVIOCSCLOCKID, &clock_id) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        return Ok(file);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "marker input device was not found",
    ))
}

fn write_metadata(directory: &Path, process_id: i32, breakpoints: &[Breakpoint]) -> io::Result<()> {
    let mut output = BufWriter::new(File::create(directory.join("metadata.json"))?);
    writeln!(output, "{{")?;
    writeln!(output, "  \"firmware\": \"3.27.3.0\",")?;
    writeln!(output, "  \"process_id\": {process_id},")?;
    writeln!(output, "  \"clock\": \"CLOCK_MONOTONIC\",")?;
    writeln!(output, "  \"pixel_format\": \"BGRA8888\",")?;
    writeln!(output, "  \"image_width\": {IMAGE_WIDTH},")?;
    writeln!(output, "  \"image_height\": {IMAGE_HEIGHT},")?;
    writeln!(output, "  \"breakpoints\": [")?;
    for (index, breakpoint) in breakpoints.iter().enumerate() {
        let comma = if index + 1 == breakpoints.len() {
            ""
        } else {
            ","
        };
        writeln!(output, "    \"0x{:08x}\"{comma}", breakpoint.address)?;
    }
    writeln!(output, "  ]")?;
    writeln!(output, "}}")
}

fn save_drawing_surface(
    directory: &Path,
    memory: &File,
    surface: DrawingSurfaceCapture,
) -> io::Result<()> {
    fs::write(directory.join("drawing-before.bgra"), surface.before)?;
    fs::write(
        directory.join("drawing-after.bgra"),
        read_drawing_surface(memory, surface.address, surface.stride)?,
    )?;
    fs::write(
        directory.join("drawing-surface.json"),
        format!(
            concat!(
                "{{\n  \"address\": \"0x{:016x}\",\n",
                "  \"stride_pixels\": {},\n  \"width\": {},\n  \"height\": {}\n}}\n"
            ),
            surface.address, surface.stride, IMAGE_WIDTH, IMAGE_HEIGHT,
        ),
    )
}

fn read_drawing_surface(memory: &File, address: u64, stride: usize) -> io::Result<Vec<u8>> {
    if !(IMAGE_WIDTH..=4096).contains(&stride) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid drawing stride {stride}"),
        ));
    }
    let mut pixels = vec![0; stride * IMAGE_HEIGHT * 4];
    memory.read_exact_at(&mut pixels, address)?;
    Ok(pixels)
}

fn install_breakpoints(thread_id: i32) -> io::Result<Vec<Breakpoint>> {
    let kinds = [
        BreakpointKind::BeginLine,
        BreakpointKind::LinePoint,
        BreakpointKind::FinishLine,
        BreakpointKind::FinishRibbon,
        BreakpointKind::Triangle,
        BreakpointKind::DisplayUpdate,
    ];
    let mut breakpoints = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let address = kind.address();
        let aligned_address = address & !7;
        if breakpoints
            .iter()
            .any(|breakpoint: &Breakpoint| breakpoint.aligned_address == aligned_address)
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "two trace breakpoints share one machine word",
            ));
        }
        let instruction_shift = (address - aligned_address) * 8;
        let original_word = peek_word(thread_id, aligned_address)?;
        let instruction_mask = u64::from(u32::MAX) << instruction_shift;
        let breakpoint_word = (original_word & !instruction_mask)
            | (u64::from(AARCH64_BREAKPOINT) << instruction_shift);
        poke_word(thread_id, aligned_address, breakpoint_word)?;
        breakpoints.push(Breakpoint {
            kind,
            address,
            aligned_address,
            original_word,
            breakpoint_word,
        });
    }
    Ok(breakpoints)
}

fn stop_threads(threads: &[i32]) -> io::Result<()> {
    for &thread_id in threads {
        let result =
            unsafe { libc::syscall(libc::SYS_tgkill, threads[0], thread_id, libc::SIGSTOP) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        wait_for_thread(thread_id)?;
    }
    Ok(())
}

fn restore_breakpoints_and_detach(threads: &[i32], breakpoints: &[Breakpoint]) -> io::Result<()> {
    let instruction_thread = *threads.first().ok_or_else(|| {
        io::Error::new(io::ErrorKind::BrokenPipe, "xochitl has no traced threads")
    })?;
    for breakpoint in breakpoints {
        poke_word(
            instruction_thread,
            breakpoint.aligned_address,
            breakpoint.original_word,
        )?;
    }
    for &thread_id in threads {
        ptrace_without_data(PTRACE_DETACH_THREAD, thread_id, 0)?;
    }
    Ok(())
}

fn find_process_named(expected_name: &str) -> io::Result<i32> {
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(process_id) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if fs::read_to_string(entry.path().join("comm"))
            .map(|name| name.trim() == expected_name)
            .unwrap_or(false)
        {
            return Ok(process_id);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("process {expected_name:?} was not found"),
    ))
}

fn require_supported_xochitl_mapping(process_id: i32) -> io::Result<()> {
    let maps = fs::read_to_string(format!("/proc/{process_id}/maps"))?;
    if maps
        .lines()
        .any(|line| line.starts_with("00400000-") && line.ends_with("/usr/bin/xochitl"))
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "xochitl is not mapped at the address expected for firmware 3.27.3.0",
        ))
    }
}

fn attach_threads(process_id: i32) -> io::Result<Vec<i32>> {
    let mut thread_ids = fs::read_dir(format!("/proc/{process_id}/task"))?
        .map(|entry| {
            entry.and_then(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<i32>().ok())
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid thread id"))
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    thread_ids.sort_unstable();
    let mut attached = Vec::new();
    for thread_id in thread_ids {
        if let Err(error) = ptrace_without_data(PTRACE_ATTACH_THREAD, thread_id, 0) {
            detach_stopped_threads(&attached);
            return Err(error);
        }
        wait_for_thread(thread_id)?;
        attached.push(thread_id);
    }
    Ok(attached)
}

fn read_pen_sample(memory: &File, address: u64) -> io::Result<PenSample> {
    let mut bytes = [0; 20];
    memory.read_exact_at(&mut bytes, address)?;
    Ok(PenSample {
        x: f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        y: f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        pressure: f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        tilt_x: f32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        tilt_y: f32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    })
}

fn read_stored_point(memory: &File, address: u64) -> io::Result<StoredPoint> {
    let mut bytes = [0; 14];
    memory.read_exact_at(&mut bytes, address)?;
    Ok(StoredPoint {
        x: f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        y: f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        two_segment_distance_quarters: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
        width_quarters: u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
        direction: bytes[12],
        pressure: bytes[13],
    })
}

fn read_i32_words<const COUNT: usize>(memory: &File, address: u64) -> io::Result<[i32; COUNT]> {
    let mut bytes = vec![0; COUNT * 4];
    memory.read_exact_at(&mut bytes, address)?;
    Ok(std::array::from_fn(|index| {
        i32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
    }))
}

fn read_u64(memory: &File, address: u64) -> io::Result<u64> {
    let mut bytes = [0; 8];
    memory.read_exact_at(&mut bytes, address)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_u32(memory: &File, address: u64) -> io::Result<u32> {
    let mut bytes = [0; 4];
    memory.read_exact_at(&mut bytes, address)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_f32(memory: &File, address: u64) -> io::Result<f32> {
    let mut bytes = [0; 4];
    memory.read_exact_at(&mut bytes, address)?;
    Ok(f32::from_le_bytes(bytes))
}

fn read_f64(memory: &File, address: u64) -> io::Result<f64> {
    let mut bytes = [0; 8];
    memory.read_exact_at(&mut bytes, address)?;
    Ok(f64::from_le_bytes(bytes))
}

fn monotonic_nanoseconds() -> io::Result<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64)
}

fn read_general_registers(thread_id: i32) -> io::Result<Aarch64Registers> {
    let mut registers = Aarch64Registers::default();
    transfer_registers(
        PTRACE_GET_REGISTER_SET,
        thread_id,
        NOTE_GENERAL_REGISTERS,
        (&mut registers as *mut Aarch64Registers).cast(),
        std::mem::size_of::<Aarch64Registers>(),
    )?;
    Ok(registers)
}

fn write_general_registers(thread_id: i32, registers: &mut Aarch64Registers) -> io::Result<()> {
    transfer_registers(
        PTRACE_SET_REGISTER_SET,
        thread_id,
        NOTE_GENERAL_REGISTERS,
        (registers as *mut Aarch64Registers).cast(),
        std::mem::size_of::<Aarch64Registers>(),
    )
}

fn read_float_registers(thread_id: i32) -> io::Result<Aarch64FloatRegisters> {
    let mut registers = Aarch64FloatRegisters::default();
    transfer_registers(
        PTRACE_GET_REGISTER_SET,
        thread_id,
        NOTE_FLOAT_REGISTERS,
        (&mut registers as *mut Aarch64FloatRegisters).cast(),
        std::mem::size_of::<Aarch64FloatRegisters>(),
    )?;
    Ok(registers)
}

fn transfer_registers(
    request: libc::c_uint,
    thread_id: i32,
    note: usize,
    registers: *mut libc::c_void,
    byte_count: usize,
) -> io::Result<()> {
    let mut region = libc::iovec {
        iov_base: registers,
        iov_len: byte_count,
    };
    let result = unsafe {
        libc::ptrace(
            request as _,
            thread_id,
            note as *mut libc::c_void,
            (&mut region as *mut libc::iovec).cast::<libc::c_void>(),
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn peek_word(thread_id: i32, address: u64) -> io::Result<u64> {
    let word = unsafe {
        libc::ptrace(
            PTRACE_READ_TEXT as _,
            thread_id,
            address as *mut libc::c_void,
            std::ptr::null_mut::<libc::c_void>(),
        )
    };
    if word == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(word as u64)
    }
}

fn poke_word(thread_id: i32, address: u64, word: u64) -> io::Result<()> {
    let result = unsafe {
        libc::ptrace(
            PTRACE_WRITE_TEXT as _,
            thread_id,
            address as *mut libc::c_void,
            word as *mut libc::c_void,
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn ptrace_without_data(request: i32, thread_id: i32, signal: i32) -> io::Result<()> {
    let result = unsafe {
        libc::ptrace(
            request as _,
            thread_id,
            std::ptr::null_mut::<libc::c_void>(),
            signal as usize as *mut libc::c_void,
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn continue_thread(thread_id: i32, signal: i32) -> io::Result<()> {
    ptrace_without_data(PTRACE_CONTINUE, thread_id, signal)
}

fn wait_for_thread(thread_id: i32) -> io::Result<()> {
    let mut status = 0;
    let result = unsafe { libc::waitpid(thread_id, &mut status, WAIT_FOR_ALL_THREADS) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else if libc::WIFSTOPPED(status) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("thread {thread_id} did not stop"),
        ))
    }
}

fn detach_stopped_threads(thread_ids: &[i32]) {
    for &thread_id in thread_ids {
        let _ = ptrace_without_data(PTRACE_DETACH_THREAD, thread_id, 0);
    }
}

use std::os::fd::AsRawFd;
