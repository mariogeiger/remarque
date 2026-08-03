use crate::bgra_image::BgraImage;
use crate::color::Color;
use crate::display::{QuillDisplay, Rectangle};
use crate::draw_toolbar::{HEIGHT as TOOLBAR_HEIGHT, draw_toolbar};
use crate::draw_viewport_indicators::draw_viewport_indicators;
use crate::edge_page_swipe::{page_delta_from_edge_swipe, starts_at_page_edge};
use crate::erase_strokes::{EraserThickness, erase_stroke};
use crate::filter_touch_sequences::RejectPalmContactSequences;
use crate::fineliner::{FinelinerStrokeBuilder, FinelinerThickness};
use crate::input::{PenFrame, PenTool, TouchFrame};
use crate::page_coordinates::migrate_fit_page_strokes_to_fit_width;
use crate::pdfium::{read_pdf_page_sizes, render_pdf_page};
use crate::render_fineliner::{
    FinelinerRasterPoint, FinelinerRasterizer, nonzero_coverage_rectangle,
    raster_width_from_stored_quarters, render_fineliner_raster_points,
};
use crate::stroke::{PenSample, Stroke, StrokePoint};
use crate::toolbar::{ToolbarAction, map_x_to_action};
use crate::view_transform::{Bounds, Point, Size, ViewTransform, centroid, two_finger_scale};
use remarque_document::{
    CurrentDocument, read_json, write_bgra_page_as_pdf, write_json_atomically,
};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PAGE_BACKGROUND: [u8; 3] = [0xff, 0xff, 0xff];
const PINCH_RENDER_INTERVAL: Duration = Duration::from_millis(16);
const NOTEBOOK_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DrawingTool {
    Fineliner,
    Eraser,
}

enum ActiveStroke {
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
}

struct ImageBackup {
    rectangle: Rectangle,
    pixels: Vec<u8>,
}

struct EdgeSwipe {
    start: Point,
    current: Point,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OpenDocument {
    source_path: PathBuf,
    display_name: String,
    page_number: u32,
    page_count: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredNotebook {
    #[serde(default)]
    format_version: u32,
    document: OpenDocument,
    #[serde(default)]
    pages: Vec<Vec<Stroke>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    strokes: Vec<Stroke>,
}

pub struct Notebook {
    display: Arc<QuillDisplay>,
    image: BgraImage,
    page_background: BgraImage,
    page_rectangle: Rectangle,
    page_size_points: [f64; 2],
    strokes: Vec<Stroke>,
    page_strokes: Vec<Vec<Stroke>>,
    document: Option<OpenDocument>,
    state_path: PathBuf,
    selected_tool: DrawingTool,
    fineliner_thickness: FinelinerThickness,
    eraser_thickness: EraserThickness,
    color: Color,
    transform: ViewTransform,
    active_stroke: Option<ActiveStroke>,
    pen_proximity: bool,
    reject_palm_contact_sequences: RejectPalmContactSequences,
    previous_pinch: Option<[Point; 2]>,
    edge_swipe: Option<EdgeSwipe>,
    last_pinch_render: Instant,
}

impl Notebook {
    pub fn new(display: Arc<QuillDisplay>, state_path: PathBuf) -> io::Result<Self> {
        let width = display.width();
        let height = display.height();
        let image = BgraImage::filled(width, height, PAGE_BACKGROUND);
        let mut notebook = Self {
            display,
            page_background: image.clone(),
            page_rectangle: Rectangle {
                x: 0,
                y: TOOLBAR_HEIGHT,
                width,
                height: height - TOOLBAR_HEIGHT,
            },
            page_size_points: [width as f64, (height - TOOLBAR_HEIGHT) as f64],
            image,
            strokes: Vec::new(),
            page_strokes: Vec::new(),
            document: None,
            state_path,
            selected_tool: DrawingTool::Fineliner,
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
            active_stroke: None,
            pen_proximity: false,
            reject_palm_contact_sequences: RejectPalmContactSequences::default(),
            previous_pinch: None,
            edge_swipe: None,
            last_pinch_render: Instant::now(),
        };
        if let Err(error) = notebook.restore_state() {
            eprintln!("notebook_state_ignored={error}");
        }
        notebook.redraw_notebook()?;
        notebook.display.show_color_full();
        Ok(notebook)
    }

    pub fn width(&self) -> usize {
        self.display.width()
    }

    pub fn height(&self) -> usize {
        self.display.height()
    }

    pub fn open_pdf(
        &mut self,
        source_path: &Path,
        display_name: String,
    ) -> io::Result<CurrentDocument> {
        let rendered =
            render_pdf_page(source_path, 0, self.width(), self.height(), TOOLBAR_HEIGHT)?;
        self.page_background = rendered.background;
        self.page_rectangle = rendered.page_rectangle;
        self.page_size_points = rendered.page_size_points;
        self.strokes.clear();
        self.page_strokes = vec![Vec::new(); rendered.page_count as usize];
        self.transform = identity_transform(self.width(), self.height());
        self.document = Some(OpenDocument {
            source_path: source_path.to_owned(),
            display_name,
            page_number: 1,
            page_count: rendered.page_count,
        });
        self.save_state()?;
        self.redraw_notebook()?;
        self.display.show_color_full();
        Ok(self.current_document().unwrap())
    }

    pub fn change_page(&mut self, delta: i32) -> io::Result<CurrentDocument> {
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no PDF is open"))?;
        let current_index = document.page_number.saturating_sub(1) as usize;
        let target_index = (current_index as i64 + i64::from(delta))
            .clamp(0, i64::from(document.page_count) - 1) as usize;
        if target_index == current_index {
            return Ok(self.current_document().unwrap());
        }
        let rendered = render_pdf_page(
            &document.source_path,
            target_index as u32,
            self.width(),
            self.height(),
            TOOLBAR_HEIGHT,
        )?;
        if rendered.page_count != document.page_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PDF page count changed",
            ));
        }
        self.page_strokes[current_index] = self.strokes.clone();
        self.strokes = self.page_strokes[target_index].clone();
        self.page_background = rendered.background;
        self.page_rectangle = rendered.page_rectangle;
        self.page_size_points = rendered.page_size_points;
        self.document.as_mut().unwrap().page_number = target_index as u32 + 1;
        self.transform = identity_transform(self.width(), self.height());
        self.save_state()?;
        self.redraw_notebook()?;
        self.display.show_color_full();
        Ok(self.current_document().unwrap())
    }

    pub fn current_document(&self) -> Option<CurrentDocument> {
        self.document.as_ref().map(|document| CurrentDocument {
            source_path: document.source_path.clone(),
            display_name: document.display_name.clone(),
            page_number: document.page_number,
            page_count: document.page_count,
        })
    }

    pub fn export_current_page(&self, destination: &Path) -> io::Result<()> {
        if self.document.is_none() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no PDF is open"));
        }
        let mut page = BgraImage::try_from_bgra(
            self.page_rectangle.width,
            self.page_rectangle.height,
            self.page_background.copy_rectangle(
                self.page_rectangle.x,
                self.page_rectangle.y,
                self.page_rectangle.width,
                self.page_rectangle.height,
            ),
        )
        .map_err(io::Error::other)?;
        for stroke in &self.strokes {
            let points = stroke
                .points
                .iter()
                .map(|point| FinelinerRasterPoint {
                    x: point.x,
                    y: point.y,
                    width: raster_width_from_stored_quarters(point.width_quarter_pixels, 1.0),
                })
                .collect::<Vec<_>>();
            render_fineliner_raster_points(&mut page, &points, stroke.color);
        }
        write_bgra_page_as_pdf(
            destination,
            page.width(),
            page.height(),
            self.page_size_points,
            page.pixels(),
        )
    }

    pub fn apply_pen_frame(&mut self, frame: PenFrame) -> io::Result<bool> {
        self.pen_proximity = frame.proximity;
        if !frame.touching {
            if self.active_stroke.is_some() {
                self.finish_active_stroke()?;
            }
            return Ok(false);
        }

        if self.active_stroke.is_none() {
            if frame.position.y < TOOLBAR_HEIGHT as f64 {
                if self.apply_toolbar_action(map_x_to_action(frame.position.x as usize)) {
                    return Ok(true);
                }
                self.active_stroke = Some(ActiveStroke::Toolbar);
                self.redraw_toolbar()?;
                return Ok(false);
            }
            if self.document.is_some()
                && !rectangle_contains(self.page_rectangle, self.view_to_scene(frame.position))
            {
                self.active_stroke = Some(ActiveStroke::OutsidePage);
                return Ok(false);
            }
            let selected_tool = if frame.tool == PenTool::EraserEnd {
                DrawingTool::Eraser
            } else {
                self.selected_tool
            };
            self.active_stroke = Some(match selected_tool {
                DrawingTool::Fineliner => ActiveStroke::Fineliner {
                    builder: FinelinerStrokeBuilder::new(self.fineliner_thickness),
                    color: self.color,
                    rasterizer: FinelinerRasterizer::new(self.color),
                    dirty: None,
                },
                DrawingTool::Eraser => ActiveStroke::Eraser {
                    centerline: Vec::new(),
                    cursor: None,
                },
            });
        }

        let scene_position = self.view_to_scene(frame.position);
        let viewport = self.viewport();
        match self.active_stroke.as_mut().unwrap() {
            ActiveStroke::Fineliner {
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
                let screen_point = transform_point(point, self.transform, viewport);
                let screen_previous = builder
                    .points()
                    .get(builder.points().len().saturating_sub(2))
                    .copied()
                    .map(|previous| transform_point(previous, self.transform, viewport))
                    .unwrap_or(screen_point);
                rasterizer.append_point(&mut self.image, screen_point);
                let changed = segment_rectangle(
                    screen_previous,
                    screen_point,
                    self.image.width(),
                    self.image.height(),
                );
                *dirty = Some(dirty.map_or(changed, |dirty| dirty.include(changed)));
                self.display.copy_from(&self.image, changed)?;
                self.display.show_mono_fast(changed);
            }
            ActiveStroke::Eraser { centerline, cursor } => {
                let previous = centerline.last().copied().unwrap_or(scene_position);
                centerline.push(scene_position);
                let width = self.eraser_thickness.pixels() * self.transform.scale;
                let preview = [
                    preview_point(previous, width, self.transform, viewport),
                    preview_point(scene_position, width, self.transform, viewport),
                ];
                let mut changed = segment_rectangle(
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
                let cursor_rectangle = segment_rectangle(
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
            ActiveStroke::Toolbar => {}
            ActiveStroke::OutsidePage => {}
        }
        Ok(false)
    }

    pub fn apply_touch_frame(&mut self, frame: TouchFrame) -> io::Result<()> {
        let Some(points) = self
            .reject_palm_contact_sequences
            .accept_at_most_two_finger_points(&frame, self.pen_proximity)
        else {
            self.edge_swipe = None;
            return self.finish_pinch();
        };
        match points {
            [] => {
                if self.previous_pinch.is_some() {
                    self.edge_swipe = None;
                    return self.finish_pinch();
                }
                if let Some(swipe) = self.edge_swipe.take()
                    && let Some(delta) =
                        page_delta_from_edge_swipe(swipe.start, swipe.current, self.width() as f64)
                {
                    self.change_page(delta)?;
                }
            }
            [point] => {
                if self.previous_pinch.is_some() {
                    return Ok(());
                }
                if let Some(swipe) = &mut self.edge_swipe {
                    swipe.current = point.position;
                } else if self
                    .document
                    .as_ref()
                    .is_some_and(|document| document.page_count > 1)
                    && point.position.y >= TOOLBAR_HEIGHT as f64
                    && starts_at_page_edge(point.position, self.width() as f64)
                {
                    self.edge_swipe = Some(EdgeSwipe {
                        start: point.position,
                        current: point.position,
                    });
                }
            }
            [first, second] => {
                self.edge_swipe = None;
                self.apply_two_finger_positions([first.position, second.position])?;
            }
            _ => unreachable!("touch filter accepts at most two fingers"),
        }
        Ok(())
    }

    fn apply_two_finger_positions(&mut self, current: [Point; 2]) -> io::Result<()> {
        let Some(previous) = self.previous_pinch.replace(current) else {
            return Ok(());
        };
        let Some(factor) = two_finger_scale(previous, current) else {
            return Ok(());
        };
        let previous_centroid = centroid(&previous).unwrap();
        let current_centroid = centroid(&current).unwrap();
        let target_scale = (self.transform.scale * factor).clamp(1.0, 5.0);
        let adjusted_factor = target_scale / self.transform.scale;
        if let Some(transform) = self.transform.scale_and_translate(
            previous_centroid,
            current_centroid,
            adjusted_factor,
            self.viewport(),
            self.scene_bounds(),
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
        if self.previous_pinch.take().is_some() {
            self.redraw_notebook()?;
            self.display.show_color_full();
        }
        Ok(())
    }

    fn finish_active_stroke(&mut self) -> io::Result<()> {
        match self.active_stroke.take() {
            Some(ActiveStroke::Fineliner {
                builder,
                color,
                mut rasterizer,
                dirty,
            }) => {
                rasterizer.finish(&mut self.image);
                let points = builder.finish();
                if !points.is_empty() {
                    let points = points
                        .into_iter()
                        .map(|mut point| {
                            point.x -= self.page_rectangle.x as f32;
                            point.y -= self.page_rectangle.y as f32;
                            point
                        })
                        .collect();
                    self.strokes.push(Stroke { points, color });
                    self.save_state()?;
                }
                if let Some(dirty) = dirty {
                    self.display.copy_from(&self.image, dirty)?;
                    self.display.show_color(dirty);
                }
            }
            Some(ActiveStroke::Eraser { centerline, .. }) => {
                if !centerline.is_empty() {
                    let mut surviving = Vec::new();
                    let local_centerline = centerline
                        .into_iter()
                        .map(|point| Point {
                            x: point.x - self.page_rectangle.x as f64,
                            y: point.y - self.page_rectangle.y as f64,
                        })
                        .collect::<Vec<_>>();
                    for stroke in self.strokes.drain(..) {
                        for points in erase_stroke(
                            &stroke.points,
                            &local_centerline,
                            self.eraser_thickness.pixels(),
                        ) {
                            surviving.push(Stroke {
                                points,
                                color: stroke.color,
                            });
                        }
                    }
                    self.strokes = surviving;
                    self.save_state()?;
                    self.redraw_notebook()?;
                    self.display.show_color_full();
                }
            }
            Some(ActiveStroke::OutsidePage | ActiveStroke::Toolbar) | None => {}
        }
        Ok(())
    }

    fn redraw_notebook(&mut self) -> io::Result<()> {
        self.image = self.render_scene(self.transform);
        draw_toolbar(
            &mut self.image,
            self.selected_tool,
            self.fineliner_thickness,
            self.color,
        );
        if self.previous_pinch.is_some() {
            let viewport = self.viewport();
            let scene = self.scene_bounds().size;
            draw_viewport_indicators(&mut self.image, self.transform, viewport, scene);
        }
        self.display.copy_from(
            &self.image,
            Rectangle::full(self.image.width(), self.image.height()),
        )
    }

    fn render_scene(&self, transform: ViewTransform) -> BgraImage {
        let viewport = self.viewport();
        let mut image = transform_background(
            &self.page_background,
            transform,
            viewport,
            self.width(),
            self.height(),
        );
        for stroke in &self.strokes {
            let points: Vec<_> = stroke
                .points
                .iter()
                .copied()
                .map(|mut point| {
                    point.x += self.page_rectangle.x as f32;
                    point.y += self.page_rectangle.y as f32;
                    transform_point(point, transform, viewport)
                })
                .collect();
            render_fineliner_raster_points(&mut image, &points, stroke.color);
        }
        image
    }

    fn restore_state(&mut self) -> io::Result<()> {
        if !self.state_path.exists() {
            return Ok(());
        }
        let stored: StoredNotebook = read_json(&self.state_path)?;
        let rendered = render_pdf_page(
            &stored.document.source_path,
            stored.document.page_number.saturating_sub(1),
            self.width(),
            self.height(),
            TOOLBAR_HEIGHT,
        )?;
        if rendered.page_count != stored.document.page_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "persisted PDF page count changed",
            ));
        }
        let mut pages = stored.pages;
        let current_index = stored.document.page_number.saturating_sub(1) as usize;
        if pages.len() != rendered.page_count as usize {
            pages = vec![Vec::new(); rendered.page_count as usize];
            pages[current_index] = stored.strokes;
        }
        if stored.format_version == 0 {
            let page_sizes = read_pdf_page_sizes(&stored.document.source_path)?;
            migrate_fit_page_strokes_to_fit_width(
                &mut pages,
                &page_sizes,
                self.width(),
                self.height(),
                TOOLBAR_HEIGHT,
            );
        } else if stored.format_version != NOTEBOOK_FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "notebook format version is unsupported",
            ));
        }
        self.page_background = rendered.background;
        self.page_rectangle = rendered.page_rectangle;
        self.page_size_points = rendered.page_size_points;
        self.strokes = pages[current_index].clone();
        self.page_strokes = pages;
        self.document = Some(stored.document);
        Ok(())
    }

    fn save_state(&self) -> io::Result<()> {
        match &self.document {
            Some(document) => {
                let mut pages = self.page_strokes.clone();
                pages[document.page_number.saturating_sub(1) as usize] = self.strokes.clone();
                write_json_atomically(
                    &self.state_path,
                    &StoredNotebook {
                        format_version: NOTEBOOK_FORMAT_VERSION,
                        document: document.clone(),
                        pages,
                        strokes: Vec::new(),
                    },
                )
            }
            None => Ok(()),
        }
    }

    fn redraw_toolbar(&mut self) -> io::Result<()> {
        draw_toolbar(
            &mut self.image,
            self.selected_tool,
            self.fineliner_thickness,
            self.color,
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

    fn apply_toolbar_action(&mut self, action: ToolbarAction) -> bool {
        match action {
            ToolbarAction::SelectFineliner => self.selected_tool = DrawingTool::Fineliner,
            ToolbarAction::SelectEraser => self.selected_tool = DrawingTool::Eraser,
            ToolbarAction::SelectThickness(thickness) => self.fineliner_thickness = thickness,
            ToolbarAction::SelectColor(color) => self.color = color,
            ToolbarAction::ExitApplication => return true,
            ToolbarAction::None => {}
        }
        false
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

    fn scene_bounds(&self) -> Bounds {
        Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: self.page_background.width() as f64,
                height: self.page_background.height() as f64,
            },
        }
    }
}

fn identity_transform(width: usize, height: usize) -> ViewTransform {
    ViewTransform {
        focal_point: Point {
            x: width as f64 * 0.5,
            y: height as f64 * 0.5,
        },
        scale: 1.0,
    }
}

fn rectangle_contains(rectangle: Rectangle, point: Point) -> bool {
    point.x >= rectangle.x as f64
        && point.y >= rectangle.y as f64
        && point.x < (rectangle.x + rectangle.width) as f64
        && point.y < (rectangle.y + rectangle.height) as f64
}

fn transform_background(
    background: &BgraImage,
    transform: ViewTransform,
    viewport: Size,
    output_width: usize,
    output_height: usize,
) -> BgraImage {
    if background.width() == output_width
        && background.height() == output_height
        && transform == identity_transform(output_width, output_height)
    {
        return background.clone();
    }
    let mut pixels = Vec::with_capacity(output_width * output_height * 4);
    for _ in 0..output_width * output_height {
        pixels.extend_from_slice(&[0xe1, 0xe4, 0xe5, 0xff]);
    }
    for y in TOOLBAR_HEIGHT..output_height {
        let scene_y = transform
            .view_to_scene(
                Point {
                    x: 0.0,
                    y: y as f64,
                },
                viewport,
            )
            .y
            .floor() as isize;
        if scene_y < 0 || scene_y >= background.height() as isize {
            continue;
        }
        for x in 0..output_width {
            let scene_x = transform
                .view_to_scene(
                    Point {
                        x: x as f64,
                        y: 0.0,
                    },
                    viewport,
                )
                .x
                .floor() as isize;
            if scene_x < 0 || scene_x >= background.width() as isize {
                continue;
            }
            let source = (scene_y as usize * background.width() + scene_x as usize) * 4;
            let destination = (y * output_width + x) * 4;
            pixels[destination..destination + 4]
                .copy_from_slice(&background.pixels()[source..source + 4]);
        }
    }
    BgraImage::try_from_bgra(output_width, output_height, pixels).unwrap()
}

fn transform_point(
    point: StrokePoint,
    transform: ViewTransform,
    viewport: Size,
) -> FinelinerRasterPoint {
    FinelinerRasterPoint {
        x: ((f64::from(point.x) - transform.focal_point.x) * transform.scale + viewport.width * 0.5)
            as f32,
        y: ((f64::from(point.y) - transform.focal_point.y) * transform.scale
            + viewport.height * 0.5) as f32,
        width: raster_width_from_stored_quarters(
            point.width_quarter_pixels,
            transform.scale as f32,
        ),
    }
}

fn preview_point(
    position: Point,
    width: f64,
    transform: ViewTransform,
    viewport: Size,
) -> FinelinerRasterPoint {
    FinelinerRasterPoint {
        x: ((position.x - transform.focal_point.x) * transform.scale + viewport.width * 0.5) as f32,
        y: ((position.y - transform.focal_point.y) * transform.scale + viewport.height * 0.5)
            as f32,
        width: 0.75 + width as f32,
    }
}

fn segment_rectangle(
    start: FinelinerRasterPoint,
    end: FinelinerRasterPoint,
    image_width: usize,
    image_height: usize,
) -> Rectangle {
    let bounds = nonzero_coverage_rectangle(start, end, image_width, image_height);
    Rectangle {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}
