use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;

const FILL_TRIANGLE: u64 = 0x00f1_69e0;
const IMAGE_WIDTH: usize = 1620;
const IMAGE_HEIGHT: usize = 2160;
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

#[derive(Clone, Copy)]
struct Breakpoint {
    address: u64,
    aligned_address: u64,
    original_word: u64,
    breakpoint_word: u64,
}

#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}

struct TriangleRecord {
    vertices: [Point; 3],
    coverage_coordinates: [[f32; 2]; 3],
    image: u64,
    stride: usize,
    color: u32,
}

struct RasterCapture {
    directory: PathBuf,
    image: Option<u64>,
    stride: usize,
    before: Vec<u8>,
    after: Vec<u8>,
    vertices: Vec<Point>,
}

fn main() {
    if let Err(error) = capture() {
        eprintln!("capture_error={error}");
        std::process::exit(1);
    }
}

fn capture() -> io::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let requested = arguments
        .next()
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
        .unwrap_or(32);
    let raster_directory = arguments.next().map(PathBuf::from);
    if requested == 0 || arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: capture-native-triangles [triangle-count] [raster-directory]",
        ));
    }

    let process_id = find_process_named("xochitl")?;
    require_supported_xochitl_mapping(process_id)?;
    let mut threads = attach_threads(process_id)?;
    let memory = File::open(format!("/proc/{process_id}/mem"))?;
    let mut raster = raster_directory.map(|directory| RasterCapture {
        directory,
        image: None,
        stride: 0,
        before: Vec::new(),
        after: Vec::new(),
        vertices: Vec::with_capacity(requested * 3),
    });
    let breakpoint = install_breakpoint(threads[0])?;
    for &thread_id in &threads {
        continue_thread(thread_id, 0)?;
    }
    let capture_result = capture_breakpoint_records(
        process_id,
        &memory,
        &mut threads,
        breakpoint,
        requested,
        raster.as_mut(),
    );
    let detach_result = restore_instruction_and_detach(&threads, breakpoint);
    capture_result.and(detach_result)?;
    if let Some(raster) = raster {
        save_raster_capture(&raster)?;
    }
    Ok(())
}

fn capture_breakpoint_records(
    process_id: i32,
    memory: &File,
    threads: &mut Vec<i32>,
    breakpoint: Breakpoint,
    requested: usize,
    mut raster: Option<&mut RasterCapture>,
) -> io::Result<()> {
    let mut sequence = 0;
    let required_hits = requested + usize::from(raster.is_some());
    while sequence < required_hits {
        let mut status = 0;
        let thread_id = unsafe { libc::waitpid(-1, &mut status, WAIT_FOR_ALL_THREADS) };
        if thread_id < 0 {
            return Err(io::Error::last_os_error());
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
        if signal != libc::SIGTRAP
            || ![breakpoint.address, breakpoint.address + 4].contains(&registers.program_counter)
        {
            continue_thread(thread_id, signal)?;
            continue;
        }

        let record = read_record(memory, &registers, &read_float_registers(thread_id)?)?;
        if let Some(raster) = &mut raster {
            if sequence == 0 {
                raster.image = Some(record.image);
                raster.stride = record.stride;
                raster.before = read_image(memory, record.image, record.stride)?;
            } else if record.image != raster.image.unwrap() || record.stride != raster.stride {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "triangle destination changed during raster capture",
                ));
            }
            if sequence == requested {
                raster.after = read_image(memory, record.image, record.stride)?;
            } else {
                raster.vertices.extend(record.vertices);
            }
        }
        if sequence < requested {
            write_record(sequence, &record)?;
        }
        sequence += 1;
        poke_word(
            thread_id,
            breakpoint.aligned_address,
            breakpoint.original_word,
        )?;
        registers.program_counter = breakpoint.address;
        write_general_registers(thread_id, &mut registers)?;
        ptrace_without_data(PTRACE_SINGLE_STEP, thread_id, 0)?;
        wait_for_thread(thread_id)?;
        if sequence < required_hits {
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

fn read_record(
    memory: &File,
    registers: &Aarch64Registers,
    floats: &Aarch64FloatRegisters,
) -> io::Result<TriangleRecord> {
    let renderer = registers.registers[0];
    let vertices_address = registers.registers[1];
    let mut vertex_bytes = [0; 24];
    memory.read_exact_at(&mut vertex_bytes, vertices_address)?;
    let coordinate = |index: usize| {
        f32::from_le_bytes(vertex_bytes[index * 4..index * 4 + 4].try_into().unwrap())
    };
    let image = read_u64(memory, renderer + 8)?;
    let stride = read_u64(memory, renderer + 16)? as usize;
    let color = read_u32(memory, renderer + 24)?;
    Ok(TriangleRecord {
        vertices: [
            Point {
                x: coordinate(0),
                y: coordinate(1),
            },
            Point {
                x: coordinate(2),
                y: coordinate(3),
            },
            Point {
                x: coordinate(4),
                y: coordinate(5),
            },
        ],
        coverage_coordinates: [
            [floats.scalar(0), floats.scalar(1)],
            [floats.scalar(2), floats.scalar(3)],
            [floats.scalar(4), floats.scalar(5)],
        ],
        image,
        stride,
        color,
    })
}

fn write_record(sequence: usize, record: &TriangleRecord) -> io::Result<()> {
    let mut output = io::stdout().lock();
    writeln!(
        output,
        concat!(
            "{{\"sequence\":{},",
            "\"vertices\":[[{:.9},{:.9}],[{:.9},{:.9}],[{:.9},{:.9}]],",
            "\"coverage_coordinates\":[[{:.9},{:.9}],[{:.9},{:.9}],[{:.9},{:.9}]],",
            "\"image\":\"0x{:016x}\",\"stride\":{},\"color\":\"0x{:08x}\"}}"
        ),
        sequence,
        record.vertices[0].x,
        record.vertices[0].y,
        record.vertices[1].x,
        record.vertices[1].y,
        record.vertices[2].x,
        record.vertices[2].y,
        record.coverage_coordinates[0][0],
        record.coverage_coordinates[0][1],
        record.coverage_coordinates[1][0],
        record.coverage_coordinates[1][1],
        record.coverage_coordinates[2][0],
        record.coverage_coordinates[2][1],
        record.image,
        record.stride,
        record.color,
    )?;
    output.flush()
}

fn read_image(memory: &File, image: u64, stride: usize) -> io::Result<Vec<u8>> {
    if !(IMAGE_WIDTH..=4096).contains(&stride) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid triangle destination stride {stride}"),
        ));
    }
    let mut pixels = vec![0; stride * IMAGE_HEIGHT * 4];
    memory.read_exact_at(&mut pixels, image)?;
    Ok(pixels)
}

fn save_raster_capture(capture: &RasterCapture) -> io::Result<()> {
    let rectangle = capture_rectangle(&capture.vertices)?;
    fs::create_dir_all(&capture.directory)?;
    fs::write(
        capture.directory.join("before.bgra"),
        copy_rectangle(&capture.before, capture.stride, rectangle),
    )?;
    fs::write(
        capture.directory.join("after.bgra"),
        copy_rectangle(&capture.after, capture.stride, rectangle),
    )?;
    let (x, y, width, height) = rectangle;
    fs::write(
        capture.directory.join("raster.json"),
        format!(
            concat!(
                "{{\n  \"firmware\": \"3.27.3.0\",\n",
                "  \"breakpoint\": \"0x00f169e0\",\n",
                "  \"pixel_format\": \"BGRA8888\",\n",
                "  \"full_stride_pixels\": {},\n",
                "  \"rectangle\": {{ \"x\": {}, \"y\": {}, ",
                "\"width\": {}, \"height\": {} }}\n}}\n"
            ),
            capture.stride, x, y, width, height,
        ),
    )?;
    Ok(())
}

fn capture_rectangle(vertices: &[Point]) -> io::Result<(usize, usize, usize, usize)> {
    let first = vertices.first().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "raster capture has no vertices")
    })?;
    let (mut minimum_x, mut maximum_x) = (first.x, first.x);
    let (mut minimum_y, mut maximum_y) = (first.y, first.y);
    for vertex in &vertices[1..] {
        minimum_x = minimum_x.min(vertex.x);
        maximum_x = maximum_x.max(vertex.x);
        minimum_y = minimum_y.min(vertex.y);
        maximum_y = maximum_y.max(vertex.y);
    }
    let margin = 8.0;
    let left = (minimum_x - margin).floor().max(0.0) as usize;
    let top = (minimum_y - margin).floor().max(0.0) as usize;
    let right = (maximum_x + margin).ceil().min(IMAGE_WIDTH as f32) as usize;
    let bottom = (maximum_y + margin).ceil().min(IMAGE_HEIGHT as f32) as usize;
    if left >= right || top >= bottom {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "captured vertices are outside the image",
        ));
    }
    Ok((left, top, right - left, bottom - top))
}

fn copy_rectangle(image: &[u8], stride: usize, rectangle: (usize, usize, usize, usize)) -> Vec<u8> {
    let (x, y, width, height) = rectangle;
    let mut copy = Vec::with_capacity(width * height * 4);
    for row in y..y + height {
        let start = (row * stride + x) * 4;
        copy.extend_from_slice(&image[start..start + width * 4]);
    }
    copy
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
    let aligned_address = FILL_TRIANGLE & !7;
    let instruction_shift = (FILL_TRIANGLE - aligned_address) * 8;
    let original_word = peek_word(thread_id, aligned_address)?;
    let instruction_mask = u64::from(u32::MAX) << instruction_shift;
    let breakpoint_word =
        (original_word & !instruction_mask) | (u64::from(AARCH64_BREAKPOINT) << instruction_shift);
    poke_word(thread_id, aligned_address, breakpoint_word)?;
    Ok(Breakpoint {
        address: FILL_TRIANGLE,
        aligned_address,
        original_word,
        breakpoint_word,
    })
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

fn restore_instruction_and_detach(threads: &[i32], breakpoint: Breakpoint) -> io::Result<()> {
    let mut stopped = BTreeSet::new();
    for &thread_id in threads {
        let result =
            unsafe { libc::syscall(libc::SYS_tgkill, threads[0], thread_id, libc::SIGSTOP) };
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
