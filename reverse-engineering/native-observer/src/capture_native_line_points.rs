use remarque_native_observer::xochitl_framebuffer::{
    BYTE_COUNT as FRAME_BYTE_COUNT, HEIGHT, STRIDE, WIDTH, XochitlFramebuffer,
};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;

const LINE_RENDER_RETURN: u64 = 0x0084_43c8;
const AARCH64_BREAKPOINT: u32 = 0xd420_0000;
const WAIT_FOR_ALL_THREADS: i32 = 0x4000_0000;
const NOTE_GENERAL_REGISTERS: usize = 1;
const PTRACE_GET_REGISTER_SET: libc::c_uint = 0x4204;
const PTRACE_SET_REGISTER_SET: libc::c_uint = 0x4205;
const PTRACE_READ_TEXT: i32 = 1;
const PTRACE_WRITE_TEXT: i32 = 4;
const PTRACE_CONTINUE: i32 = 7;
const PTRACE_SINGLE_STEP: i32 = 9;
const PTRACE_ATTACH_THREAD: i32 = 16;
const PTRACE_DETACH_THREAD: i32 = 17;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Aarch64Registers {
    registers: [u64; 31],
    stack_pointer: u64,
    program_counter: u64,
    processor_state: u64,
}

#[derive(Clone, Copy)]
struct Breakpoint {
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
struct ViewPoint {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy)]
struct RenderPoint {
    x: f32,
    y: f32,
    width: f32,
}

struct RasterCapture {
    directory: PathBuf,
    framebuffer: XochitlFramebuffer,
    before: Vec<u8>,
    after: Vec<u8>,
    view_points: Vec<ViewPoint>,
}

fn main() {
    if let Err(error) = capture_requested_points() {
        eprintln!("capture_error={error}");
        std::process::exit(1);
    }
}

fn capture_requested_points() -> io::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let requested = arguments
        .next()
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
        .unwrap_or(64);
    let raster_directory = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: capture-native-line-points [point-count] [raster-directory]",
        ));
    }
    if requested == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "point count must be positive",
        ));
    }

    let process_id = find_process_named("xochitl")?;
    require_supported_xochitl_mapping(process_id)?;
    let mut raster = raster_directory
        .map(|directory| {
            let framebuffer = XochitlFramebuffer::locate_for_process(process_id as u32)?;
            Ok::<_, io::Error>(RasterCapture {
                directory,
                framebuffer,
                before: vec![0; FRAME_BYTE_COUNT],
                after: vec![0; FRAME_BYTE_COUNT],
                view_points: Vec::with_capacity(requested),
            })
        })
        .transpose()?;
    let mut traced_threads = attach_threads(process_id)?;
    let memory = File::open(format!("/proc/{process_id}/mem"))?;
    if let Some(raster) = &mut raster {
        raster.framebuffer.read(&memory, &mut raster.before)?;
    }
    let breakpoint = install_breakpoint(*traced_threads.first().unwrap())?;

    for &thread_id in &traced_threads {
        continue_thread(thread_id, 0)?;
    }

    let capture_result = capture_breakpoint_records(
        process_id,
        &memory,
        &mut traced_threads,
        breakpoint,
        requested,
        raster.as_mut(),
    );
    let detach_result = restore_instruction_and_detach(&traced_threads, breakpoint);
    capture_result.and(detach_result)?;
    if let Some(raster) = raster {
        save_raster_capture(&raster)?;
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

fn install_breakpoint(thread_id: i32) -> io::Result<Breakpoint> {
    let aligned_address = LINE_RENDER_RETURN & !7;
    let instruction_shift = (LINE_RENDER_RETURN - aligned_address) * 8;
    let original_word = peek_word(thread_id, aligned_address)?;
    let instruction_mask = u64::from(u32::MAX) << instruction_shift;
    let breakpoint_word =
        (original_word & !instruction_mask) | (u64::from(AARCH64_BREAKPOINT) << instruction_shift);
    poke_word(thread_id, aligned_address, breakpoint_word)?;
    Ok(Breakpoint {
        address: LINE_RENDER_RETURN,
        aligned_address,
        original_word,
        breakpoint_word,
    })
}

fn capture_breakpoint_records(
    process_id: i32,
    memory: &File,
    traced_threads: &mut Vec<i32>,
    breakpoint: Breakpoint,
    requested: usize,
    mut raster: Option<&mut RasterCapture>,
) -> io::Result<()> {
    let mut sequence = 0;
    while sequence < requested {
        let mut status = 0;
        let thread_id = unsafe { libc::waitpid(-1, &mut status, WAIT_FOR_ALL_THREADS) };
        if thread_id < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
            traced_threads.retain(|candidate| *candidate != thread_id);
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
        let mut registers = read_registers(thread_id)?;
        if signal != libc::SIGTRAP
            || ![breakpoint.address, breakpoint.address + 4].contains(&registers.program_counter)
        {
            continue_thread(thread_id, signal)?;
            continue;
        }

        let view_point = write_capture_record(sequence, memory, registers.registers[19])?;
        if let Some(raster) = &mut raster {
            raster.view_points.push(view_point);
        }
        sequence += 1;
        if sequence == requested
            && let Some(raster) = &mut raster
        {
            raster.framebuffer.read(memory, &mut raster.after)?;
        }
        poke_word(
            thread_id,
            breakpoint.aligned_address,
            breakpoint.original_word,
        )?;
        registers.program_counter = breakpoint.address;
        write_registers(thread_id, &mut registers)?;
        ptrace_without_data(PTRACE_SINGLE_STEP, thread_id, 0)?;
        wait_for_thread(thread_id)?;
        if sequence < requested {
            poke_word(
                thread_id,
                breakpoint.aligned_address,
                breakpoint.breakpoint_word,
            )?;
        }
        continue_thread(thread_id, 0)?;
    }
    Ok(())
}

fn write_capture_record(sequence: usize, memory: &File, line_input: u64) -> io::Result<ViewPoint> {
    let state = line_input + 0x290;
    let sample = read_pen_sample(memory, state + 0x10)?;
    let previous_sample = read_pen_sample(memory, state + 0x24)?;
    let renderer = read_u64(memory, line_input + 0x370)?;
    let scene_to_view_scale = read_f32(memory, renderer + 0x60)?;
    let pipeline = read_u64(memory, renderer + 0x30)?;
    let stroke_state = read_u64(memory, pipeline + 8)?;
    let triangle_sink = read_u64(memory, stroke_state + 0x18)?;
    let triangle_sink_vtable = read_u64(memory, triangle_sink)?;
    let fill_triangle = read_u64(memory, triangle_sink_vtable + 0x10)?;
    let render_point = RenderPoint {
        x: read_f32(memory, stroke_state + 0x48)?,
        y: read_f32(memory, stroke_state + 0x4c)?,
        width: read_f32(memory, stroke_state + 4)?,
    };
    let points = read_u64(memory, state + 0x58)?;
    let point_count = read_u64(memory, state + 0x60)?;
    if point_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "point conversion returned without storing a point",
        ));
    }
    let point = read_stored_point(memory, points + (point_count - 1) * 14)?;
    let view_point = ViewPoint {
        x: read_f64(memory, line_input + 0x348)?,
        y: read_f64(memory, line_input + 0x350)?,
    };
    let mut output = io::stdout().lock();
    writeln!(
        output,
        concat!(
            "{{\"sequence\":{},\"scene_to_view_scale\":{},",
            "\"view\":{{\"x\":{},\"y\":{}}},",
            "\"render_point\":{{\"x\":{},\"y\":{},\"width\":{}}},",
            "\"fill_triangle\":\"0x{:08x}\",",
            "\"sample\":{{\"x\":{},\"y\":{},\"pressure\":{},",
            "\"tilt_x\":{},\"tilt_y\":{}}},",
            "\"previous_sample\":{{\"x\":{},\"y\":{},\"pressure\":{},",
            "\"tilt_x\":{},\"tilt_y\":{}}},",
            "\"native_point\":{{\"x\":{},\"y\":{},",
            "\"two_segment_distance_quarters\":{},\"width_quarters\":{},",
            "\"direction\":{},\"pressure\":{}}}}}"
        ),
        sequence,
        scene_to_view_scale,
        view_point.x,
        view_point.y,
        render_point.x,
        render_point.y,
        render_point.width,
        fill_triangle,
        sample.x,
        sample.y,
        sample.pressure,
        sample.tilt_x,
        sample.tilt_y,
        previous_sample.x,
        previous_sample.y,
        previous_sample.pressure,
        previous_sample.tilt_x,
        previous_sample.tilt_y,
        point.x,
        point.y,
        point.two_segment_distance_quarters,
        point.width_quarters,
        point.direction,
        point.pressure,
    )?;
    output.flush()?;
    Ok(view_point)
}

fn save_raster_capture(capture: &RasterCapture) -> io::Result<()> {
    let rectangle = capture_rectangle(&capture.view_points)?;
    fs::create_dir_all(&capture.directory)?;
    fs::write(
        capture.directory.join("before.bgra"),
        copy_rectangle(&capture.before, rectangle),
    )?;
    fs::write(
        capture.directory.join("after.bgra"),
        copy_rectangle(&capture.after, rectangle),
    )?;
    let (x, y, width, height) = rectangle;
    fs::write(
        capture.directory.join("raster.json"),
        format!(
            concat!(
                "{{\n  \"firmware\": \"3.27.3.0\",\n",
                "  \"breakpoint\": \"0x008443c8\",\n",
                "  \"pixel_format\": \"BGRA8888\",\n",
                "  \"full_stride\": {},\n",
                "  \"rectangle\": {{ \"x\": {}, \"y\": {}, ",
                "\"width\": {}, \"height\": {} }}\n}}\n"
            ),
            STRIDE, x, y, width, height,
        ),
    )?;
    Ok(())
}

fn capture_rectangle(view_points: &[ViewPoint]) -> io::Result<(usize, usize, usize, usize)> {
    let first = view_points.first().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "raster capture has no view points",
        )
    })?;
    let (mut minimum_x, mut maximum_x) = (first.x, first.x);
    let (mut minimum_y, mut maximum_y) = (first.y, first.y);
    for point in &view_points[1..] {
        minimum_x = minimum_x.min(point.x);
        maximum_x = maximum_x.max(point.x);
        minimum_y = minimum_y.min(point.y);
        maximum_y = maximum_y.max(point.y);
    }
    let margin = 32.0;
    let left = (minimum_x - margin).floor().max(0.0) as usize;
    let top = (minimum_y - margin).floor().max(0.0) as usize;
    let right = (maximum_x + margin).ceil().min(WIDTH as f64) as usize;
    let bottom = (maximum_y + margin).ceil().min(HEIGHT as f64) as usize;
    if left >= right || top >= bottom {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "captured view points are outside the display",
        ));
    }
    Ok((left, top, right - left, bottom - top))
}

fn copy_rectangle(frame: &[u8], rectangle: (usize, usize, usize, usize)) -> Vec<u8> {
    let (x, y, width, height) = rectangle;
    let mut copy = Vec::with_capacity(width * height * 4);
    for row in y..y + height {
        let start = row * STRIDE + x * 4;
        copy.extend_from_slice(&frame[start..start + width * 4]);
    }
    copy
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

fn read_u64(memory: &File, address: u64) -> io::Result<u64> {
    let mut bytes = [0; 8];
    memory.read_exact_at(&mut bytes, address)?;
    Ok(u64::from_le_bytes(bytes))
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

fn read_registers(thread_id: i32) -> io::Result<Aarch64Registers> {
    let mut registers = Aarch64Registers::default();
    transfer_registers(
        PTRACE_GET_REGISTER_SET,
        thread_id,
        (&mut registers as *mut Aarch64Registers).cast(),
        std::mem::size_of::<Aarch64Registers>(),
    )?;
    Ok(registers)
}

fn write_registers(thread_id: i32, registers: &mut Aarch64Registers) -> io::Result<()> {
    transfer_registers(
        PTRACE_SET_REGISTER_SET,
        thread_id,
        (registers as *mut Aarch64Registers).cast(),
        std::mem::size_of::<Aarch64Registers>(),
    )
}

fn transfer_registers(
    request: libc::c_uint,
    thread_id: i32,
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
            NOTE_GENERAL_REGISTERS as *mut libc::c_void,
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

fn restore_instruction_and_detach(
    traced_threads: &[i32],
    breakpoint: Breakpoint,
) -> io::Result<()> {
    let mut stopped = BTreeSet::new();
    for &thread_id in traced_threads {
        let result = unsafe {
            libc::syscall(
                libc::SYS_tgkill,
                traced_threads[0],
                thread_id,
                libc::SIGSTOP,
            )
        };
        if result == 0 && wait_for_thread(thread_id).is_ok() {
            stopped.insert(thread_id);
        }
    }
    let instruction_thread = stopped.iter().next().copied().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            "no xochitl thread could be stopped",
        )
    })?;
    poke_word(
        instruction_thread,
        breakpoint.aligned_address,
        breakpoint.original_word,
    )?;
    for thread_id in stopped {
        let _ = ptrace_without_data(PTRACE_DETACH_THREAD, thread_id, 0);
    }
    Ok(())
}

fn detach_stopped_threads(thread_ids: &[i32]) {
    for &thread_id in thread_ids {
        let _ = ptrace_without_data(PTRACE_DETACH_THREAD, thread_id, 0);
    }
}
