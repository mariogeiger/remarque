struct SharedPageSession {
    identity: PageIdentity,
    journal: PageJournal,
    connection: Option<SharedPageConnection>,
    pending_commands: Vec<PageCommand>,
    client_instance_nonce: [u8; 16],
    next_identifier: u64,
    rasterizers: BTreeMap<StrokeId, SharedStrokeRasterizer>,
    optimistic_pending_pixels_visible: bool,
}

struct SharedStrokeRasterizer {
    rasterizer: FinelinerRasterizer,
    previous: Option<crate::stroke::StrokePoint>,
}

enum SharedPageUpdate {
    None,
    FullSnapshot,
    Applied {
        operation: PageOperation,
        locally_drawn: bool,
    },
}

impl Notebook {
    pub fn disconnect_page_share(&mut self) {
        self.shared_page = None;
    }

    pub fn prepare_page_share(
        &mut self,
        destination_directory: &Path,
    ) -> io::Result<(PathBuf, Option<PathBuf>)> {
        self.finish_editing_input_sequences()?;
        self.store_open_document_strokes()?;
        self.save_state()?;
        fs::create_dir_all(destination_directory)?;
        fs::set_permissions(destination_directory, fs::Permissions::from_mode(0o700))?;

        let document_id = self.open_document_id()?.to_owned();
        let page_index = self.library.current_page_index(&document_id)?;
        let identity = PageIdentity {
            document_id,
            page_index: u32::try_from(page_index)
                .map_err(|_| io::Error::other("page index exceeds sharing limits"))?,
        };
        let page = self.page()?;
        let dimensions = PageDimensions {
            width: u32::try_from(page.rectangle.width)
                .map_err(|_| io::Error::other("page width exceeds sharing limits"))?,
            height: u32::try_from(page.rectangle.height)
                .map_err(|_| io::Error::other("page height exceeds sharing limits"))?,
        };
        let background_path = if page.background.is_some() {
            let pixels = page
                .raster_background(self.width(), self.height())
                .copy_rectangle(
                    page.rectangle.x,
                    page.rectangle.y,
                    page.rectangle.width,
                    page.rectangle.height,
                );
            let path = destination_directory.join("page-background.bgra");
            write_bytes_atomically(&path, &pixels)?;
            Some((path, *blake3::hash(&pixels).as_bytes()))
        } else {
            None
        };
        let placeholder_author = ParticipantId::from_bytes([0; 16]);
        let strokes = page
            .strokes
            .iter()
            .enumerate()
            .map(|(index, stroke)| SharedStroke {
                id: initial_stroke_id(&identity, index, stroke),
                author: placeholder_author,
                color: stroke.color,
                points: stroke.points.clone(),
            })
            .collect();
        let snapshot = PageSnapshot {
            identity: identity.clone(),
            dimensions,
            background: background_path.as_ref().map(|(_, digest)| BackgroundAsset {
                digest: *digest,
                dimensions,
                encoding: BackgroundEncoding::Bgra8,
            }),
            strokes,
            active_strokes: Vec::new(),
            revision: 0,
        };
        let journal = PageJournal::from_snapshot(snapshot.clone()).map_err(io::Error::other)?;
        let snapshot_path = destination_directory.join("page-snapshot.json");
        write_json_atomically(&snapshot_path, &snapshot)?;
        let mut client_instance_nonce = [0; 16];
        getrandom::fill(&mut client_instance_nonce).map_err(io::Error::other)?;
        self.shared_page = Some(SharedPageSession {
            identity,
            journal,
            connection: None,
            pending_commands: Vec::new(),
            client_instance_nonce,
            next_identifier: 0,
            rasterizers: BTreeMap::new(),
            optimistic_pending_pixels_visible: true,
        });
        Ok((snapshot_path, background_path.map(|(path, _)| path)))
    }

    pub fn connect_page_share(
        &mut self,
        share_id: &str,
        websocket_url: &str,
        owner_token: &str,
    ) -> io::Result<()> {
        let _: remarque_page_log::ShareId = share_id
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if !websocket_url.starts_with("wss://")
            || !websocket_url.ends_with(&format!("/api/shares/{share_id}/ws"))
            || owner_token.len() < 32
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "shared page connection parameters are invalid",
            ));
        }
        let session = self
            .shared_page
            .as_mut()
            .ok_or_else(|| io::Error::other("no page share has been prepared"))?;
        session.connection = Some(SharedPageConnection::connect(
            websocket_url.to_owned(),
            owner_token.to_owned(),
        )?);
        Ok(())
    }

    pub fn apply_shared_page_messages(&mut self) -> io::Result<()> {
        let events = self
            .shared_page
            .as_ref()
            .and_then(|session| session.connection.as_ref())
            .map(SharedPageConnection::drain)
            .unwrap_or_default();
        let mut updates = Vec::new();
        for event in events {
            match event {
                SharedPageEvent::Connected => {}
                SharedPageEvent::Disconnected(reason) => {
                    eprintln!("shared_page_disconnected={reason}");
                }
                SharedPageEvent::Message(message) => {
                    updates.push(self.apply_shared_page_message(message)?);
                }
            }
        }
        let full_snapshot = updates
            .iter()
            .any(|update| matches!(update, SharedPageUpdate::FullSnapshot));
        if full_snapshot {
            if let Some(session) = &mut self.shared_page {
                session.rasterizers.clear();
            }
            self.store_shared_page_snapshot(true)?;
            return Ok(());
        }
        let mut persist = false;
        let mut incremental_failed = false;
        for update in updates {
            let SharedPageUpdate::Applied {
                operation,
                locally_drawn,
            } = update
            else {
                continue;
            };
            persist |= matches!(
                operation,
                PageOperation::CommitStroke { .. } | PageOperation::ReplaceStrokes { .. }
            );
            if !locally_drawn && !self.render_shared_page_operation(operation)? {
                incremental_failed = true;
            }
        }
        if persist || incremental_failed {
            self.store_shared_page_snapshot(incremental_failed)?;
        }
        Ok(())
    }

    fn apply_shared_page_message(
        &mut self,
        message: ServerMessage,
    ) -> io::Result<SharedPageUpdate> {
        let session = self
            .shared_page
            .as_mut()
            .ok_or_else(|| io::Error::other("shared page session disappeared"))?;
        match message {
            ServerMessage::Welcome {
                protocol_version,
                participant,
                snapshot,
            } => {
                if protocol_version != remarque_page_log::PROTOCOL_VERSION
                    || participant.role != remarque_page_log::ParticipantRole::Owner
                    || participant.color != Color::Black
                    || snapshot.identity != session.identity
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "relay returned an incompatible shared page",
                    ));
                }
                session.journal = PageJournal::from_snapshot(snapshot).map_err(io::Error::other)?;
                session.optimistic_pending_pixels_visible = session.pending_commands.is_empty();
                if let Some(connection) = &session.connection {
                    for command in &session.pending_commands {
                        connection.send(ClientMessage::Submit {
                            command: command.clone(),
                        })?;
                    }
                }
                Ok(SharedPageUpdate::FullSnapshot)
            }
            ServerMessage::Applied { operation } => {
                let was_pending = session
                    .pending_commands
                    .iter()
                    .position(|command| command.id == operation.command_id)
                    .map(|index| session.pending_commands.remove(index))
                    .is_some();
                let locally_drawn = was_pending && session.optimistic_pending_pixels_visible;
                if session.pending_commands.is_empty() {
                    session.optimistic_pending_pixels_visible = true;
                }
                if operation.revision <= session.journal.snapshot().revision {
                    return Ok(SharedPageUpdate::None);
                }
                let page_operation = operation.operation.clone();
                if let Err(error) = session.journal.apply(operation) {
                    eprintln!("shared_page_operation_rejected={error}");
                    request_shared_page_snapshot(session)?;
                    return Ok(SharedPageUpdate::None);
                }
                Ok(SharedPageUpdate::Applied {
                    operation: page_operation,
                    locally_drawn,
                })
            }
            ServerMessage::Snapshot { snapshot } => {
                if snapshot.identity != session.identity {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "relay snapshot identifies a different page",
                    ));
                }
                session.journal = PageJournal::from_snapshot(snapshot).map_err(io::Error::other)?;
                session.optimistic_pending_pixels_visible = session.pending_commands.is_empty();
                Ok(SharedPageUpdate::FullSnapshot)
            }
            ServerMessage::Digest { revision, digest } => {
                let snapshot = session.journal.snapshot();
                if revision != snapshot.revision || digest != snapshot_digest(snapshot) {
                    request_shared_page_snapshot(session)?;
                }
                Ok(SharedPageUpdate::None)
            }
            ServerMessage::Rejected { command, reason } => {
                if let Some(command) = command {
                    session
                        .pending_commands
                        .retain(|pending| pending.id != command);
                }
                eprintln!("shared_page_command_rejected={reason}");
                request_shared_page_snapshot(session)?;
                Ok(SharedPageUpdate::None)
            }
        }
    }

    fn store_shared_page_snapshot(&mut self, redraw: bool) -> io::Result<()> {
        let session = self
            .shared_page
            .as_ref()
            .ok_or_else(|| io::Error::other("shared page session disappeared"))?;
        let identity = session.identity.clone();
        let strokes = session
            .journal
            .snapshot()
            .strokes
            .iter()
            .map(shared_stroke_to_stroke)
            .collect::<Vec<_>>();
        self.library.store_page_strokes(
            &identity.document_id,
            identity.page_index as usize,
            strokes.clone(),
        )?;
        save_document_library(&self.state_path, &self.library)?;
        if self.current_page_identity().as_ref() == Some(&identity) {
            self.page_mut()?.strokes = strokes;
            if redraw {
                let changed = self.redraw_notebook()?;
                self.display.submit_mode_four_color(changed);
            }
        }
        Ok(())
    }

    fn render_shared_page_operation(&mut self, operation: PageOperation) -> io::Result<bool> {
        if !self.current_page_is_shared() {
            return Ok(true);
        }
        match operation {
            PageOperation::BeginStroke { stroke } => {
                self.shared_page.as_mut().unwrap().rasterizers.insert(
                    stroke.id,
                    SharedStrokeRasterizer {
                        rasterizer: FinelinerRasterizer::new(stroke.color),
                        previous: None,
                    },
                );
                Ok(true)
            }
            PageOperation::AppendStrokePoints {
                stroke_id, points, ..
            } => {
                let mut stroke = match self
                    .shared_page
                    .as_mut()
                    .unwrap()
                    .rasterizers
                    .remove(&stroke_id)
                {
                    Some(stroke) => stroke,
                    None => return Ok(false),
                };
                let page_rectangle = self.page()?.rectangle;
                let transform = self.transform;
                let view_size = self.view_size();
                let mut changed = None;
                for point in points {
                    let previous = stroke.previous.unwrap_or(point);
                    let screen_previous =
                        shared_point_to_view(previous, page_rectangle, transform, view_size);
                    let screen_point =
                        shared_point_to_view(point, page_rectangle, transform, view_size);
                    stroke
                        .rasterizer
                        .append_point(&mut self.image, screen_point);
                    let rectangle = fineliner_segment_rectangle(
                        screen_previous,
                        screen_point,
                        self.image.width(),
                        self.image.height(),
                    );
                    changed = Some(
                        changed.map_or(rectangle, |changed: Rectangle| changed.include(rectangle)),
                    );
                    stroke.previous = Some(point);
                }
                self.shared_page
                    .as_mut()
                    .unwrap()
                    .rasterizers
                    .insert(stroke_id, stroke);
                self.submit_shared_page_pixels(changed, false)?;
                Ok(true)
            }
            PageOperation::CommitStroke { stroke_id } => {
                let Some(mut stroke) = self
                    .shared_page
                    .as_mut()
                    .unwrap()
                    .rasterizers
                    .remove(&stroke_id)
                else {
                    return Ok(false);
                };
                stroke.rasterizer.finish(&mut self.image);
                let page_rectangle = self.page()?.rectangle;
                let transform = self.transform;
                let view_size = self.view_size();
                let changed = stroke.previous.map(|point| {
                    let point = shared_point_to_view(point, page_rectangle, transform, view_size);
                    fineliner_segment_rectangle(
                        point,
                        point,
                        self.image.width(),
                        self.image.height(),
                    )
                });
                self.submit_shared_page_pixels(changed, true)?;
                Ok(true)
            }
            PageOperation::CancelStroke { stroke_id } => {
                self.shared_page
                    .as_mut()
                    .unwrap()
                    .rasterizers
                    .remove(&stroke_id);
                Ok(false)
            }
            PageOperation::ReplaceStrokes { .. } => Ok(false),
        }
    }

    fn submit_shared_page_pixels(
        &mut self,
        changed: Option<Rectangle>,
        finished: bool,
    ) -> io::Result<()> {
        let Some(mut changed) = changed else {
            return Ok(());
        };
        if changed.y < TOOLBAR_HEIGHT {
            self.draw_toolbar_into_image()?;
            changed = changed.include(Rectangle {
                x: 0,
                y: 0,
                width: self.image.width(),
                height: TOOLBAR_HEIGHT,
            });
        }
        let changed = self.display.copy_changed_from(&self.image, changed)?;
        if finished {
            self.display.submit_mode_four_color(changed);
        } else if let Some(changed) = changed {
            self.display.submit_mode_zero_monochrome(changed);
        }
        Ok(())
    }
    fn begin_shared_stroke(&mut self) -> io::Result<Option<SharedLocalStroke>> {
        if !self.current_page_is_shared() {
            return Ok(None);
        }
        let stroke_id = {
            let session = self.shared_page.as_mut().unwrap();
            StrokeId::from_bytes(session.next_identifier(b"stroke"))
        };
        self.submit_shared_operation(SubmittedPageOperation::BeginStroke { stroke_id })?;
        Ok(Some(SharedLocalStroke {
            id: stroke_id,
            submitted_points: 0,
        }))
    }

    fn flush_shared_stroke_points(&mut self) -> io::Result<()> {
        let Some(PenContact::Fineliner {
            builder,
            shared_stroke: Some(shared),
            ..
        }) = self.active_pen_contact.as_ref()
        else {
            return Ok(());
        };
        let point_count = builder.points().len();
        if point_count == shared.submitted_points {
            return Ok(());
        }
        let stroke_id = shared.id;
        let first_point = shared.submitted_points;
        let page = self.page()?.rectangle;
        let points = builder.points()[first_point..]
            .iter()
            .copied()
            .map(|mut point| {
                point.x -= page.x as f32;
                point.y -= page.y as f32;
                point
            })
            .collect::<Vec<_>>();
        self.submit_shared_stroke_points(stroke_id, first_point, &points)?;
        if let Some(PenContact::Fineliner {
            shared_stroke: Some(shared),
            ..
        }) = self.active_pen_contact.as_mut()
            && shared.id == stroke_id
        {
            shared.submitted_points = point_count;
        }
        Ok(())
    }

    fn finish_shared_stroke(
        &mut self,
        shared: SharedLocalStroke,
        points: &[crate::stroke::StrokePoint],
    ) -> io::Result<()> {
        let remaining = points.get(shared.submitted_points..).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "submitted shared stroke point count exceeds the finished stroke",
            )
        })?;
        self.submit_shared_stroke_points(
            shared.id,
            shared.submitted_points,
            remaining,
        )?;
        self.submit_shared_operation(SubmittedPageOperation::CommitStroke {
            stroke_id: shared.id,
        })
    }

    fn cancel_shared_stroke(&mut self, shared: SharedLocalStroke) -> io::Result<()> {
        self.submit_shared_operation(SubmittedPageOperation::CancelStroke {
            stroke_id: shared.id,
        })
    }

    fn submit_shared_stroke_points(
        &mut self,
        stroke_id: StrokeId,
        first_point: usize,
        points: &[crate::stroke::StrokePoint],
    ) -> io::Result<()> {
        for (chunk_index, chunk) in points.chunks(2048).enumerate() {
            let first_point = u32::try_from(first_point + chunk_index * 2048)
                .map_err(|_| io::Error::other("stroke point count exceeds sharing limits"))?;
            self.submit_shared_operation(SubmittedPageOperation::AppendStrokePoints {
                stroke_id,
                first_point,
                points: chunk.to_vec(),
            })?;
        }
        Ok(())
    }

    fn submit_shared_eraser(&mut self, centerline: &[Point], eraser_width: f64) -> io::Result<()> {
        if !self.current_page_is_shared() {
            return Ok(());
        }
        let strokes = self
            .shared_page
            .as_ref()
            .unwrap()
            .journal
            .snapshot()
            .strokes
            .clone();
        let mut surviving_by_stroke = Vec::new();
        for stroke in strokes {
            let surviving = erase_stroke(&stroke.points, centerline, eraser_width);
            if surviving.len() != 1 || surviving[0] != stroke.points {
                surviving_by_stroke.push((stroke, surviving));
            }
        }
        if surviving_by_stroke.is_empty() {
            return Ok(());
        }
        let session = self.shared_page.as_mut().unwrap();
        let replacements = surviving_by_stroke
            .into_iter()
            .map(|(stroke, surviving)| StrokeReplacement {
                removed: stroke.id,
                fragments: surviving
                    .into_iter()
                    .map(|points| SharedStroke {
                        id: StrokeId::from_bytes(session.next_identifier(b"erased-stroke")),
                        author: stroke.author,
                        color: stroke.color,
                        points,
                    })
                    .collect(),
            })
            .collect();
        self.submit_shared_operation(SubmittedPageOperation::ReplaceStrokes { replacements })
    }

    fn submit_shared_operation(&mut self, operation: SubmittedPageOperation) -> io::Result<()> {
        let session = self
            .shared_page
            .as_mut()
            .ok_or_else(|| io::Error::other("shared page session disappeared"))?;
        let command = PageCommand {
            id: CommandId::from_bytes(session.next_identifier(b"command")),
            operation,
        };
        session.pending_commands.push(command.clone());
        if let Some(connection) = &session.connection {
            connection.send(ClientMessage::Submit { command })?;
        }
        Ok(())
    }

    fn current_page_is_shared(&self) -> bool {
        self.current_page_identity().is_some_and(|identity| {
            self.shared_page
                .as_ref()
                .is_some_and(|session| session.identity == identity)
        })
    }

    fn current_page_identity(&self) -> Option<PageIdentity> {
        let document = self.open_document.as_ref()?;
        let page_index = self
            .library
            .current_page_index(&document.document_id)
            .ok()?;
        Some(PageIdentity {
            document_id: document.document_id.clone(),
            page_index: u32::try_from(page_index).ok()?,
        })
    }
}

impl SharedPageSession {
    fn next_identifier(&mut self, domain: &[u8]) -> [u8; 16] {
        let mut input = serde_json::to_vec(&self.identity)
            .expect("shared page identities are JSON serializable");
        input.extend_from_slice(&self.client_instance_nonce);
        input.extend_from_slice(&self.next_identifier.to_le_bytes());
        input.extend_from_slice(domain);
        self.next_identifier = self.next_identifier.wrapping_add(1);
        blake3::hash(&input).as_bytes()[..16].try_into().unwrap()
    }
}

fn request_shared_page_snapshot(session: &SharedPageSession) -> io::Result<()> {
    if let Some(connection) = &session.connection {
        connection.send(ClientMessage::RequestSnapshot)?;
    }
    Ok(())
}

fn initial_stroke_id(identity: &PageIdentity, index: usize, stroke: &Stroke) -> StrokeId {
    let bytes = serde_json::to_vec(&(identity, index, stroke))
        .expect("persisted strokes are JSON serializable");
    StrokeId::from_bytes(blake3::hash(&bytes).as_bytes()[..16].try_into().unwrap())
}

fn shared_stroke_to_stroke(stroke: &SharedStroke) -> Stroke {
    Stroke {
        points: stroke.points.clone(),
        color: stroke.color,
    }
}

fn shared_point_to_view(
    mut point: crate::stroke::StrokePoint,
    page: crate::bgra_image::PixelRectangle,
    transform: ViewTransform,
    view_size: Size,
) -> crate::render_fineliner::FinelinerRasterPoint {
    point.x += page.x as f32;
    point.y += page.y as f32;
    transform_stroke_point(point, transform, view_size)
}
