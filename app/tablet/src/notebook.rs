use crate::battery::read_battery;
use crate::bgra_image::BgraImage;
use crate::color::Color;
use crate::device_status::format_device_status;
use crate::display::{EpaperDisplay, Rectangle};
use crate::document_library::{DocumentLibrary, restore_document_library, save_document_library};
use crate::draw_document_library::{
    DEVICE_STATUS_RECTANGLE, DocumentLibraryAction, document_library_action_at, draw_device_status,
    draw_document_library,
};
use crate::draw_sleep_screen::draw_sleep_screen;
use crate::draw_toolbar::{HEIGHT as TOOLBAR_HEIGHT, draw_toolbar};
use crate::draw_viewport_indicators::draw_viewport_indicators;
use crate::edge_page_swipe::{page_delta_from_edge_swipe, starts_at_page_edge};
use crate::erase_strokes::{EraserThickness, erase_stroke};
use crate::export_document_pages::export_document_pages;
use crate::fineliner::{FinelinerStrokeBuilder, FinelinerThickness};
use crate::input::{PenFrame, PenTool, TouchFrame};
use crate::page::Page;
use crate::pdfium::read_pdf_page_sizes;
use crate::render_fineliner::{FinelinerRasterizer, render_fineliner_raster_points};
use crate::render_page_view::{
    eraser_preview_point, fineliner_segment_rectangle, identity_transform,
    transform_background_nearest_neighbor, transform_stroke_point,
};
use crate::shared_page_connection::{SharedPageConnection, SharedPageEvent};
use crate::stroke::{PenSample, Stroke};
use crate::toolbar::{ToolbarAction, toolbar_action_at_x};
use crate::touch_gesture::{
    OneFingerGesture, PinchScaleStart, TouchGestureEvent, TouchGestureRecognizer,
};
use crate::touch_tap::TapSurface;
use crate::view_transform::{Bounds, Point, Size, ViewTransform};
use crate::wifi::read_wifi_connection;
use remarque_document::{
    DocumentSummary, ExportScope, write_bytes_atomically, write_json_atomically,
};
use remarque_page_log::{
    BackgroundAsset, BackgroundEncoding, ClientMessage, CommandId, PageCommand, PageDimensions,
    PageIdentity, PageJournal, PageOperation, PageSnapshot, ParticipantId, ServerMessage,
    SharedStroke, StrokeId, StrokeReplacement, SubmittedPageOperation, snapshot_digest,
};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PINCH_RENDER_INTERVAL: Duration = Duration::from_millis(16);
const PEN_RENDER_INTERVAL: Duration = Duration::from_millis(16);
const DEVICE_STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MINIMUM_SCALE: f64 = 1.0;
const MAXIMUM_SCALE: f64 = 5.0;
const MINIMUM_SCALE_PINCH_SEPARATION_BARRIER_RATIO: f64 = 0.02;

enum PenContact {
    Fineliner {
        builder: FinelinerStrokeBuilder,
        color: Color,
        rasterizer: FinelinerRasterizer,
        dirty: Option<Rectangle>,
        shared_stroke: Option<SharedLocalStroke>,
    },
    Eraser {
        centerline: Vec<Point>,
        cursor: Option<ImageBackup>,
    },
    OutsidePage,
    Toolbar,
    Library,
}

struct SharedLocalStroke {
    id: StrokeId,
    submitted_points: usize,
}

struct ImageBackup {
    rectangle: Rectangle,
    pixels: Vec<u8>,
}

struct OpenDocument {
    document_id: String,
    page: Page,
}

include!("notebook_shared_page.rs");

pub struct Notebook {
    display: Arc<EpaperDisplay>,
    image: BgraImage,
    open_document: Option<OpenDocument>,
    library: DocumentLibrary,
    library_screen_index: usize,
    state_path: PathBuf,
    fineliner_thickness: FinelinerThickness,
    eraser_thickness: EraserThickness,
    color: Color,
    transform: ViewTransform,
    active_pen_contact: Option<PenContact>,
    pending_pen_pixels: Option<Rectangle>,
    last_pen_render: Instant,
    pen_proximity: bool,
    touch_gestures: TouchGestureRecognizer,
    last_pinch_render: Instant,
    transform_pixels_need_render: bool,
    device_status: String,
    last_device_status_read: Instant,
    shared_page: Option<SharedPageSession>,
}

impl Notebook {
    pub fn new(display: Arc<EpaperDisplay>, state_path: PathBuf) -> io::Result<Self> {
        let width = display.width();
        let height = display.height();
        let image = BgraImage::filled(width, height, [0xff, 0xff, 0xff]);
        let mut notebook = Self {
            display,
            image,
            open_document: None,
            library: DocumentLibrary::with_default_notebook(),
            library_screen_index: 0,
            state_path,
            fineliner_thickness: FinelinerThickness::Thin,
            eraser_thickness: EraserThickness::Thin,
            color: Color::Black,
            transform: ViewTransform {
                focal_point: Point {
                    x: width as f64 * 0.5,
                    y: height as f64 * 0.5,
                },
                scale: 1.0,
            },
            active_pen_contact: None,
            pending_pen_pixels: None,
            last_pen_render: Instant::now(),
            pen_proximity: false,
            touch_gestures: TouchGestureRecognizer::default(),
            last_pinch_render: Instant::now(),
            transform_pixels_need_render: false,
            device_status: String::new(),
            last_device_status_read: Instant::now(),
            shared_page: None,
        };
        if let Err(error) = notebook.restore_state() {
            eprintln!("notebook_state_ignored={error}");
        }
        notebook.redraw_document_library()?;
        Ok(notebook)
    }

    pub fn width(&self) -> usize {
        self.display.width()
    }

    pub fn height(&self) -> usize {
        self.display.height()
    }

    pub fn finish_input_sequences_and_save_state(&mut self) -> io::Result<()> {
        self.finish_pen_contact()?;
        self.touch_gestures.reset();
        self.pen_proximity = false;
        self.save_state()
    }

    pub fn show_sleep_screen(&mut self) -> io::Result<()> {
        draw_sleep_screen(&mut self.image);
        let full = Rectangle::full(self.image.width(), self.image.height());
        self.display.copy_changed_from(&self.image, full)?;
        self.display.submit_mode_four_color_full();
        Ok(())
    }

    pub fn redraw_active_view_with_full_refresh(&mut self) -> io::Result<()> {
        if self.open_document.is_some() {
            self.redraw_notebook()?;
            self.display.submit_mode_four_color_full();
            Ok(())
        } else {
            self.redraw_document_library()
        }
    }

    pub fn redraw_library_if_device_status_changed(&mut self) -> io::Result<()> {
        if self.open_document.is_some()
            || self.last_device_status_read.elapsed() < DEVICE_STATUS_REFRESH_INTERVAL
        {
            return Ok(());
        }
        let device_status = format_device_status(read_battery().ok(), read_wifi_connection());
        self.last_device_status_read = Instant::now();
        if device_status == self.device_status {
            return Ok(());
        }
        self.device_status = device_status;
        self.redraw_device_status()
    }

    pub fn import_pdf(
        &mut self,
        document_id: String,
        source_path: &Path,
        title: String,
    ) -> io::Result<DocumentSummary> {
        self.finish_editing_input_sequences()?;
        self.store_open_document_strokes()?;
        let page_count = u32::try_from(read_pdf_page_sizes(source_path)?.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "PDF has too many pages"))?;
        let summary =
            self.library
                .import_pdf(document_id, source_path.to_owned(), title, page_count)?;
        self.show_document(&summary.document_id)?;
        Ok(summary)
    }

    pub fn open_document(&mut self, document_id: &str) -> io::Result<DocumentSummary> {
        self.finish_editing_input_sequences()?;
        self.store_open_document_strokes()?;
        let summary = self.library.document_summary(document_id)?;
        self.show_document(document_id)?;
        Ok(summary)
    }

    pub fn create_notebook(&mut self) -> io::Result<DocumentSummary> {
        self.finish_editing_input_sequences()?;
        self.store_open_document_strokes()?;
        let summary = self.library.create_notebook()?;
        self.show_document(&summary.document_id)?;
        Ok(summary)
    }

    pub fn insert_blank_page(&mut self) -> io::Result<DocumentSummary> {
        self.finish_editing_input_sequences()?;
        self.store_open_document_strokes()?;
        let document_id = self.open_document_id()?.to_owned();
        let summary = self.library.insert_blank_page(&document_id)?;
        self.show_document(&document_id)?;
        Ok(summary)
    }

    pub fn change_page(&mut self, delta: i32) -> io::Result<DocumentSummary> {
        self.finish_editing_input_sequences()?;
        self.store_open_document_strokes()?;
        let document_id = self.open_document_id()?.to_owned();
        if self.library.change_page(&document_id, delta)? {
            self.show_document(&document_id)?;
        }
        self.library.document_summary(&document_id)
    }

    pub fn documents(&self) -> Vec<DocumentSummary> {
        self.library.summaries()
    }

    pub fn export(&mut self, destination: &Path, scope: ExportScope) -> io::Result<()> {
        self.finish_editing_input_sequences()?;
        self.store_open_document_strokes()?;
        self.save_state()?;
        let document_id = self.open_document_id()?.to_owned();
        let width = self.width();
        let height = self.height();
        match scope {
            ExportScope::CurrentPage => export_document_pages(
                destination,
                std::slice::from_ref(self.library.page(&document_id)?),
                width,
                height,
                TOOLBAR_HEIGHT,
            ),
            ExportScope::AllPages => export_document_pages(
                destination,
                self.library.pages(&document_id)?,
                width,
                height,
                TOOLBAR_HEIGHT,
            ),
        }
    }

    pub fn apply_pen_frames(&mut self, frames: Vec<PenFrame>) -> io::Result<bool> {
        for frame in frames {
            if self.apply_pen_frame(frame)? {
                self.submit_all_pending_pen_pixels();
                return Ok(true);
            }
        }
        if self.last_pen_render.elapsed() >= PEN_RENDER_INTERVAL {
            self.flush_shared_stroke_points()?;
        }
        self.submit_pending_pen_pixels_if_due();
        Ok(false)
    }

    fn apply_pen_frame(&mut self, frame: PenFrame) -> io::Result<bool> {
        self.pen_proximity = frame.proximity;
        if !frame.touching {
            if self.active_pen_contact.is_some() {
                self.finish_pen_contact()?;
            }
            return Ok(false);
        }

        if self.open_document.is_none() {
            if self.active_pen_contact.is_none() {
                let documents = self.library.summaries();
                let action = document_library_action_at(
                    frame.position,
                    &documents,
                    self.library_screen_index,
                );
                self.active_pen_contact = Some(PenContact::Library);
                if self.apply_document_library_action(action)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        if matches!(
            self.active_pen_contact.as_ref(),
            Some(PenContact::Fineliner { .. } | PenContact::Eraser { .. })
        ) && !self.page_contains_drawable_view_point(frame.position)?
        {
            self.finish_pen_contact()?;
            self.active_pen_contact = Some(PenContact::OutsidePage);
            return Ok(false);
        }

        if self.active_pen_contact.is_none() {
            if frame.position.y < TOOLBAR_HEIGHT as f64 {
                self.apply_toolbar_action(toolbar_action_at_x(frame.position.x as usize))?;
                self.active_pen_contact = Some(PenContact::Toolbar);
                if self.open_document.is_some() {
                    self.redraw_toolbar()?;
                }
                return Ok(false);
            }
            if !self.page_contains_drawable_view_point(frame.position)? {
                self.active_pen_contact = Some(PenContact::OutsidePage);
                return Ok(false);
            }
            let drawing_color = if self.current_page_is_shared() {
                Color::Black
            } else {
                self.color
            };
            self.active_pen_contact = Some(match frame.tool {
                PenTool::Tip => PenContact::Fineliner {
                    builder: FinelinerStrokeBuilder::new(self.fineliner_thickness),
                    color: drawing_color,
                    rasterizer: FinelinerRasterizer::new(drawing_color),
                    dirty: None,
                    shared_stroke: self.begin_shared_stroke()?,
                },
                PenTool::EraserEnd => PenContact::Eraser {
                    centerline: Vec::new(),
                    cursor: None,
                },
            });
        }

        let scene_position = self.view_to_scene(frame.position);
        let view_size = self.view_size();
        let contact = self
            .active_pen_contact
            .as_mut()
            .ok_or_else(|| io::Error::other("touching pen has no active contact"))?;
        let changed = match contact {
            PenContact::Fineliner {
                builder,
                color: _,
                rasterizer,
                dirty,
                shared_stroke: _,
            } => {
                let point = builder.append_sample(
                    PenSample {
                        x: scene_position.x as f32,
                        y: scene_position.y as f32,
                        pressure: frame.pressure,
                    },
                    self.transform.scale as f32,
                );
                let screen_point = transform_stroke_point(point, self.transform, view_size);
                let screen_previous = builder
                    .points()
                    .get(builder.points().len().saturating_sub(2))
                    .copied()
                    .map(|previous| transform_stroke_point(previous, self.transform, view_size))
                    .unwrap_or(screen_point);
                rasterizer.append_point(&mut self.image, screen_point);
                let changed = fineliner_segment_rectangle(
                    screen_previous,
                    screen_point,
                    self.image.width(),
                    self.image.height(),
                );
                *dirty = Some(dirty.map_or(changed, |dirty| dirty.include(changed)));
                changed
            }
            PenContact::Eraser { centerline, cursor } => {
                let previous = centerline.last().copied().unwrap_or(scene_position);
                centerline.push(scene_position);
                let width = self.eraser_thickness.pixels() * self.transform.scale;
                let preview = [
                    eraser_preview_point(previous, width, self.transform, view_size),
                    eraser_preview_point(scene_position, width, self.transform, view_size),
                ];
                let mut changed = fineliner_segment_rectangle(
                    preview[0],
                    preview[1],
                    self.image.width(),
                    self.image.height(),
                );
                if let Some(previous_cursor) = cursor.take() {
                    self.image.restore_rectangle(
                        previous_cursor.rectangle.x,
                        previous_cursor.rectangle.y,
                        previous_cursor.rectangle.width,
                        previous_cursor.rectangle.height,
                        &previous_cursor.pixels,
                    );
                    changed = changed.include(previous_cursor.rectangle);
                }
                render_fineliner_raster_points(&mut self.image, &preview, Color::White);
                let cursor_rectangle = fineliner_segment_rectangle(
                    preview[1],
                    preview[1],
                    self.image.width(),
                    self.image.height(),
                );
                let pixels = self.image.copy_rectangle(
                    cursor_rectangle.x,
                    cursor_rectangle.y,
                    cursor_rectangle.width,
                    cursor_rectangle.height,
                );
                self.image.draw_circle_outline(
                    preview[1].x,
                    preview[1].y,
                    width as f32,
                    [0x55, 0x55, 0x55],
                );
                *cursor = Some(ImageBackup {
                    rectangle: cursor_rectangle,
                    pixels,
                });
                changed = changed.include(cursor_rectangle);
                changed
            }
            PenContact::Toolbar | PenContact::Library | PenContact::OutsidePage => {
                return Ok(false);
            }
        };
        if changed.y < TOOLBAR_HEIGHT {
            self.draw_toolbar_into_image()?;
        }
        if let Some(changed) = self.display.copy_changed_from(&self.image, changed)? {
            self.pending_pen_pixels = Some(
                self.pending_pen_pixels
                    .map_or(changed, |pending| pending.include(changed)),
            );
        }
        Ok(false)
    }

    fn submit_pending_pen_pixels_if_due(&mut self) {
        if self.last_pen_render.elapsed() < PEN_RENDER_INTERVAL {
            return;
        }
        self.submit_all_pending_pen_pixels();
    }

    fn submit_all_pending_pen_pixels(&mut self) {
        if let Some(changed) = self.pending_pen_pixels.take() {
            self.display.submit_mode_zero_monochrome(changed);
            self.last_pen_render = Instant::now();
        }
    }

    fn take_pending_pen_pixels_with(&mut self, changed: Option<Rectangle>) -> Option<Rectangle> {
        match (self.pending_pen_pixels.take(), changed) {
            (Some(pending), Some(changed)) => Some(pending.include(changed)),
            (Some(pending), None) => Some(pending),
            (None, changed) => changed,
        }
    }

    pub fn apply_touch_frame(&mut self, frame: TouchFrame) -> io::Result<bool> {
        let document_is_open = self.open_document.is_some();
        let page_count = self
            .open_document
            .as_ref()
            .map(|document| self.library.document_summary(&document.document_id))
            .transpose()?
            .map_or(0, |summary| summary.page_count);
        let screen_width = self.width() as f64;
        let pinch_scale_start = if self.transform.scale <= MINIMUM_SCALE {
            let view = self.view_size();
            PinchScaleStart::AfterSeparationIncrease(
                view.width.min(view.height) * MINIMUM_SCALE_PINCH_SEPARATION_BARRIER_RATIO,
            )
        } else {
            PinchScaleStart::Immediate
        };
        let event =
            self.touch_gestures
                .update(&frame, self.pen_proximity, pinch_scale_start, |position| {
                    if !document_is_open {
                        Some(OneFingerGesture::Tap(TapSurface::DocumentLibrary))
                    } else if position.y < TOOLBAR_HEIGHT as f64 {
                        Some(OneFingerGesture::Tap(TapSurface::Toolbar))
                    } else if page_count > 1 && starts_at_page_edge(position, screen_width) {
                        Some(OneFingerGesture::PageSwipe)
                    } else {
                        None
                    }
                });
        match event {
            Some(TouchGestureEvent::Tap { surface, position }) => {
                return self.apply_touch_tap(surface, position);
            }
            Some(TouchGestureEvent::PageSwipe { start, end }) => {
                if let Some(delta) = page_delta_from_edge_swipe(start, end, screen_width) {
                    self.change_page(delta)?;
                }
            }
            Some(TouchGestureEvent::PinchChanged {
                previous_centroid,
                current_centroid,
                scale_factor,
            }) => {
                if document_is_open {
                    self.apply_pinch_change(previous_centroid, current_centroid, scale_factor)?;
                }
            }
            Some(TouchGestureEvent::PinchFinished) if document_is_open => self.finish_pinch()?,
            Some(TouchGestureEvent::PinchFinished) => {}
            None => {}
        }
        Ok(false)
    }

    fn apply_touch_tap(&mut self, surface: TapSurface, position: Point) -> io::Result<bool> {
        match surface {
            TapSurface::DocumentLibrary if self.open_document.is_none() => {
                let documents = self.library.summaries();
                let action =
                    document_library_action_at(position, &documents, self.library_screen_index);
                self.apply_document_library_action(action)
            }
            TapSurface::Toolbar
                if self.open_document.is_some() && position.y < TOOLBAR_HEIGHT as f64 =>
            {
                self.apply_toolbar_action(toolbar_action_at_x(position.x as usize))?;
                if self.open_document.is_some() {
                    self.redraw_toolbar()?;
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn apply_pinch_change(
        &mut self,
        previous_centroid: Point,
        current_centroid: Point,
        scale_factor: f64,
    ) -> io::Result<()> {
        let scale_factor = self.transform.factor_clamped_to_scale_limits(
            scale_factor,
            MINIMUM_SCALE,
            MAXIMUM_SCALE,
        );
        if let Some(transform) = self.transform.scale_and_translate(
            previous_centroid,
            current_centroid,
            scale_factor,
            self.view_size(),
            self.drawable_view_bounds(),
            self.page_bounds()?,
        ) && transform != self.transform
        {
            self.transform = transform;
            self.transform_pixels_need_render = true;
        }
        Ok(())
    }

    pub fn redraw_pending_pinch_frame(&mut self) -> io::Result<()> {
        if self.transform_pixels_need_render
            && self.last_pinch_render.elapsed() >= PINCH_RENDER_INTERVAL
        {
            if let Some(changed) = self.redraw_notebook()? {
                self.display.submit_mode_zero_monochrome(changed);
            }
            self.last_pinch_render = Instant::now();
        }
        Ok(())
    }

    fn finish_pinch(&mut self) -> io::Result<()> {
        let changed = if self.transform_pixels_need_render {
            self.redraw_notebook()?
        } else {
            None
        };
        self.display.submit_mode_four_color(changed);
        Ok(())
    }

    fn finish_pen_contact(&mut self) -> io::Result<()> {
        match self.active_pen_contact.take() {
            Some(PenContact::Fineliner {
                builder,
                color,
                mut rasterizer,
                dirty,
                shared_stroke,
            }) => {
                rasterizer.finish(&mut self.image);
                let points = builder.finish();
                if points.is_empty() {
                    if let Some(shared_stroke) = shared_stroke {
                        self.cancel_shared_stroke(shared_stroke)?;
                    }
                } else {
                    let page_x = self.page()?.rectangle.x as f32;
                    let page_y = self.page()?.rectangle.y as f32;
                    let points: Vec<_> = points
                        .into_iter()
                        .map(|mut point| {
                            point.x -= page_x;
                            point.y -= page_y;
                            point
                        })
                        .collect();
                    self.page_mut()?.strokes.push(Stroke {
                        points: points.clone(),
                        color,
                    });
                    if let Some(shared_stroke) = shared_stroke {
                        self.finish_shared_stroke(shared_stroke, &points)?;
                    }
                    self.save_state()?;
                }
                if let Some(dirty) = dirty {
                    if dirty.y < TOOLBAR_HEIGHT {
                        self.draw_toolbar_into_image()?;
                    }
                    let changed = self.display.copy_changed_from(&self.image, dirty)?;
                    let changed = self.take_pending_pen_pixels_with(changed);
                    self.display.submit_mode_four_color(changed);
                }
            }
            Some(PenContact::Eraser { centerline, .. }) => {
                if !centerline.is_empty() {
                    let mut surviving = Vec::new();
                    let page_x = self.page()?.rectangle.x as f64;
                    let page_y = self.page()?.rectangle.y as f64;
                    let local_centerline = centerline
                        .into_iter()
                        .map(|point| Point {
                            x: point.x - page_x,
                            y: point.y - page_y,
                        })
                        .collect::<Vec<_>>();
                    let eraser_width = self.eraser_thickness.pixels();
                    self.submit_shared_eraser(&local_centerline, eraser_width)?;
                    let page = self.page_mut()?;
                    for stroke in page.strokes.drain(..) {
                        for points in erase_stroke(&stroke.points, &local_centerline, eraser_width)
                        {
                            surviving.push(Stroke {
                                points,
                                color: stroke.color,
                            });
                        }
                    }
                    page.strokes = surviving;
                    self.save_state()?;
                    let changed = self.redraw_notebook()?;
                    let changed = self.take_pending_pen_pixels_with(changed);
                    self.display.submit_mode_four_color(changed);
                }
            }
            Some(PenContact::OutsidePage | PenContact::Toolbar | PenContact::Library) | None => {}
        }
        Ok(())
    }

    fn finish_editing_input_sequences(&mut self) -> io::Result<()> {
        if matches!(
            self.active_pen_contact.as_ref(),
            Some(PenContact::Fineliner { .. } | PenContact::Eraser { .. })
        ) {
            self.finish_pen_contact()?;
        }
        self.touch_gestures.reset();
        Ok(())
    }

    fn redraw_notebook(&mut self) -> io::Result<Option<Rectangle>> {
        self.image = self.render_scene(self.transform)?;
        self.draw_toolbar_into_image()?;
        let transform = self.transform;
        let view_size = self.view_size();
        let visible_view = self.drawable_view_bounds();
        let page = self.page_bounds()?;
        draw_viewport_indicators(&mut self.image, transform, view_size, visible_view, page);
        let changed = self.display.copy_changed_from(
            &self.image,
            Rectangle::full(self.image.width(), self.image.height()),
        )?;
        self.transform_pixels_need_render = false;
        Ok(changed)
    }

    fn render_scene(&self, transform: ViewTransform) -> io::Result<BgraImage> {
        let view_size = self.view_size();
        let page = self.page()?;
        let background = page.raster_background(self.width(), self.height());
        let mut image = transform_background_nearest_neighbor(
            &background,
            transform,
            view_size,
            self.width(),
            self.height(),
            TOOLBAR_HEIGHT,
        );
        for stroke in &page.strokes {
            let points: Vec<_> = stroke
                .points
                .iter()
                .copied()
                .map(|mut point| {
                    point.x += page.rectangle.x as f32;
                    point.y += page.rectangle.y as f32;
                    transform_stroke_point(point, transform, view_size)
                })
                .collect();
            render_fineliner_raster_points(&mut image, &points, stroke.color);
        }
        if let Some(session) = &self.shared_page
            && self.current_page_identity().as_ref() == Some(&session.identity)
        {
            for active in &session.journal.snapshot().active_strokes {
                let points = active
                    .stroke
                    .points
                    .iter()
                    .copied()
                    .map(|mut point| {
                        point.x += page.rectangle.x as f32;
                        point.y += page.rectangle.y as f32;
                        transform_stroke_point(point, transform, view_size)
                    })
                    .collect::<Vec<_>>();
                render_fineliner_raster_points(&mut image, &points, active.stroke.color);
            }
        }
        Ok(image)
    }

    fn restore_state(&mut self) -> io::Result<()> {
        self.library = restore_document_library(
            &self.state_path,
            self.width(),
            self.height(),
            TOOLBAR_HEIGHT,
        )?;
        save_document_library(&self.state_path, &self.library)
    }

    fn save_state(&mut self) -> io::Result<()> {
        self.store_open_document_strokes()?;
        save_document_library(&self.state_path, &self.library)
    }

    fn store_open_document_strokes(&mut self) -> io::Result<()> {
        if let Some(open_document) = &self.open_document {
            self.library.store_strokes(
                &open_document.document_id,
                open_document.page.strokes.clone(),
            )?;
        }
        Ok(())
    }

    fn show_document(&mut self, document_id: &str) -> io::Result<()> {
        let page = Page::from_document_page(
            self.library.page(document_id)?,
            self.width(),
            self.height(),
            TOOLBAR_HEIGHT,
        )?;
        self.open_document = Some(OpenDocument {
            document_id: document_id.to_owned(),
            page,
        });
        self.transform = identity_transform(self.width(), self.height());
        self.save_state()?;
        self.redraw_notebook()?;
        self.display.submit_mode_four_color_full();
        Ok(())
    }

    fn show_document_library(&mut self) -> io::Result<()> {
        self.save_state()?;
        self.open_document = None;
        self.library_screen_index = 0;
        self.touch_gestures.reset();
        self.redraw_document_library()
    }

    fn redraw_document_library(&mut self) -> io::Result<()> {
        self.device_status = format_device_status(read_battery().ok(), read_wifi_connection());
        self.last_device_status_read = Instant::now();
        self.redraw_document_library_from_current_status()
    }

    fn redraw_document_library_from_current_status(&mut self) -> io::Result<()> {
        let documents = self.library.summaries();
        draw_document_library(
            &mut self.image,
            &documents,
            self.library_screen_index,
            &self.device_status,
        );
        let full = Rectangle::full(self.image.width(), self.image.height());
        self.display.copy_changed_from(&self.image, full)?;
        self.display.submit_mode_four_color_full();
        Ok(())
    }

    fn redraw_device_status(&mut self) -> io::Result<()> {
        draw_device_status(&mut self.image, &self.device_status);
        let changed = self
            .display
            .copy_changed_from(&self.image, DEVICE_STATUS_RECTANGLE)?;
        self.display.submit_mode_four_color(changed);
        Ok(())
    }

    fn apply_document_library_action(&mut self, action: DocumentLibraryAction) -> io::Result<bool> {
        match action {
            DocumentLibraryAction::CreateNotebook => {
                self.create_notebook()?;
            }
            DocumentLibraryAction::ExitApplication => return Ok(true),
            DocumentLibraryAction::OpenDocument(document_id) => {
                self.open_document(&document_id)?;
            }
            DocumentLibraryAction::PreviousScreen => {
                self.library_screen_index = self.library_screen_index.saturating_sub(1);
                self.redraw_document_library()?;
            }
            DocumentLibraryAction::NextScreen => {
                self.library_screen_index += 1;
                self.redraw_document_library()?;
            }
            DocumentLibraryAction::None => {}
        }
        Ok(false)
    }

    fn redraw_toolbar(&mut self) -> io::Result<()> {
        self.draw_toolbar_into_image()?;
        let toolbar = Rectangle {
            x: 0,
            y: 0,
            width: self.image.width(),
            height: TOOLBAR_HEIGHT,
        };
        let changed = self.display.copy_changed_from(&self.image, toolbar)?;
        self.display.submit_mode_three_color(changed);
        Ok(())
    }

    fn draw_toolbar_into_image(&mut self) -> io::Result<()> {
        let document = self.library.document_summary(self.open_document_id()?)?;
        draw_toolbar(
            &mut self.image,
            self.fineliner_thickness,
            self.color,
            document.page_number,
            document.page_count,
        );
        Ok(())
    }

    fn apply_toolbar_action(&mut self, action: ToolbarAction) -> io::Result<()> {
        match action {
            ToolbarAction::SelectThickness(thickness) => self.fineliner_thickness = thickness,
            ToolbarAction::SelectColor(color) => self.color = color,
            ToolbarAction::ShowLibrary => {
                self.show_document_library()?;
            }
            ToolbarAction::InsertBlankPage => {
                self.insert_blank_page()?;
            }
            ToolbarAction::None => {}
        }
        Ok(())
    }

    fn view_to_scene(&self, point: Point) -> Point {
        self.transform.view_to_scene(point, self.view_size())
    }

    fn view_size(&self) -> Size {
        Size {
            width: self.display.width() as f64,
            height: self.display.height() as f64,
        }
    }

    fn drawable_view_bounds(&self) -> Bounds {
        Bounds {
            origin: Point {
                x: 0.0,
                y: TOOLBAR_HEIGHT as f64,
            },
            size: Size {
                width: self.width() as f64,
                height: self.height().saturating_sub(TOOLBAR_HEIGHT) as f64,
            },
        }
    }

    fn page_bounds(&self) -> io::Result<Bounds> {
        let rectangle = self.page()?.rectangle;
        Ok(Bounds {
            origin: Point {
                x: rectangle.x as f64,
                y: rectangle.y as f64,
            },
            size: Size {
                width: rectangle.width as f64,
                height: rectangle.height as f64,
            },
        })
    }

    fn page_contains_drawable_view_point(&self, point: Point) -> io::Result<bool> {
        Ok(self.drawable_view_bounds().contains(point)
            && self.page_bounds()?.contains(self.view_to_scene(point)))
    }

    fn open_document_id(&self) -> io::Result<&str> {
        self.open_document
            .as_ref()
            .map(|document| document.document_id.as_str())
            .ok_or_else(|| io::Error::other("no document is open"))
    }

    fn page(&self) -> io::Result<&Page> {
        self.open_document
            .as_ref()
            .map(|document| &document.page)
            .ok_or_else(|| io::Error::other("no document is open"))
    }

    fn page_mut(&mut self) -> io::Result<&mut Page> {
        self.open_document
            .as_mut()
            .map(|document| &mut document.page)
            .ok_or_else(|| io::Error::other("no document is open"))
    }
}
