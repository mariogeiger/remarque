use crate::battery::{BatteryReading, read_battery};
use crate::bgra_image::BgraImage;
use crate::color::Color;
use crate::display::{QuillDisplay, Rectangle};
use crate::document_library::{DocumentLibrary, restore_document_library, save_document_library};
use crate::draw_document_library::{
    DocumentLibraryAction, document_library_action_at, draw_document_library,
};
use crate::draw_sleep_screen::draw_sleep_screen;
use crate::draw_toolbar::{HEIGHT as TOOLBAR_HEIGHT, draw_toolbar};
use crate::draw_viewport_indicators::{
    draw_viewport_indicators, viewport_indicators_visible_at_scale,
};
use crate::edge_page_swipe::{page_delta_from_edge_swipe, starts_at_page_edge};
use crate::erase_strokes::{EraserThickness, erase_stroke};
use crate::export_document_pages::export_document_pages;
use crate::fineliner::{FinelinerStrokeBuilder, FinelinerThickness};
use crate::input::{PenFrame, PenTool, TouchFrame};
use crate::page::Page;
use crate::pdfium::read_pdf_page_sizes;
use crate::render_fineliner::{FinelinerRasterizer, render_fineliner_raster_points};
use crate::render_page_view::{
    eraser_preview_point, fineliner_segment_rectangle, identity_transform, midpoint,
    rectangle_contains_point, transform_background_nearest_neighbor, transform_stroke_point,
};
use crate::stroke::{PenSample, Stroke};
use crate::toolbar::{ToolbarAction, toolbar_action_at_x};
use crate::touch_gesture::{OneFingerGesture, TouchGestureEvent, TouchGestureRecognizer};
use crate::touch_tap::TapSurface;
use crate::view_transform::{Bounds, Point, Size, ViewTransform, two_finger_scale};
use crate::wifi::{WifiConnection, read_wifi_connection};
use remarque_document::{DocumentSummary, ExportScope};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PINCH_RENDER_INTERVAL: Duration = Duration::from_millis(16);
const DEVICE_STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

enum PenContact {
    Fineliner {
        builder: FinelinerStrokeBuilder,
        color: Color,
        rasterizer: FinelinerRasterizer,
        dirty: Option<Rectangle>,
    },
    Eraser {
        centerline: Vec<Point>,
        cursor: Option<ImageBackup>,
    },
    OutsidePage,
    Toolbar,
    Library,
}

struct ImageBackup {
    rectangle: Rectangle,
    pixels: Vec<u8>,
}

struct OpenDocument {
    document_id: String,
    page: Page,
}

pub struct Notebook {
    display: Arc<QuillDisplay>,
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
    pen_proximity: bool,
    touch_gestures: TouchGestureRecognizer,
    last_pinch_render: Instant,
    battery: Option<BatteryReading>,
    wifi: WifiConnection,
    last_device_status_read: Instant,
}

impl Notebook {
    pub fn new(display: Arc<QuillDisplay>, state_path: PathBuf) -> io::Result<Self> {
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
            pen_proximity: false,
            touch_gestures: TouchGestureRecognizer::default(),
            last_pinch_render: Instant::now(),
            battery: None,
            wifi: WifiConnection::Unavailable,
            last_device_status_read: Instant::now(),
        };
        if let Err(error) = notebook.restore_state() {
            eprintln!("notebook_state_ignored={error}");
        }
        notebook.redraw_document_library()?;
        notebook.display.show_color_full();
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
        self.display.copy_from(&self.image, full)?;
        self.display.show_color_full();
        Ok(())
    }

    pub fn redraw_active_view_with_full_refresh(&mut self) -> io::Result<()> {
        if self.open_document.is_some() {
            self.redraw_notebook()?;
            self.display.show_color_full();
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
        let battery = read_battery().ok();
        let wifi = read_wifi_connection();
        self.last_device_status_read = Instant::now();
        if battery == self.battery && wifi == self.wifi {
            return Ok(());
        }
        self.battery = battery;
        self.wifi = wifi;
        self.redraw_document_library_from_current_status()
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

    pub fn apply_pen_frame(&mut self, frame: PenFrame) -> io::Result<bool> {
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

        if self.active_pen_contact.is_none() {
            if frame.position.y < TOOLBAR_HEIGHT as f64 {
                self.apply_toolbar_action(toolbar_action_at_x(frame.position.x as usize))?;
                self.active_pen_contact = Some(PenContact::Toolbar);
                if self.open_document.is_some() {
                    self.redraw_toolbar()?;
                }
                return Ok(false);
            }
            if !rectangle_contains_point(self.page()?.rectangle, self.view_to_scene(frame.position))
            {
                self.active_pen_contact = Some(PenContact::OutsidePage);
                return Ok(false);
            }
            self.active_pen_contact = Some(match frame.tool {
                PenTool::Tip => PenContact::Fineliner {
                    builder: FinelinerStrokeBuilder::new(self.fineliner_thickness),
                    color: self.color,
                    rasterizer: FinelinerRasterizer::new(self.color),
                    dirty: None,
                },
                PenTool::EraserEnd => PenContact::Eraser {
                    centerline: Vec::new(),
                    cursor: None,
                },
            });
        }

        let scene_position = self.view_to_scene(frame.position);
        let viewport = self.viewport();
        let contact = self
            .active_pen_contact
            .as_mut()
            .ok_or_else(|| io::Error::other("touching pen has no active contact"))?;
        match contact {
            PenContact::Fineliner {
                builder,
                color: _,
                rasterizer,
                dirty,
            } => {
                let point = builder.append_sample(
                    PenSample {
                        x: scene_position.x as f32,
                        y: scene_position.y as f32,
                        pressure: frame.pressure,
                    },
                    self.transform.scale as f32,
                );
                let screen_point = transform_stroke_point(point, self.transform, viewport);
                let screen_previous = builder
                    .points()
                    .get(builder.points().len().saturating_sub(2))
                    .copied()
                    .map(|previous| transform_stroke_point(previous, self.transform, viewport))
                    .unwrap_or(screen_point);
                rasterizer.append_point(&mut self.image, screen_point);
                let changed = fineliner_segment_rectangle(
                    screen_previous,
                    screen_point,
                    self.image.width(),
                    self.image.height(),
                );
                *dirty = Some(dirty.map_or(changed, |dirty| dirty.include(changed)));
                self.display.copy_from(&self.image, changed)?;
                self.display.show_mono_fast(changed);
            }
            PenContact::Eraser { centerline, cursor } => {
                let previous = centerline.last().copied().unwrap_or(scene_position);
                centerline.push(scene_position);
                let width = self.eraser_thickness.pixels() * self.transform.scale;
                let preview = [
                    eraser_preview_point(previous, width, self.transform, viewport),
                    eraser_preview_point(scene_position, width, self.transform, viewport),
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
                self.display.copy_from(&self.image, changed)?;
                self.display.show_mono_fast(changed);
            }
            PenContact::Toolbar | PenContact::Library => {}
            PenContact::OutsidePage => {}
        }
        Ok(false)
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
        let event = self
            .touch_gestures
            .update(&frame, self.pen_proximity, |position| {
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
            Some(TouchGestureEvent::PinchChanged { previous, current }) => {
                if document_is_open {
                    self.apply_pinch_change(previous, current)?;
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

    fn apply_pinch_change(&mut self, previous: [Point; 2], current: [Point; 2]) -> io::Result<()> {
        let Some(factor) = two_finger_scale(previous, current) else {
            return Ok(());
        };
        let previous_centroid = midpoint(previous);
        let current_centroid = midpoint(current);
        let target_scale = (self.transform.scale * factor).clamp(1.0, 5.0);
        let adjusted_factor = target_scale / self.transform.scale;
        if let Some(transform) = self.transform.scale_and_translate(
            previous_centroid,
            current_centroid,
            adjusted_factor,
            self.viewport(),
            self.scene_bounds()?,
        ) {
            self.transform = transform;
        }
        if self.last_pinch_render.elapsed() >= PINCH_RENDER_INTERVAL {
            self.redraw_notebook()?;
            self.display
                .show_mono_fast(Rectangle::full(self.image.width(), self.image.height()));
            self.last_pinch_render = Instant::now();
        }
        Ok(())
    }

    fn finish_pinch(&mut self) -> io::Result<()> {
        self.redraw_notebook()?;
        self.display.show_color_full();
        Ok(())
    }

    fn finish_pen_contact(&mut self) -> io::Result<()> {
        match self.active_pen_contact.take() {
            Some(PenContact::Fineliner {
                builder,
                color,
                mut rasterizer,
                dirty,
            }) => {
                rasterizer.finish(&mut self.image);
                let points = builder.finish();
                if !points.is_empty() {
                    let page_x = self.page()?.rectangle.x as f32;
                    let page_y = self.page()?.rectangle.y as f32;
                    let points = points
                        .into_iter()
                        .map(|mut point| {
                            point.x -= page_x;
                            point.y -= page_y;
                            point
                        })
                        .collect();
                    self.page_mut()?.strokes.push(Stroke { points, color });
                    self.save_state()?;
                }
                if let Some(dirty) = dirty {
                    self.display.copy_from(&self.image, dirty)?;
                    self.display.show_color(dirty);
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
                    self.redraw_notebook()?;
                    self.display.show_color_full();
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

    fn redraw_notebook(&mut self) -> io::Result<()> {
        self.image = self.render_scene(self.transform)?;
        let document = self.library.document_summary(self.open_document_id()?)?;
        draw_toolbar(
            &mut self.image,
            self.fineliner_thickness,
            self.color,
            document.page_number,
            document.page_count,
        );
        if viewport_indicators_visible_at_scale(self.transform.scale) {
            let viewport = self.viewport();
            let scene = self.scene_bounds()?.size;
            draw_viewport_indicators(&mut self.image, self.transform, viewport, scene);
        }
        self.display.copy_from(
            &self.image,
            Rectangle::full(self.image.width(), self.image.height()),
        )
    }

    fn render_scene(&self, transform: ViewTransform) -> io::Result<BgraImage> {
        let viewport = self.viewport();
        let page = self.page()?;
        let background = page.raster_background(self.width(), self.height());
        let mut image = transform_background_nearest_neighbor(
            &background,
            transform,
            viewport,
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
                    transform_stroke_point(point, transform, viewport)
                })
                .collect();
            render_fineliner_raster_points(&mut image, &points, stroke.color);
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
        self.display.show_color_full();
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
        self.battery = read_battery().ok();
        self.wifi = read_wifi_connection();
        self.last_device_status_read = Instant::now();
        self.redraw_document_library_from_current_status()
    }

    fn redraw_document_library_from_current_status(&mut self) -> io::Result<()> {
        let documents = self.library.summaries();
        draw_document_library(
            &mut self.image,
            &documents,
            self.library_screen_index,
            self.battery,
            self.wifi,
        );
        let full = Rectangle::full(self.image.width(), self.image.height());
        self.display.copy_from(&self.image, full)?;
        self.display.show_color_full();
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
        let document = self.library.document_summary(self.open_document_id()?)?;
        draw_toolbar(
            &mut self.image,
            self.fineliner_thickness,
            self.color,
            document.page_number,
            document.page_count,
        );
        let toolbar = Rectangle {
            x: 0,
            y: 0,
            width: self.image.width(),
            height: TOOLBAR_HEIGHT,
        };
        self.display.copy_from(&self.image, toolbar)?;
        self.display.show_color(toolbar);
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
        self.transform.view_to_scene(point, self.viewport())
    }

    fn viewport(&self) -> Size {
        Size {
            width: self.display.width() as f64,
            height: self.display.height() as f64,
        }
    }

    fn scene_bounds(&self) -> io::Result<Bounds> {
        let page = self.page()?;
        Ok(Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: page.scene_width(self.width()) as f64,
                height: page.scene_height(self.height()) as f64,
            },
        })
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
