use crate::bgra_image::{BgraImage, PixelRectangle};
use std::io;
use std::ptr::NonNull;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

unsafe extern "C" {
    fn quill_init() -> i32;
    fn quill_width() -> i32;
    fn quill_height() -> i32;
    fn quill_stride() -> i32;
    fn quill_format() -> i32;
    fn quill_buffer() -> *mut u8;
    fn quill_swap_mono_fast(x: i32, y: i32, width: i32, height: i32) -> u64;
    fn quill_swap_mono_quality(x: i32, y: i32, width: i32, height: i32) -> u64;
    fn quill_swap_color(x: i32, y: i32, width: i32, height: i32) -> u64;
    fn quill_swap_color_full(x: i32, y: i32, width: i32, height: i32) -> u64;
    fn quill_process_events();
}

pub type Rectangle = PixelRectangle;

pub struct DisplaySnapshot {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    pub generation: u64,
}

pub struct QuillDisplay {
    pixels: NonNull<u8>,
    width: usize,
    height: usize,
    stride: usize,
    pixel_access: RwLock<()>,
    generation: AtomicU64,
}

impl QuillDisplay {
    pub fn open() -> io::Result<Self> {
        let result = unsafe { quill_init() };
        if result != 0 {
            return Err(io::Error::other(format!("quill_init failed with {result}")));
        }
        let width = usize::try_from(unsafe { quill_width() })
            .map_err(|_| io::Error::other("invalid quill width"))?;
        let height = usize::try_from(unsafe { quill_height() })
            .map_err(|_| io::Error::other("invalid quill height"))?;
        let stride = usize::try_from(unsafe { quill_stride() })
            .map_err(|_| io::Error::other("invalid quill stride"))?;
        let pixels = NonNull::new(unsafe { quill_buffer() })
            .ok_or_else(|| io::Error::other("quill returned a null framebuffer"))?;
        if width == 0 || height == 0 || stride < width * 4 {
            return Err(io::Error::other("invalid quill framebuffer geometry"));
        }
        eprintln!(
            "display={}x{} stride={} format={}",
            width,
            height,
            stride,
            unsafe { quill_format() }
        );
        Ok(Self {
            pixels,
            width,
            height,
            stride,
            pixel_access: RwLock::new(()),
            generation: AtomicU64::new(0),
        })
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub fn copy_from(&self, image: &BgraImage, rectangle: Rectangle) -> io::Result<()> {
        if image.width() != self.width || image.height() != self.height {
            return Err(io::Error::other("image and display geometry differ"));
        }
        let x_end = rectangle.x.saturating_add(rectangle.width).min(self.width);
        let y_end = rectangle
            .y
            .saturating_add(rectangle.height)
            .min(self.height);
        if rectangle.x >= x_end || rectangle.y >= y_end {
            return Ok(());
        }
        let _pixel_access = self
            .pixel_access
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let source_stride = self.width * 4;
        let row_bytes = (x_end - rectangle.x) * 4;
        let destination = unsafe {
            std::slice::from_raw_parts_mut(self.pixels.as_ptr(), self.stride * self.height)
        };
        for y in rectangle.y..y_end {
            let source_offset = y * source_stride + rectangle.x * 4;
            let destination_offset = y * self.stride + rectangle.x * 4;
            destination[destination_offset..destination_offset + row_bytes]
                .copy_from_slice(&image.pixels()[source_offset..source_offset + row_bytes]);
        }
        self.generation.fetch_add(1, Ordering::Release);
        Ok(())
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn copy_snapshot(&self) -> DisplaySnapshot {
        let _pixel_access = self
            .pixel_access
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let source =
            unsafe { std::slice::from_raw_parts(self.pixels.as_ptr(), self.stride * self.height) };
        let row_bytes = self.width * 4;
        let mut pixels = Vec::with_capacity(row_bytes * self.height);
        for row in 0..self.height {
            let offset = row * self.stride;
            pixels.extend_from_slice(&source[offset..offset + row_bytes]);
        }
        DisplaySnapshot {
            width: self.width,
            height: self.height,
            pixels,
            generation: self.generation.load(Ordering::Acquire),
        }
    }

    pub fn show_mono_fast(&self, rectangle: Rectangle) {
        unsafe {
            quill_swap_mono_fast(
                rectangle.x as i32,
                rectangle.y as i32,
                rectangle.width as i32,
                rectangle.height as i32,
            );
            quill_process_events();
        }
    }

    pub fn show_mono_quality(&self, rectangle: Rectangle) {
        unsafe {
            quill_swap_mono_quality(
                rectangle.x as i32,
                rectangle.y as i32,
                rectangle.width as i32,
                rectangle.height as i32,
            );
            quill_process_events();
        }
    }

    pub fn show_color(&self, rectangle: Rectangle) {
        unsafe {
            quill_swap_color(
                rectangle.x as i32,
                rectangle.y as i32,
                rectangle.width as i32,
                rectangle.height as i32,
            );
            quill_process_events();
        }
    }

    pub fn show_color_full(&self) {
        unsafe {
            quill_swap_color_full(0, 0, self.width as i32, self.height as i32);
            quill_process_events();
        }
    }
}

unsafe impl Send for QuillDisplay {}
unsafe impl Sync for QuillDisplay {}
