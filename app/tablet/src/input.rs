use crate::view_transform::Point;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

const EVENT_BYTES: usize = 24;
const EVENTS_PER_READ: usize = 64;
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_PRESSURE: u16 = 24;
const ABS_MT_SLOT: u16 = 47;
const ABS_MT_TOUCH_MAJOR: u16 = 48;
const ABS_MT_POSITION_X: u16 = 53;
const ABS_MT_POSITION_Y: u16 = 54;
const ABS_MT_TOOL_TYPE: u16 = 55;
const ABS_MT_TRACKING_ID: u16 = 57;
const BTN_TOOL_PEN: u16 = 320;
const BTN_TOOL_RUBBER: u16 = 321;
const BTN_TOUCH: u16 = 330;
const EVIOCGRAB: libc::c_ulong = 0x40044590;
const PEN_MAX_X: i32 = 11180;
const PEN_MAX_Y: i32 = 15340;
const PEN_MAX_PRESSURE: i32 = 4096;
const TOUCH_MAX_X: i32 = 2064;
const TOUCH_MAX_Y: i32 = 2832;
const TOUCH_SLOTS: usize = 16;
const MT_TOOL_PALM: i32 = 2;
const PALM_AREA_THRESHOLD: f64 = 900.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PenTool {
    Tip,
    EraserEnd,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenFrame {
    pub position: Point,
    pub pressure: f32,
    pub tool: PenTool,
    pub touching: bool,
    pub proximity: bool,
}

pub struct PenDevice {
    file: OwnedFd,
    screen_width: usize,
    screen_height: usize,
    raw_x: i32,
    raw_y: i32,
    raw_pressure: i32,
    tool: PenTool,
    touching: bool,
    pen_in_range: bool,
    eraser_in_range: bool,
    changed: bool,
}

impl PenDevice {
    pub fn open(screen_width: usize, screen_height: usize) -> io::Result<Self> {
        Ok(Self {
            file: open_named_input("marker")?,
            screen_width,
            screen_height,
            raw_x: 0,
            raw_y: 0,
            raw_pressure: 0,
            tool: PenTool::Tip,
            touching: false,
            pen_in_range: false,
            eraser_in_range: false,
            changed: false,
        })
    }

    pub fn raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    pub fn drain(&mut self) -> io::Result<Vec<PenFrame>> {
        let mut frames = Vec::new();
        read_events(self.raw_fd(), |event_type, code, value| {
            match (event_type, code) {
                (EV_ABS, ABS_X) => self.raw_x = value,
                (EV_ABS, ABS_Y) => self.raw_y = value,
                (EV_ABS, ABS_PRESSURE) => self.raw_pressure = value,
                (EV_KEY, BTN_TOUCH) => self.touching = value != 0,
                (EV_KEY, BTN_TOOL_PEN) => {
                    self.pen_in_range = value != 0;
                    if self.pen_in_range {
                        self.tool = PenTool::Tip;
                    }
                }
                (EV_KEY, BTN_TOOL_RUBBER) => {
                    self.eraser_in_range = value != 0;
                    if self.eraser_in_range {
                        self.tool = PenTool::EraserEnd;
                    }
                }
                (EV_SYN, SYN_REPORT) if self.changed => {
                    self.changed = false;
                    frames.push(PenFrame {
                        position: Point {
                            x: f64::from(self.raw_x) * (self.screen_width - 1) as f64
                                / f64::from(PEN_MAX_X),
                            y: f64::from(self.raw_y) * (self.screen_height - 1) as f64
                                / f64::from(PEN_MAX_Y),
                        },
                        pressure: (self.raw_pressure as f32 / PEN_MAX_PRESSURE as f32)
                            .clamp(0.0, 1.0),
                        tool: self.tool,
                        touching: self.touching,
                        proximity: self.pen_in_range || self.eraser_in_range,
                    });
                    return;
                }
                _ => return,
            }
            self.changed = true;
        })?;
        Ok(frames)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TouchSlot {
    tracking_id: Option<i32>,
    raw_x: i32,
    raw_y: i32,
    touch_major: i32,
    tool_type: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchPoint {
    pub position: Point,
    pub major_diameter: f64,
    pub palm_classified: bool,
}

impl TouchPoint {
    pub fn is_palm(self) -> bool {
        self.palm_classified || self.major_diameter * self.major_diameter > PALM_AREA_THRESHOLD
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TouchFrame {
    pub points: Vec<TouchPoint>,
}

pub struct TouchDevice {
    file: OwnedFd,
    screen_width: usize,
    screen_height: usize,
    selected_slot: usize,
    slots: [TouchSlot; TOUCH_SLOTS],
}

impl TouchDevice {
    pub fn open(screen_width: usize, screen_height: usize) -> io::Result<Self> {
        Ok(Self {
            file: open_named_input("touch")?,
            screen_width,
            screen_height,
            selected_slot: 0,
            slots: [TouchSlot::default(); TOUCH_SLOTS],
        })
    }

    pub fn raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    pub fn drain(&mut self) -> io::Result<Vec<TouchFrame>> {
        let mut frames = Vec::new();
        read_events(self.raw_fd(), |event_type, code, value| {
            match (event_type, code) {
                (EV_ABS, ABS_MT_SLOT) => {
                    self.selected_slot = usize::try_from(value.max(0))
                        .unwrap_or(0)
                        .min(TOUCH_SLOTS - 1);
                }
                (EV_ABS, ABS_MT_TOUCH_MAJOR) => self.slots[self.selected_slot].touch_major = value,
                (EV_ABS, ABS_MT_POSITION_X) => self.slots[self.selected_slot].raw_x = value,
                (EV_ABS, ABS_MT_POSITION_Y) => self.slots[self.selected_slot].raw_y = value,
                (EV_ABS, ABS_MT_TOOL_TYPE) => self.slots[self.selected_slot].tool_type = value,
                (EV_ABS, ABS_MT_TRACKING_ID) => {
                    if value >= 0 {
                        self.slots[self.selected_slot] = TouchSlot {
                            tracking_id: Some(value),
                            ..TouchSlot::default()
                        };
                    } else {
                        self.slots[self.selected_slot] = TouchSlot::default();
                    }
                }
                (EV_SYN, SYN_REPORT) => {
                    frames.push(TouchFrame {
                        points: self
                            .slots
                            .iter()
                            .filter(|slot| slot.tracking_id.is_some())
                            .map(|slot| TouchPoint {
                                position: Point {
                                    x: f64::from(slot.raw_x) * (self.screen_width - 1) as f64
                                        / f64::from(TOUCH_MAX_X),
                                    y: f64::from(slot.raw_y) * (self.screen_height - 1) as f64
                                        / f64::from(TOUCH_MAX_Y),
                                },
                                major_diameter: f64::from(slot.touch_major),
                                palm_classified: slot.tool_type == MT_TOOL_PALM,
                            })
                            .collect(),
                    });
                }
                _ => {}
            }
        })?;
        Ok(frames)
    }
}

fn open_named_input(name_fragment: &str) -> io::Result<OwnedFd> {
    for index in 0..16 {
        let name_path = format!("/sys/class/input/event{index}/device/name");
        let Ok(name) = fs::read_to_string(name_path) else {
            continue;
        };
        if !name.to_lowercase().contains(name_fragment) {
            continue;
        }
        let path = CString::new(format!("/dev/input/event{index}")).unwrap();
        let descriptor = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { OwnedFd::from_raw_fd(descriptor) };
        let result = unsafe { libc::ioctl(file.as_raw_fd(), EVIOCGRAB, 1_i32) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        return Ok(file);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("no input device contains {name_fragment:?}"),
    ))
}

fn read_events(descriptor: RawFd, mut consume: impl FnMut(u16, u16, i32)) -> io::Result<()> {
    let mut bytes = [0_u8; EVENT_BYTES * EVENTS_PER_READ];
    loop {
        let count = unsafe {
            libc::read(
                descriptor,
                bytes.as_mut_ptr().cast::<libc::c_void>(),
                bytes.len(),
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            return match error.kind() {
                io::ErrorKind::WouldBlock => Ok(()),
                io::ErrorKind::Interrupted => continue,
                _ => Err(error),
            };
        }
        if count == 0 {
            return Ok(());
        }
        for event in bytes[..count as usize].chunks_exact(EVENT_BYTES) {
            consume(
                u16::from_ne_bytes(event[16..18].try_into().unwrap()),
                u16::from_ne_bytes(event[18..20].try_into().unwrap()),
                i32::from_ne_bytes(event[20..24].try_into().unwrap()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_point(major_diameter: f64, palm_classified: bool) -> TouchPoint {
        TouchPoint {
            position: Point { x: 0.0, y: 0.0 },
            major_diameter,
            palm_classified,
        }
    }

    #[test]
    fn native_area_threshold_accepts_diameter_thirty() {
        assert!(!touch_point(30.0, false).is_palm());
    }

    #[test]
    fn native_area_threshold_rejects_diameter_above_thirty() {
        assert!(touch_point(31.0, false).is_palm());
    }

    #[test]
    fn kernel_palm_classification_rejects_small_contact() {
        assert!(touch_point(8.0, true).is_palm());
    }
}
