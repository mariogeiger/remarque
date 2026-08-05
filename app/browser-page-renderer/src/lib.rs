use remarque_core::bgra_image::{BgraImage, PixelRectangle};
use remarque_core::erase_strokes::erase_stroke;
use remarque_core::fineliner::{FinelinerStrokeBuilder, FinelinerThickness};
use remarque_core::render_fineliner::{
    FinelinerRasterizer, nonzero_coverage_rectangle, render_fineliner,
};
use remarque_core::stroke::PenSample;
use remarque_core::view_transform::Point;
use remarque_page_log::{
    AppliedPageOperation, ClientMessage, CommandId, PageCommand, PageJournal, PageOperation,
    PageSnapshot, Participant, ParticipantId, ServerMessage, SharedStroke, StrokeId,
    StrokeReplacement, SubmittedPageOperation, decode_server_message, encode_client_message,
    snapshot_digest,
};
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;

const WHITE: [u8; 3] = [0xff, 0xff, 0xff];

struct LocalStroke {
    id: StrokeId,
    builder: FinelinerStrokeBuilder,
}

#[wasm_bindgen]
pub struct BrowserPageRenderer {
    participant: Option<Participant>,
    journal: Option<PageJournal>,
    image: BgraImage,
    background: Option<BgraImage>,
    rgba: Vec<u8>,
    rasterizers: BTreeMap<StrokeId, FinelinerRasterizer>,
    local_stroke: Option<LocalStroke>,
    fineliner_thickness: FinelinerThickness,
    erase_preview_base: Option<BgraImage>,
    client_instance_nonce: [u8; 16],
    next_identifier: u64,
    dirty: Option<PixelRectangle>,
    needs_snapshot: bool,
    rejection: Option<String>,
    pending_commands: Vec<PageCommand>,
    reconnect_ready: bool,
}

#[wasm_bindgen]
impl BrowserPageRenderer {
    #[wasm_bindgen(constructor)]
    pub fn new(client_instance_nonce: &[u8]) -> Result<Self, JsValue> {
        let client_instance_nonce = client_instance_nonce
            .try_into()
            .map_err(|_| JsValue::from_str("client instance nonce must contain 16 bytes"))?;
        Ok(Self {
            participant: None,
            journal: None,
            image: BgraImage::filled(1, 1, WHITE),
            background: None,
            rgba: vec![0xff; 4],
            rasterizers: BTreeMap::new(),
            local_stroke: None,
            fineliner_thickness: FinelinerThickness::Thin,
            erase_preview_base: None,
            client_instance_nonce,
            next_identifier: 0,
            dirty: Some(PixelRectangle::full(1, 1)),
            needs_snapshot: false,
            rejection: None,
            pending_commands: Vec::new(),
            reconnect_ready: false,
        })
    }

    pub fn apply_server_message(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        let message = decode_server_message(bytes).map_err(js_error)?;
        self.restore_erase_preview()?;
        match message {
            ServerMessage::Welcome {
                protocol_version,
                participant,
                snapshot,
            } => {
                if protocol_version != remarque_page_log::PROTOCOL_VERSION {
                    return Err(JsValue::from_str("unsupported page protocol version"));
                }
                self.queue_unfinished_stroke_cancellation()?;
                self.participant = Some(participant);
                self.replace_snapshot(snapshot)?;
                self.reconnect_ready = true;
            }
            ServerMessage::Applied { operation } => {
                self.pending_commands
                    .retain(|command| command.id != operation.command_id);
                if self
                    .journal
                    .as_ref()
                    .is_none_or(|journal| operation.revision > journal.snapshot().revision)
                {
                    self.apply_operation(operation)?;
                }
            }
            ServerMessage::Snapshot { snapshot } => self.replace_snapshot(snapshot)?,
            ServerMessage::Digest { revision, digest } => {
                let Some(journal) = &self.journal else {
                    self.needs_snapshot = true;
                    return Ok(());
                };
                self.needs_snapshot = revision != journal.snapshot().revision
                    || digest != snapshot_digest(journal.snapshot());
            }
            ServerMessage::Rejected { command, reason } => {
                if let Some(command_id) = command {
                    self.pending_commands
                        .retain(|command| command.id != command_id);
                }
                self.needs_snapshot = true;
                self.rejection = Some(reason);
            }
        }
        Ok(())
    }

    pub fn set_background_bgra(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        let snapshot = self.snapshot()?;
        let width = snapshot.dimensions.width as usize;
        let height = snapshot.dimensions.height as usize;
        self.background = Some(
            BgraImage::try_from_bgra(width, height, bytes.to_vec()).map_err(JsValue::from_str)?,
        );
        self.redraw_snapshot()
    }

    pub fn begin_stroke(&mut self) -> Result<Vec<u8>, JsValue> {
        if self.local_stroke.is_some() {
            return Err(JsValue::from_str("a local stroke is already active"));
        }
        let stroke_id = StrokeId::from_bytes(self.next_identifier(b"stroke"));
        self.local_stroke = Some(LocalStroke {
            id: stroke_id,
            builder: FinelinerStrokeBuilder::new(self.fineliner_thickness),
        });
        self.encode_command(SubmittedPageOperation::BeginStroke { stroke_id })
    }

    pub fn set_fineliner_thickness(&mut self, preset: u8) -> Result<(), JsValue> {
        self.fineliner_thickness = match preset {
            0 => FinelinerThickness::Thin,
            1 => FinelinerThickness::Medium,
            2 => FinelinerThickness::Thick,
            3 => FinelinerThickness::ExtraThick,
            _ => return Err(JsValue::from_str("unknown fineliner thickness preset")),
        };
        Ok(())
    }

    pub fn append_samples(&mut self, coordinates_and_pressure: &[f32]) -> Result<Vec<u8>, JsValue> {
        if coordinates_and_pressure.is_empty() || !coordinates_and_pressure.len().is_multiple_of(3)
        {
            return Err(JsValue::from_str(
                "stroke samples must contain x, y, pressure triples",
            ));
        }
        let local = self
            .local_stroke
            .as_mut()
            .ok_or_else(|| JsValue::from_str("no local stroke is active"))?;
        let first_point = u32::try_from(local.builder.points().len())
            .map_err(|_| JsValue::from_str("stroke point count exceeds protocol limits"))?;
        let points = coordinates_and_pressure
            .chunks_exact(3)
            .map(|sample| {
                local.builder.append_sample(
                    PenSample {
                        x: sample[0],
                        y: sample[1],
                        pressure: sample[2],
                    },
                    1.0,
                )
            })
            .collect();
        let stroke_id = local.id;
        self.encode_command(SubmittedPageOperation::AppendStrokePoints {
            stroke_id,
            first_point,
            points,
        })
    }

    pub fn commit_stroke(&mut self) -> Result<Vec<u8>, JsValue> {
        let local = self
            .local_stroke
            .take()
            .ok_or_else(|| JsValue::from_str("no local stroke is active"))?;
        self.encode_command(SubmittedPageOperation::CommitStroke {
            stroke_id: local.id,
        })
    }

    pub fn cancel_stroke(&mut self) -> Result<Vec<u8>, JsValue> {
        let local = self
            .local_stroke
            .take()
            .ok_or_else(|| JsValue::from_str("no local stroke is active"))?;
        self.encode_command(SubmittedPageOperation::CancelStroke {
            stroke_id: local.id,
        })
    }

    pub fn erase_with_centerline(
        &mut self,
        coordinates: &[f64],
        width: f64,
    ) -> Result<Vec<u8>, JsValue> {
        let centerline = eraser_centerline(coordinates, width)?;
        let participant = self
            .participant
            .ok_or_else(|| JsValue::from_str("participant has not joined the page"))?;
        let strokes = self.snapshot()?.strokes.clone();
        let mut replacements = Vec::new();
        for stroke in strokes
            .into_iter()
            .filter(|stroke| stroke.author == participant.id)
        {
            let surviving = erase_stroke(&stroke.points, &centerline, width);
            if surviving.len() == 1 && surviving[0] == stroke.points {
                continue;
            }
            let fragments = surviving
                .into_iter()
                .map(|points| SharedStroke {
                    id: StrokeId::from_bytes(self.next_identifier(b"erased-stroke")),
                    author: stroke.author,
                    color: stroke.color,
                    points,
                })
                .collect();
            replacements.push(StrokeReplacement {
                removed: stroke.id,
                fragments,
            });
        }
        if replacements.is_empty() {
            self.restore_erase_preview()?;
            return Ok(Vec::new());
        }
        self.encode_command(SubmittedPageOperation::ReplaceStrokes { replacements })
    }

    pub fn preview_erase_with_centerline(
        &mut self,
        coordinates: &[f64],
        width: f64,
    ) -> Result<(), JsValue> {
        let centerline = eraser_centerline(coordinates, width)?;
        let participant = self
            .participant
            .ok_or_else(|| JsValue::from_str("participant has not joined the page"))?;
        if self.erase_preview_base.is_none() {
            self.erase_preview_base = Some(self.image.clone());
        }
        let snapshot = self.snapshot()?.clone();
        let preview =
            self.render_snapshot_with_erase_preview(&snapshot, participant.id, &centerline, width);
        self.replace_visible_image(preview)
    }

    pub fn cancel_erase_preview(&mut self) -> Result<(), JsValue> {
        self.restore_erase_preview()
    }

    pub fn request_snapshot(&self) -> Result<Vec<u8>, JsValue> {
        encode_client_message(&ClientMessage::RequestSnapshot).map_err(js_error)
    }

    pub fn acknowledge(&self) -> Result<Vec<u8>, JsValue> {
        let revision = self.snapshot()?.revision;
        encode_client_message(&ClientMessage::Acknowledge { revision }).map_err(js_error)
    }

    pub fn width(&self) -> u32 {
        self.image.width() as u32
    }

    pub fn height(&self) -> u32 {
        self.image.height() as u32
    }

    pub fn rgba_pointer(&self) -> *const u8 {
        self.rgba.as_ptr()
    }

    pub fn rgba_pixels(&self) -> js_sys::Uint8ClampedArray {
        unsafe { js_sys::Uint8ClampedArray::view(&self.rgba) }
    }

    pub fn rgba_length(&self) -> usize {
        self.rgba.len()
    }

    pub fn dirty_x(&self) -> u32 {
        self.dirty.map_or(0, |rectangle| rectangle.x as u32)
    }

    pub fn dirty_y(&self) -> u32 {
        self.dirty.map_or(0, |rectangle| rectangle.y as u32)
    }

    pub fn dirty_width(&self) -> u32 {
        self.dirty.map_or(0, |rectangle| rectangle.width as u32)
    }

    pub fn dirty_height(&self) -> u32 {
        self.dirty.map_or(0, |rectangle| rectangle.height as u32)
    }

    pub fn clear_dirty_rectangle(&mut self) {
        self.dirty = None;
    }

    pub fn needs_snapshot(&self) -> bool {
        self.needs_snapshot
    }

    pub fn ready(&self) -> bool {
        self.participant.is_some() && self.journal.is_some()
    }

    pub fn participant_color(&self) -> String {
        let rgb = self
            .participant
            .map_or([0, 0, 0], |participant| participant.color.rgb());
        format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
    }

    pub fn background_digest(&self) -> Option<String> {
        self.journal
            .as_ref()?
            .snapshot()
            .background
            .as_ref()
            .map(|background| encode_hex(&background.digest))
    }

    pub fn take_rejection(&mut self) -> Option<String> {
        self.rejection.take()
    }

    pub fn take_reconnect_ready(&mut self) -> bool {
        std::mem::take(&mut self.reconnect_ready)
    }

    pub fn pending_messages(&self) -> Result<js_sys::Array, JsValue> {
        let messages = js_sys::Array::new();
        for command in &self.pending_commands {
            let bytes = encode_client_message(&ClientMessage::Submit {
                command: command.clone(),
            })
            .map_err(js_error)?;
            messages.push(&js_sys::Uint8Array::from(bytes.as_slice()));
        }
        Ok(messages)
    }
}

impl Default for BrowserPageRenderer {
    fn default() -> Self {
        Self::new(&[0; 16]).expect("the default client instance nonce has the required length")
    }
}

impl BrowserPageRenderer {
    fn replace_snapshot(&mut self, snapshot: PageSnapshot) -> Result<(), JsValue> {
        self.journal = Some(PageJournal::from_snapshot(snapshot).map_err(js_error)?);
        self.local_stroke = None;
        self.needs_snapshot = false;
        self.redraw_snapshot()
    }

    fn redraw_snapshot(&mut self) -> Result<(), JsValue> {
        self.erase_preview_base = None;
        let snapshot = self.snapshot()?.clone();
        let width = snapshot.dimensions.width as usize;
        let height = snapshot.dimensions.height as usize;
        self.image = self
            .background
            .clone()
            .filter(|background| background.width() == width && background.height() == height)
            .unwrap_or_else(|| BgraImage::filled(width, height, WHITE));
        for stroke in &snapshot.strokes {
            render_fineliner(&mut self.image, &stroke.points, stroke.color);
        }
        self.rasterizers.clear();
        for active in &snapshot.active_strokes {
            let mut rasterizer = FinelinerRasterizer::new(active.stroke.color);
            for point in &active.stroke.points {
                rasterizer.append_point(&mut self.image, *point);
            }
            self.rasterizers.insert(active.stroke.id, rasterizer);
        }
        self.resize_rgba();
        self.copy_bgra_to_rgba(PixelRectangle::full(width, height));
        self.dirty = Some(PixelRectangle::full(width, height));
        Ok(())
    }

    fn render_snapshot_with_erase_preview(
        &self,
        snapshot: &PageSnapshot,
        participant_id: ParticipantId,
        centerline: &[Point],
        eraser_width: f64,
    ) -> BgraImage {
        let width = snapshot.dimensions.width as usize;
        let height = snapshot.dimensions.height as usize;
        let mut image = self
            .background
            .clone()
            .filter(|background| background.width() == width && background.height() == height)
            .unwrap_or_else(|| BgraImage::filled(width, height, WHITE));
        for stroke in &snapshot.strokes {
            if stroke.author == participant_id {
                for points in erase_stroke(&stroke.points, centerline, eraser_width) {
                    render_fineliner(&mut image, &points, stroke.color);
                }
            } else {
                render_fineliner(&mut image, &stroke.points, stroke.color);
            }
        }
        for active in &snapshot.active_strokes {
            let mut rasterizer = FinelinerRasterizer::new(active.stroke.color);
            for point in &active.stroke.points {
                rasterizer.append_point(&mut image, *point);
            }
        }
        image
    }

    fn replace_visible_image(&mut self, image: BgraImage) -> Result<(), JsValue> {
        let width = self.image.width();
        let height = self.image.height();
        let changed = image
            .difference_rectangle_against_strided_bgra(
                self.image.pixels(),
                width * 4,
                PixelRectangle::full(width, height),
            )
            .map_err(JsValue::from_str)?;
        self.image = image;
        if let Some(changed) = changed {
            self.copy_bgra_to_rgba(changed);
            self.dirty = Some(include(self.dirty, changed));
        }
        Ok(())
    }

    fn restore_erase_preview(&mut self) -> Result<(), JsValue> {
        let Some(base) = self.erase_preview_base.take() else {
            return Ok(());
        };
        self.replace_visible_image(base)
    }

    fn apply_operation(&mut self, operation: AppliedPageOperation) -> Result<(), JsValue> {
        let page_operation = operation.operation.clone();
        self.journal_mut()?.apply(operation).map_err(js_error)?;
        let changed = match page_operation {
            PageOperation::BeginStroke { stroke } => {
                self.rasterizers
                    .insert(stroke.id, FinelinerRasterizer::new(stroke.color));
                None
            }
            PageOperation::AppendStrokePoints {
                stroke_id, points, ..
            } => {
                let width = self.image.width();
                let height = self.image.height();
                let rasterizer = self
                    .rasterizers
                    .get_mut(&stroke_id)
                    .ok_or_else(|| JsValue::from_str("active stroke has no rasterizer"))?;
                let mut changed = None;
                let mut previous = self
                    .journal
                    .as_ref()
                    .and_then(|journal| {
                        journal
                            .snapshot()
                            .active_strokes
                            .iter()
                            .find(|active| active.stroke.id == stroke_id)
                    })
                    .and_then(|active| {
                        active
                            .stroke
                            .points
                            .get(active.stroke.points.len().saturating_sub(points.len() + 1))
                    })
                    .copied();
                for point in points {
                    rasterizer.append_point(&mut self.image, point);
                    let rectangle = nonzero_coverage_rectangle(
                        previous.unwrap_or(point).into(),
                        point.into(),
                        width,
                        height,
                    );
                    changed = Some(include(changed, rectangle));
                    previous = Some(point);
                }
                changed
            }
            PageOperation::CommitStroke { stroke_id } => {
                let mut rasterizer = self
                    .rasterizers
                    .remove(&stroke_id)
                    .ok_or_else(|| JsValue::from_str("committed stroke has no rasterizer"))?;
                rasterizer.finish(&mut self.image);
                self.journal
                    .as_ref()
                    .and_then(|journal| {
                        journal
                            .snapshot()
                            .strokes
                            .iter()
                            .find(|stroke| stroke.id == stroke_id)
                    })
                    .and_then(|stroke| stroke.points.last())
                    .map(|point| {
                        nonzero_coverage_rectangle(
                            (*point).into(),
                            (*point).into(),
                            self.image.width(),
                            self.image.height(),
                        )
                    })
            }
            PageOperation::CancelStroke { stroke_id } => {
                self.rasterizers.remove(&stroke_id);
                self.redraw_snapshot()?;
                return Ok(());
            }
            PageOperation::ReplaceStrokes { .. } => {
                self.redraw_snapshot()?;
                return Ok(());
            }
        };
        if let Some(changed) = changed {
            self.copy_bgra_to_rgba(changed);
            self.dirty = Some(include(self.dirty, changed));
        }
        Ok(())
    }

    fn encode_command(&mut self, operation: SubmittedPageOperation) -> Result<Vec<u8>, JsValue> {
        self.participant
            .ok_or_else(|| JsValue::from_str("participant has not joined the page"))?;
        let command = PageCommand {
            id: CommandId::from_bytes(self.next_identifier(b"command")),
            operation,
        };
        self.pending_commands.push(command.clone());
        encode_client_message(&ClientMessage::Submit { command }).map_err(js_error)
    }

    fn queue_unfinished_stroke_cancellation(&mut self) -> Result<(), JsValue> {
        let Some(local) = self.local_stroke.take() else {
            return Ok(());
        };
        self.encode_command(SubmittedPageOperation::CancelStroke {
            stroke_id: local.id,
        })?;
        Ok(())
    }

    fn next_identifier(&mut self, domain: &[u8]) -> [u8; 16] {
        let mut input = Vec::with_capacity(16 + 16 + 8 + domain.len());
        if let Some(participant) = self.participant {
            input.extend_from_slice(&participant.id.bytes());
        }
        input.extend_from_slice(&self.client_instance_nonce);
        input.extend_from_slice(&self.next_identifier.to_le_bytes());
        input.extend_from_slice(domain);
        self.next_identifier = self.next_identifier.wrapping_add(1);
        let digest = blake3::hash(&input);
        digest.as_bytes()[..16].try_into().unwrap()
    }

    fn snapshot(&self) -> Result<&PageSnapshot, JsValue> {
        self.journal
            .as_ref()
            .map(PageJournal::snapshot)
            .ok_or_else(|| JsValue::from_str("page snapshot has not arrived"))
    }

    fn journal_mut(&mut self) -> Result<&mut PageJournal, JsValue> {
        self.journal
            .as_mut()
            .ok_or_else(|| JsValue::from_str("page snapshot has not arrived"))
    }

    fn resize_rgba(&mut self) {
        self.rgba.resize(self.image.pixels().len(), 0xff);
    }

    fn copy_bgra_to_rgba(&mut self, rectangle: PixelRectangle) {
        let width = self.image.width();
        let right = rectangle.x.saturating_add(rectangle.width).min(width);
        let bottom = rectangle
            .y
            .saturating_add(rectangle.height)
            .min(self.image.height());
        for y in rectangle.y.min(bottom)..bottom {
            for x in rectangle.x.min(right)..right {
                let offset = (y * width + x) * 4;
                let bgra = &self.image.pixels()[offset..offset + 4];
                self.rgba[offset..offset + 4].copy_from_slice(&[bgra[2], bgra[1], bgra[0], 0xff]);
            }
        }
    }
}

fn include(accumulated: Option<PixelRectangle>, rectangle: PixelRectangle) -> PixelRectangle {
    accumulated.map_or(rectangle, |accumulated| accumulated.include(rectangle))
}

fn eraser_centerline(coordinates: &[f64], width: f64) -> Result<Vec<Point>, JsValue> {
    if coordinates.len() < 2
        || !coordinates.len().is_multiple_of(2)
        || !width.is_finite()
        || width <= 0.0
        || coordinates.iter().any(|coordinate| !coordinate.is_finite())
    {
        return Err(JsValue::from_str("eraser path or width is invalid"));
    }
    Ok(coordinates
        .chunks_exact(2)
        .map(|point| Point {
            x: point[0],
            y: point[1],
        })
        .collect())
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(text, "{byte:02x}").expect("writing to a string cannot fail");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use remarque_core::color::Color;
    use remarque_core::stroke::StrokePoint;
    use remarque_page_log::{
        PROTOCOL_VERSION, PageDimensions, PageIdentity, ParticipantId, decode_client_message,
    };

    fn welcome() -> Vec<u8> {
        encode_server_message_for_test(ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            participant: Participant {
                id: ParticipantId::from_bytes([1; 16]),
                role: remarque_page_log::ParticipantRole::Editor,
                color: Color::Red,
            },
            snapshot: PageSnapshot {
                identity: PageIdentity {
                    document_id: "notebook-1".to_owned(),
                    page_index: 0,
                },
                dimensions: PageDimensions {
                    width: 32,
                    height: 48,
                },
                background: None,
                strokes: Vec::new(),
                active_strokes: Vec::new(),
                revision: 0,
            },
        })
    }

    fn encode_server_message_for_test(message: ServerMessage) -> Vec<u8> {
        remarque_page_log::encode_server_message(&message).unwrap()
    }

    fn welcome_with_owned_stroke() -> Vec<u8> {
        let participant_id = ParticipantId::from_bytes([1; 16]);
        encode_server_message_for_test(ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            participant: Participant {
                id: participant_id,
                role: remarque_page_log::ParticipantRole::Editor,
                color: Color::Red,
            },
            snapshot: PageSnapshot {
                identity: PageIdentity {
                    document_id: "notebook-1".to_owned(),
                    page_index: 0,
                },
                dimensions: PageDimensions {
                    width: 32,
                    height: 48,
                },
                background: None,
                strokes: vec![SharedStroke {
                    id: StrokeId::from_bytes([2; 16]),
                    author: participant_id,
                    color: Color::Red,
                    points: vec![
                        StrokePoint {
                            x: 4.0,
                            y: 24.0,
                            two_segment_distance_quarters: 0,
                            width_quarter_pixels: 8,
                            direction: 0,
                            pressure: 255,
                        },
                        StrokePoint {
                            x: 28.0,
                            y: 24.0,
                            two_segment_distance_quarters: 96,
                            width_quarter_pixels: 8,
                            direction: 0,
                            pressure: 255,
                        },
                    ],
                }],
                active_strokes: Vec::new(),
                revision: 0,
            },
        })
    }

    #[test]
    fn welcome_sizes_the_pixel_buffer() {
        let mut renderer = BrowserPageRenderer::new(&[3; 16]).unwrap();
        renderer.apply_server_message(&welcome()).unwrap();
        assert_eq!((renderer.width(), renderer.height()), (32, 48));
        assert_eq!(renderer.rgba_length(), 32 * 48 * 4);
    }

    #[test]
    fn local_commands_use_distinct_identifiers() {
        let mut renderer = BrowserPageRenderer::new(&[3; 16]).unwrap();
        renderer.apply_server_message(&welcome()).unwrap();
        let begin = renderer.begin_stroke().unwrap();
        let append = renderer.append_samples(&[1.0, 2.0, 0.5]).unwrap();
        assert_ne!(begin, append);
    }

    #[test]
    fn reconnect_queues_cancellation_after_unfinished_stroke_commands() {
        let mut renderer = BrowserPageRenderer::new(&[3; 16]).unwrap();
        renderer.apply_server_message(&welcome()).unwrap();
        renderer.begin_stroke().unwrap();
        renderer.apply_server_message(&welcome()).unwrap();
        assert!(renderer.local_stroke.is_none());
        assert!(matches!(
            renderer.pending_commands.last().unwrap().operation,
            SubmittedPageOperation::CancelStroke { .. }
        ));
    }

    #[test]
    fn selected_fineliner_thickness_is_encoded_in_stroke_points() {
        let mut renderer = BrowserPageRenderer::new(&[3; 16]).unwrap();
        renderer.apply_server_message(&welcome()).unwrap();
        renderer.set_fineliner_thickness(3).unwrap();
        renderer.begin_stroke().unwrap();
        let append = renderer.append_samples(&[1.0, 2.0, 0.5]).unwrap();
        let ClientMessage::Submit { command } = decode_client_message(&append).unwrap() else {
            panic!("append did not encode a submitted command");
        };
        let SubmittedPageOperation::AppendStrokePoints { points, .. } = command.operation else {
            panic!("append did not encode stroke points");
        };
        assert_eq!(points[0].width_quarter_pixels, 32);
    }

    #[test]
    fn eraser_preview_changes_only_pixels_and_cancel_restores_them() {
        let mut renderer = BrowserPageRenderer::new(&[3; 16]).unwrap();
        renderer
            .apply_server_message(&welcome_with_owned_stroke())
            .unwrap();
        let before = renderer.rgba.clone();
        let snapshot_before = renderer.snapshot().unwrap().clone();

        renderer
            .preview_erase_with_centerline(&[16.0, 24.0], 8.0)
            .unwrap();

        assert_ne!(renderer.rgba, before);
        assert_eq!(renderer.snapshot().unwrap(), &snapshot_before);
        renderer.cancel_erase_preview().unwrap();
        assert_eq!(renderer.rgba, before);
        assert_eq!(renderer.snapshot().unwrap(), &snapshot_before);
    }

    #[test]
    fn browser_instances_use_distinct_command_identifiers() {
        let mut first = BrowserPageRenderer::new(&[3; 16]).unwrap();
        let mut second = BrowserPageRenderer::new(&[4; 16]).unwrap();
        first.apply_server_message(&welcome()).unwrap();
        second.apply_server_message(&welcome()).unwrap();

        assert_ne!(
            first.begin_stroke().unwrap(),
            second.begin_stroke().unwrap()
        );
    }
}
