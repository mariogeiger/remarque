use crate::bgra_image::{BgraImage, PixelRectangle};
use crate::fast_mono_cleanup::FastMonoCleanup;
use std::io;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

#[repr(C)]
struct PaperProEpaperFramebuffer {
    pixels: *mut u8,
    width: i32,
    height: i32,
    stride: i32,
    format: i32,
}

unsafe extern "C" {
    fn paper_pro_epaper_open(framebuffer: *mut PaperProEpaperFramebuffer) -> i32;
    fn paper_pro_epaper_submit_update(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        content_type: i32,
        screen_mode: i32,
        update_flags: i32,
    ) -> i32;
    fn paper_pro_epaper_run_pending_events();
}

const MONOCHROME_CONTENT: i32 = 0;
const COLOR_CONTENT: i32 = 1;
const MODE_ZERO: i32 = 0;
const MODE_THREE: i32 = 3;
const MODE_FOUR: i32 = 4;
const PARTIAL_UPDATE: i32 = 0;
const COMPLETE_UPDATE: i32 = 1;

pub type Rectangle = PixelRectangle;

pub struct DisplaySnapshot {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    pub generation: u64,
}

pub struct EpaperDisplay {
    pixels: NonNull<u8>,
    width: usize,
    height: usize,
    stride: usize,
    pixel_access: RwLock<()>,
    generation: AtomicU64,
    fast_mono_cleanup: Mutex<FastMonoCleanup>,
}

impl EpaperDisplay {
    pub fn open() -> io::Result<Self> {
        let mut framebuffer = PaperProEpaperFramebuffer {
            pixels: std::ptr::null_mut(),
            width: 0,
            height: 0,
            stride: 0,
            format: -1,
        };
        let result = unsafe { paper_pro_epaper_open(&mut framebuffer) };
        if result != 0 {
            return Err(io::Error::other(format!(
                "Paper Pro e-paper initialization failed with {result}"
            )));
        }
        let width = usize::try_from(framebuffer.width)
            .map_err(|_| io::Error::other("invalid e-paper framebuffer width"))?;
        let height = usize::try_from(framebuffer.height)
            .map_err(|_| io::Error::other("invalid e-paper framebuffer height"))?;
        let stride = usize::try_from(framebuffer.stride)
            .map_err(|_| io::Error::other("invalid e-paper framebuffer stride"))?;
        let pixels = NonNull::new(framebuffer.pixels)
            .ok_or_else(|| io::Error::other("e-paper framebuffer is null"))?;
        if width == 0 || height == 0 || stride < width * 4 {
            return Err(io::Error::other("invalid e-paper framebuffer geometry"));
        }
        eprintln!(
            "display={}x{} stride={} format={}",
            width, height, stride, framebuffer.format
        );
        Ok(Self {
            pixels,
            width,
            height,
            stride,
            pixel_access: RwLock::new(()),
            generation: AtomicU64::new(0),
            fast_mono_cleanup: Mutex::new(FastMonoCleanup::default()),
        })
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub fn copy_changed_from(
        &self,
        image: &BgraImage,
        rectangle: Rectangle,
    ) -> io::Result<Option<Rectangle>> {
        if image.width() != self.width || image.height() != self.height {
            return Err(io::Error::other("image and display geometry differ"));
        }
        let x_end = rectangle.x.saturating_add(rectangle.width).min(self.width);
        let y_end = rectangle
            .y
            .saturating_add(rectangle.height)
            .min(self.height);
        if rectangle.x >= x_end || rectangle.y >= y_end {
            return Ok(None);
        }
        let _pixel_access = self
            .pixel_access
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let source_stride = self.width * 4;
        let destination = unsafe {
            std::slice::from_raw_parts_mut(self.pixels.as_ptr(), self.stride * self.height)
        };
        let Some(changed) = image
            .difference_rectangle_against_strided_bgra(destination, self.stride, rectangle)
            .map_err(io::Error::other)?
        else {
            return Ok(None);
        };
        let row_bytes = changed.width * 4;
        for y in changed.y..changed.y + changed.height {
            let source_offset = y * source_stride + changed.x * 4;
            let destination_offset = y * self.stride + changed.x * 4;
            destination[destination_offset..destination_offset + row_bytes]
                .copy_from_slice(&image.pixels()[source_offset..source_offset + row_bytes]);
        }
        self.generation.fetch_add(1, Ordering::Release);
        Ok(Some(changed))
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

    fn submit_update(
        &self,
        rectangle: Rectangle,
        content_type: i32,
        screen_mode: i32,
        update_flags: i32,
    ) {
        let right = rectangle.x.saturating_add(rectangle.width).min(self.width);
        let bottom = rectangle
            .y
            .saturating_add(rectangle.height)
            .min(self.height);
        let left = rectangle.x.min(right);
        let top = rectangle.y.min(bottom);
        if left == right || top == bottom {
            return;
        }
        let submitted = unsafe {
            paper_pro_epaper_submit_update(
                i32::try_from(left).expect("display width fits i32"),
                i32::try_from(top).expect("display height fits i32"),
                i32::try_from(right - left).expect("display width fits i32"),
                i32::try_from(bottom - top).expect("display height fits i32"),
                content_type,
                screen_mode,
                update_flags,
            )
        };
        assert_eq!(submitted, 1, "Paper Pro e-paper update was rejected");
        unsafe {
            paper_pro_epaper_run_pending_events();
        }
    }

    pub fn submit_mode_zero_monochrome(&self, rectangle: Rectangle) {
        self.fast_mono_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .include_update(rectangle);
        self.submit_update(rectangle, MONOCHROME_CONTENT, MODE_ZERO, PARTIAL_UPDATE);
    }

    pub fn submit_mode_four_color(&self, changed: Option<Rectangle>) {
        let rectangle = self
            .fast_mono_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take_color_update(changed);
        let Some(rectangle) = rectangle else {
            return;
        };
        self.submit_update(rectangle, COLOR_CONTENT, MODE_FOUR, PARTIAL_UPDATE);
    }

    pub fn submit_mode_three_color(&self, changed: Option<Rectangle>) {
        let rectangle = self
            .fast_mono_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take_color_update(changed);
        let Some(rectangle) = rectangle else {
            return;
        };
        self.submit_update(rectangle, COLOR_CONTENT, MODE_THREE, PARTIAL_UPDATE);
    }

    pub fn submit_mode_four_color_full(&self) {
        self.fast_mono_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.submit_update(
            Rectangle {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
            COLOR_CONTENT,
            MODE_FOUR,
            COMPLETE_UPDATE,
        );
    }
}

unsafe impl Send for EpaperDisplay {}
unsafe impl Sync for EpaperDisplay {}
