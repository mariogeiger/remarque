use std::fs::{self, File};
use std::io;
use std::os::unix::fs::FileExt;

pub const WIDTH: usize = 1620;
pub const HEIGHT: usize = 2160;
pub const STRIDE: usize = 6528;
pub const BYTE_COUNT: usize = STRIDE * HEIGHT;

const ALLOCATION_THRESHOLD: u32 = 1632 * 2154 * 4;
const PIXEL_DATA_OFFSET: u64 = 16;
const MAX_HEADER_STEPS: usize = 4096;
const MAX_ALLOCATION_SIZE: u32 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct XochitlFramebuffer {
    process_id: u32,
    address: u64,
}

impl XochitlFramebuffer {
    pub fn locate() -> io::Result<Self> {
        Self::locate_for_process(find_process_named("xochitl")?)
    }

    pub fn locate_for_process(process_id: u32) -> io::Result<Self> {
        let maps = fs::read_to_string(format!("/proc/{process_id}/maps"))?;
        let drm_end = last_drm_mapping_end(&maps).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "xochitl has no /dev/dri/card0 mapping",
            )
        })?;
        let memory = File::open(format!("/proc/{process_id}/mem"))?;
        let (address, allocation_size) = follow_allocation_headers(&memory, drm_end)?;
        if allocation_size < BYTE_COUNT as u32 + PIXEL_DATA_OFFSET as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "candidate allocation is {allocation_size} bytes, expected at least {}",
                    BYTE_COUNT + PIXEL_DATA_OFFSET as usize
                ),
            ));
        }
        Ok(Self {
            process_id,
            address,
        })
    }

    pub fn open(&self) -> io::Result<File> {
        File::open(format!("/proc/{}/mem", self.process_id))
    }

    pub fn read(&self, memory: &File, destination: &mut [u8]) -> io::Result<()> {
        if destination.len() != BYTE_COUNT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "frame destination has {} bytes, expected {BYTE_COUNT}",
                    destination.len()
                ),
            ));
        }
        memory.read_exact_at(destination, self.address)
    }
}

fn find_process_named(expected_name: &str) -> io::Result<u32> {
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(process_id) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        if fs::read_to_string(entry.path().join("comm"))
            .map(|comm| comm.trim() == expected_name)
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

pub fn last_drm_mapping_end(maps: &str) -> Option<u64> {
    maps.lines()
        .filter(|line| line.contains("/dev/dri/card0"))
        .fold(None, |_, line| {
            line.split_whitespace()
                .next()
                .and_then(|range| range.split_once('-'))
                .and_then(|(_, end)| u64::from_str_radix(end, 16).ok())
        })
}

fn follow_allocation_headers(memory: &File, start: u64) -> io::Result<(u64, u32)> {
    let mut offset = 0_u64;
    let mut allocation_size = 2_u32;
    for _ in 0..MAX_HEADER_STEPS {
        if allocation_size >= ALLOCATION_THRESHOLD {
            return Ok((start + offset + PIXEL_DATA_OFFSET, allocation_size));
        }
        if !(2..=MAX_ALLOCATION_SIZE).contains(&allocation_size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid allocation header size {allocation_size}"),
            ));
        }
        offset = offset
            .checked_add(u64::from(allocation_size - 2))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "allocation offset overflow")
            })?;
        let mut header = [0_u8; 4];
        memory.read_exact_at(&mut header, start + offset + 8)?;
        allocation_size = u32::from_le_bytes(header);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "framebuffer allocation was not found within the step limit",
    ))
}
