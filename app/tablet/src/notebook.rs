use crate::bgra_image::BgraImage;
use crate::color::Color;
use crate::display::{QuillDisplay, Rectangle};
use crate::draw_toolbar::{HEIGHT as TOOLBAR_HEIGHT, draw_toolbar};
use crate::draw_viewport_indicators::draw_viewport_indicators;
use crate::erase_strokes::{EraserThickness, erase_stroke};
use crate::filter_touch_sequences::RejectPalmContactSequences;
use crate::fineliner::{FinelinerStrokeBuilder, FinelinerThickness};
use crate::input::{PenFrame, PenTool, TouchFrame};
use crate::render_fineliner::{
    FinelinerRasterPoint, FinelinerRasterizer, nonzero_coverage_rectangle,
    raster_width_from_stored_quarters, render_fineliner_raster_points,
};
use crate::stroke::{PenSample, Stroke, StrokePoint};
use crate::toolbar::{ToolbarAction, map_x_to_action};
use crate::view_transform::{Bounds, Point, Size, ViewTransform, centroid, two_finger_scale};
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PAGE_BACKGROUND: [u8; 3] = [0xff, 0xff, 0xff];
const PINCH_RENDER_INTERVAL: Duration = Duration::from_millis(16);

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
    Toolbar,
}

struct ImageBackup {
    rectangle: Rectangle,
    pixels: Vec<u8>,
}

pub struct Notebook {
    display: Arc<QuillDisplay>,
    image: BgraImage,
    strokes: Vec<Stroke>,
    selected_tool: DrawingTool,
    fineliner_thickness: FinelinerThickness,
    eraser_thickness: EraserThickness,
    color: Color,
    transform: ViewTransform,
    active_stroke: Option<ActiveStroke>,
    pen_proximity: bool,
    reject_palm_contact_sequences: RejectPalmContactSequences,
    previous_pinch: Option<[Point; 2]>,
    last_pinch_render: Instant,
}

impl Notebook {
    pub fn new(display: Arc<QuillDisplay>) -> io::Result<Self> {
        let width = display.width();
        let height = display.height();
        let image = BgraImage::filled(width, height, PAGE_BACKGROUND);
        let mut notebook = Self {
            display,
            image,
            strokes: Vec::new(),
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
            last_pinch_render: Instant::now(),
        };
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
        }
        Ok(false)
    }

    pub fn apply_touch_frame(&mut self, frame: TouchFrame) -> io::Result<()> {
        let Some(current) = self
            .reject_palm_contact_sequences
            .accept_two_finger_positions(&frame, self.pen_proximity)
        else {
            if self.previous_pinch.take().is_some() {
                self.redraw_notebook()?;
                self.display.show_color_full();
            }
            return Ok(());
        };

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
                    self.strokes.push(Stroke { points, color });
                }
                if let Some(dirty) = dirty {
                    self.display.copy_from(&self.image, dirty)?;
                    self.display.show_color(dirty);
                }
            }
            Some(ActiveStroke::Eraser { centerline, .. }) => {
                if !centerline.is_empty() {
                    let mut surviving = Vec::new();
                    for stroke in self.strokes.drain(..) {
                        for points in erase_stroke(
                            &stroke.points,
                            &centerline,
                            self.eraser_thickness.pixels(),
                        ) {
                            surviving.push(Stroke {
                                points,
                                color: stroke.color,
                            });
                        }
                    }
                    self.strokes = surviving;
                    self.redraw_notebook()?;
                    self.display.show_color_full();
                }
            }
            Some(ActiveStroke::Toolbar) | None => {}
        }
        Ok(())
    }

    fn redraw_notebook(&mut self) -> io::Result<()> {
        self.image =
            BgraImage::filled(self.display.width(), self.display.height(), PAGE_BACKGROUND);
        for stroke in &self.strokes {
            let points: Vec<_> = stroke
                .points
                .iter()
                .copied()
                .map(|point| transform_point(point, self.transform, self.viewport()))
                .collect();
            render_fineliner_raster_points(&mut self.image, &points, stroke.color);
        }
        draw_toolbar(
            &mut self.image,
            self.selected_tool,
            self.fineliner_thickness,
            self.color,
        );
        if self.previous_pinch.is_some() {
            let viewport = self.viewport();
            draw_viewport_indicators(&mut self.image, self.transform, viewport);
        }
        self.display.copy_from(
            &self.image,
            Rectangle::full(self.image.width(), self.image.height()),
        )
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
            size: self.viewport(),
        }
    }
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
