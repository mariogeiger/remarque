use crate::{
    ActiveStroke, AppliedPageOperation, CommandId, PageCommand, PageOperation, PageSnapshot,
    Participant, ParticipantRole, SharedStroke, StrokeId, StrokeReplacement,
    SubmittedPageOperation,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const MAXIMUM_POINTS_PER_APPEND: usize = 2048;
const MAXIMUM_POINTS_PER_STROKE: usize = 250_000;
const MAXIMUM_POINTS_PER_PAGE: usize = 2_000_000;
const MAXIMUM_STROKES_PER_PAGE: usize = 50_000;
const MAXIMUM_PAGE_DIMENSION: u32 = 8192;
const MAXIMUM_PAGE_PIXELS: u64 = 32_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalError {
    ConflictingCommand,
    EmptyAppend,
    EmptyStroke,
    MissingStroke,
    PointIndexMismatch,
    RevisionMismatch,
    RevisionOverflow,
    StrokeAlreadyExists,
    TooManyPoints,
    Unauthorized,
    InvalidReplacement,
    InvalidPage,
    InvalidPoint,
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ConflictingCommand => "command ID was reused for a different operation",
            Self::EmptyAppend => "stroke append contains no points",
            Self::EmptyStroke => "stroke cannot be committed without points",
            Self::MissingStroke => "stroke does not exist in the required state",
            Self::PointIndexMismatch => {
                "stroke append does not continue at the current point count"
            }
            Self::RevisionMismatch => "operation does not follow the current page revision",
            Self::RevisionOverflow => "page revision range is exhausted",
            Self::StrokeAlreadyExists => "stroke ID is already in use",
            Self::TooManyPoints => "stroke append exceeds the point limit",
            Self::Unauthorized => "participant cannot apply this operation",
            Self::InvalidReplacement => "stroke replacement is structurally invalid",
            Self::InvalidPage => "page snapshot is structurally invalid",
            Self::InvalidPoint => "stroke contains a non-finite point",
        })
    }
}

impl std::error::Error for JournalError {}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PageJournal {
    snapshot: PageSnapshot,
    commands: BTreeMap<CommandId, ([u8; 32], AppliedPageOperation)>,
    #[serde(default)]
    point_count: usize,
    #[serde(default)]
    stroke_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Submission {
    pub operation: AppliedPageOperation,
    pub newly_applied: bool,
}

impl PageJournal {
    pub fn from_snapshot(snapshot: PageSnapshot) -> Result<Self, JournalError> {
        let (stroke_count, point_count) = validate_snapshot(&snapshot)?;
        Ok(Self {
            snapshot,
            commands: BTreeMap::new(),
            point_count,
            stroke_count,
        })
    }

    pub const fn snapshot(&self) -> &PageSnapshot {
        &self.snapshot
    }

    pub fn revalidate(&mut self) -> Result<(), JournalError> {
        let (stroke_count, point_count) = validate_snapshot(&self.snapshot)?;
        if self.commands.values().any(|(_, operation)| {
            operation.revision == 0 || operation.revision > self.snapshot.revision
        }) {
            return Err(JournalError::InvalidPage);
        }
        self.stroke_count = stroke_count;
        self.point_count = point_count;
        Ok(())
    }

    pub fn submit(
        &mut self,
        participant: Participant,
        command: PageCommand,
    ) -> Result<Submission, JournalError> {
        let command_digest = command_digest(&command)?;
        if let Some((existing_digest, operation)) = self.commands.get(&command.id) {
            if existing_digest != &command_digest {
                return Err(JournalError::ConflictingCommand);
            }
            return Ok(Submission {
                operation: operation.clone(),
                newly_applied: false,
            });
        }
        let operation = materialize_operation(
            &self.snapshot,
            self.stroke_count,
            self.point_count,
            participant,
            &command.operation,
        )?;
        let revision = self
            .snapshot
            .revision
            .checked_add(1)
            .ok_or(JournalError::RevisionOverflow)?;
        let applied = AppliedPageOperation {
            revision,
            command_id: command.id,
            actor: participant.id,
            operation,
        };
        apply_operation(
            &mut self.snapshot,
            &mut self.stroke_count,
            &mut self.point_count,
            &applied.operation,
        )?;
        self.snapshot.revision = revision;
        self.commands
            .insert(command.id, (command_digest, applied.clone()));
        Ok(Submission {
            operation: applied,
            newly_applied: true,
        })
    }

    pub fn apply(&mut self, operation: AppliedPageOperation) -> Result<(), JournalError> {
        if operation.revision != self.snapshot.revision + 1 {
            return Err(JournalError::RevisionMismatch);
        }
        apply_operation(
            &mut self.snapshot,
            &mut self.stroke_count,
            &mut self.point_count,
            &operation.operation,
        )?;
        self.snapshot.revision = operation.revision;
        Ok(())
    }
}

fn command_digest(command: &PageCommand) -> Result<[u8; 32], JournalError> {
    let bytes = postcard::to_allocvec(command).map_err(|_| JournalError::InvalidPage)?;
    Ok(*blake3::hash(&bytes).as_bytes())
}

fn validate_snapshot(snapshot: &PageSnapshot) -> Result<(usize, usize), JournalError> {
    if snapshot.dimensions.width == 0
        || snapshot.dimensions.height == 0
        || snapshot.dimensions.width > MAXIMUM_PAGE_DIMENSION
        || snapshot.dimensions.height > MAXIMUM_PAGE_DIMENSION
        || u64::from(snapshot.dimensions.width) * u64::from(snapshot.dimensions.height)
            > MAXIMUM_PAGE_PIXELS
        || snapshot
            .background
            .as_ref()
            .is_some_and(|background| background.dimensions != snapshot.dimensions)
    {
        return Err(JournalError::InvalidPage);
    }
    let mut ids = BTreeSet::new();
    let mut point_count = 0usize;
    for stroke in &snapshot.strokes {
        if stroke.points.len() > MAXIMUM_POINTS_PER_STROKE {
            return Err(JournalError::TooManyPoints);
        }
        if !ids.insert(stroke.id) {
            return Err(JournalError::StrokeAlreadyExists);
        }
        if !stroke.points.iter().all(valid_point) {
            return Err(JournalError::InvalidPoint);
        }
        point_count = point_count
            .checked_add(stroke.points.len())
            .ok_or(JournalError::TooManyPoints)?;
    }
    for active in &snapshot.active_strokes {
        if active.stroke.points.len() > MAXIMUM_POINTS_PER_STROKE {
            return Err(JournalError::TooManyPoints);
        }
        if !ids.insert(active.stroke.id) {
            return Err(JournalError::StrokeAlreadyExists);
        }
        if !active.stroke.points.iter().all(valid_point) {
            return Err(JournalError::InvalidPoint);
        }
        point_count = point_count
            .checked_add(active.stroke.points.len())
            .ok_or(JournalError::TooManyPoints)?;
    }
    if ids.len() > MAXIMUM_STROKES_PER_PAGE || point_count > MAXIMUM_POINTS_PER_PAGE {
        return Err(JournalError::TooManyPoints);
    }
    Ok((ids.len(), point_count))
}

fn materialize_operation(
    snapshot: &PageSnapshot,
    stroke_count: usize,
    point_count: usize,
    participant: Participant,
    submitted: &SubmittedPageOperation,
) -> Result<PageOperation, JournalError> {
    match submitted {
        SubmittedPageOperation::BeginStroke { stroke_id } => {
            if stroke_count >= MAXIMUM_STROKES_PER_PAGE {
                return Err(JournalError::TooManyPoints);
            }
            ensure_stroke_id_is_unused(snapshot, *stroke_id)?;
            Ok(PageOperation::BeginStroke {
                stroke: SharedStroke {
                    id: *stroke_id,
                    author: participant.id,
                    color: participant.color,
                    points: Vec::new(),
                },
            })
        }
        SubmittedPageOperation::AppendStrokePoints {
            stroke_id,
            first_point,
            points,
        } => {
            let stroke = active_stroke(snapshot, *stroke_id)?;
            if stroke.author != participant.id {
                return Err(JournalError::Unauthorized);
            }
            validate_append(stroke, point_count, *first_point, points)?;
            Ok(PageOperation::AppendStrokePoints {
                stroke_id: *stroke_id,
                first_point: *first_point,
                points: points.clone(),
            })
        }
        SubmittedPageOperation::CommitStroke { stroke_id } => {
            let stroke = active_stroke(snapshot, *stroke_id)?;
            if stroke.author != participant.id {
                return Err(JournalError::Unauthorized);
            }
            if stroke.points.is_empty() {
                return Err(JournalError::EmptyStroke);
            }
            Ok(PageOperation::CommitStroke {
                stroke_id: *stroke_id,
            })
        }
        SubmittedPageOperation::CancelStroke { stroke_id } => {
            let stroke = active_stroke(snapshot, *stroke_id)?;
            if stroke.author != participant.id {
                return Err(JournalError::Unauthorized);
            }
            Ok(PageOperation::CancelStroke {
                stroke_id: *stroke_id,
            })
        }
        SubmittedPageOperation::ReplaceStrokes { replacements } => {
            validate_replacements(
                snapshot,
                stroke_count,
                point_count,
                participant,
                replacements,
            )?;
            Ok(PageOperation::ReplaceStrokes {
                replacements: replacements.clone(),
            })
        }
    }
}

fn apply_operation(
    snapshot: &mut PageSnapshot,
    stroke_count: &mut usize,
    point_count: &mut usize,
    operation: &PageOperation,
) -> Result<(), JournalError> {
    match operation {
        PageOperation::BeginStroke { stroke } => {
            if *stroke_count >= MAXIMUM_STROKES_PER_PAGE {
                return Err(JournalError::TooManyPoints);
            }
            ensure_stroke_id_is_unused(snapshot, stroke.id)?;
            snapshot.active_strokes.push(ActiveStroke {
                stroke: stroke.clone(),
            });
            *stroke_count += 1;
        }
        PageOperation::AppendStrokePoints {
            stroke_id,
            first_point,
            points,
        } => {
            let stroke = active_stroke_mut(snapshot, *stroke_id)?;
            validate_append(stroke, *point_count, *first_point, points)?;
            stroke.points.extend_from_slice(points);
            *point_count += points.len();
        }
        PageOperation::CommitStroke { stroke_id } => {
            let index = snapshot
                .active_strokes
                .iter()
                .position(|active| active.stroke.id == *stroke_id)
                .ok_or(JournalError::MissingStroke)?;
            let active = snapshot.active_strokes.remove(index);
            snapshot.strokes.push(active.stroke);
        }
        PageOperation::CancelStroke { stroke_id } => {
            let index = snapshot
                .active_strokes
                .iter()
                .position(|active| active.stroke.id == *stroke_id)
                .ok_or(JournalError::MissingStroke)?;
            let active = snapshot.active_strokes.remove(index);
            *point_count -= active.stroke.points.len();
            *stroke_count -= 1;
        }
        PageOperation::ReplaceStrokes { replacements } => {
            replace_strokes(snapshot, stroke_count, point_count, replacements)?;
        }
    }
    Ok(())
}

fn validate_append(
    stroke: &SharedStroke,
    page_point_count: usize,
    first_point: u32,
    points: &[remarque_core::stroke::StrokePoint],
) -> Result<(), JournalError> {
    if points.is_empty() {
        return Err(JournalError::EmptyAppend);
    }
    if points.len() > MAXIMUM_POINTS_PER_APPEND
        || stroke.points.len().checked_add(points.len()).is_none()
        || stroke.points.len() + points.len() > MAXIMUM_POINTS_PER_STROKE
        || page_point_count.checked_add(points.len()).is_none()
        || page_point_count + points.len() > MAXIMUM_POINTS_PER_PAGE
    {
        return Err(JournalError::TooManyPoints);
    }
    if !points.iter().all(valid_point) {
        return Err(JournalError::InvalidPoint);
    }
    if usize::try_from(first_point).ok() != Some(stroke.points.len()) {
        return Err(JournalError::PointIndexMismatch);
    }
    Ok(())
}

fn valid_point(point: &remarque_core::stroke::StrokePoint) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

fn validate_replacements(
    snapshot: &PageSnapshot,
    stroke_count: usize,
    point_count: usize,
    participant: Participant,
    replacements: &[StrokeReplacement],
) -> Result<(), JournalError> {
    if replacements.is_empty() {
        return Err(JournalError::InvalidReplacement);
    }
    let mut removed = BTreeSet::new();
    let mut new_ids = BTreeSet::new();
    let mut removed_points = 0usize;
    let mut replacement_points = 0usize;
    let mut replacement_strokes = 0usize;
    for replacement in replacements {
        if !removed.insert(replacement.removed) {
            return Err(JournalError::InvalidReplacement);
        }
        let original = committed_stroke(snapshot, replacement.removed)?;
        removed_points = removed_points
            .checked_add(original.points.len())
            .ok_or(JournalError::TooManyPoints)?;
        if participant.role != ParticipantRole::Owner && original.author != participant.id {
            return Err(JournalError::Unauthorized);
        }
        for fragment in &replacement.fragments {
            if fragment.points.is_empty()
                || fragment.author != original.author
                || fragment.color != original.color
                || !new_ids.insert(fragment.id)
            {
                return Err(JournalError::InvalidReplacement);
            }
            if !fragment.points.iter().all(valid_point) {
                return Err(JournalError::InvalidPoint);
            }
            if fragment.points.len() > MAXIMUM_POINTS_PER_STROKE {
                return Err(JournalError::TooManyPoints);
            }
            replacement_points = replacement_points
                .checked_add(fragment.points.len())
                .ok_or(JournalError::TooManyPoints)?;
            replacement_strokes = replacement_strokes
                .checked_add(1)
                .ok_or(JournalError::TooManyPoints)?;
            ensure_stroke_id_is_unused(snapshot, fragment.id)?;
        }
    }
    let resulting_points = point_count
        .checked_sub(removed_points)
        .and_then(|points| points.checked_add(replacement_points))
        .ok_or(JournalError::InvalidPage)?;
    let resulting_strokes = stroke_count
        .checked_sub(removed.len())
        .and_then(|strokes| strokes.checked_add(replacement_strokes))
        .ok_or(JournalError::InvalidPage)?;
    if resulting_points > MAXIMUM_POINTS_PER_PAGE || resulting_strokes > MAXIMUM_STROKES_PER_PAGE {
        return Err(JournalError::TooManyPoints);
    }
    Ok(())
}

fn replace_strokes(
    snapshot: &mut PageSnapshot,
    stroke_count: &mut usize,
    point_count: &mut usize,
    replacements: &[StrokeReplacement],
) -> Result<(), JournalError> {
    let replacements = replacements
        .iter()
        .map(|replacement| (replacement.removed, &replacement.fragments))
        .collect::<BTreeMap<_, _>>();
    if replacements
        .keys()
        .any(|stroke_id| committed_stroke(snapshot, *stroke_id).is_err())
    {
        return Err(JournalError::MissingStroke);
    }
    let removed_points = replacements
        .keys()
        .map(|stroke_id| committed_stroke(snapshot, *stroke_id).unwrap().points.len())
        .sum::<usize>();
    let replacement_points = replacements
        .values()
        .flat_map(|fragments| fragments.iter())
        .map(|stroke| stroke.points.len())
        .sum::<usize>();
    let replacement_strokes = replacements
        .values()
        .map(|fragments| fragments.len())
        .sum::<usize>();
    let mut strokes = Vec::with_capacity(snapshot.strokes.len());
    for stroke in snapshot.strokes.drain(..) {
        if let Some(fragments) = replacements.get(&stroke.id) {
            strokes.extend(fragments.iter().cloned());
        } else {
            strokes.push(stroke);
        }
    }
    snapshot.strokes = strokes;
    *point_count = *point_count - removed_points + replacement_points;
    *stroke_count = *stroke_count - replacements.len() + replacement_strokes;
    Ok(())
}

fn ensure_stroke_id_is_unused(
    snapshot: &PageSnapshot,
    stroke_id: StrokeId,
) -> Result<(), JournalError> {
    if snapshot.strokes.iter().any(|stroke| stroke.id == stroke_id)
        || snapshot
            .active_strokes
            .iter()
            .any(|active| active.stroke.id == stroke_id)
    {
        Err(JournalError::StrokeAlreadyExists)
    } else {
        Ok(())
    }
}

fn committed_stroke(
    snapshot: &PageSnapshot,
    stroke_id: StrokeId,
) -> Result<&SharedStroke, JournalError> {
    snapshot
        .strokes
        .iter()
        .find(|stroke| stroke.id == stroke_id)
        .ok_or(JournalError::MissingStroke)
}

fn active_stroke(
    snapshot: &PageSnapshot,
    stroke_id: StrokeId,
) -> Result<&SharedStroke, JournalError> {
    snapshot
        .active_strokes
        .iter()
        .find(|active| active.stroke.id == stroke_id)
        .map(|active| &active.stroke)
        .ok_or(JournalError::MissingStroke)
}

fn active_stroke_mut(
    snapshot: &mut PageSnapshot,
    stroke_id: StrokeId,
) -> Result<&mut SharedStroke, JournalError> {
    snapshot
        .active_strokes
        .iter_mut()
        .find(|active| active.stroke.id == stroke_id)
        .map(|active| &mut active.stroke)
        .ok_or(JournalError::MissingStroke)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PageDimensions, PageIdentity, ParticipantId};
    use remarque_core::color::Color;
    use remarque_core::stroke::StrokePoint;

    fn id(byte: u8) -> [u8; 16] {
        [byte; 16]
    }

    fn participant(role: ParticipantRole) -> Participant {
        Participant {
            id: ParticipantId::from_bytes(id(1)),
            role,
            color: Color::Blue,
        }
    }

    fn point(x: f32) -> StrokePoint {
        StrokePoint {
            x,
            y: 2.0,
            two_segment_distance_quarters: 0,
            width_quarter_pixels: 8,
            direction: 0,
            pressure: 0,
        }
    }

    fn journal() -> PageJournal {
        PageJournal::from_snapshot(PageSnapshot {
            identity: PageIdentity {
                document_id: "notebook-1".to_owned(),
                page_index: 0,
            },
            dimensions: PageDimensions {
                width: 100,
                height: 200,
            },
            background: None,
            strokes: Vec::new(),
            active_strokes: Vec::new(),
            revision: 0,
        })
        .unwrap()
    }

    #[test]
    fn stroke_lifecycle_is_ordered_and_idempotent() {
        let mut journal = journal();
        let stroke_id = StrokeId::from_bytes(id(2));
        let begin = PageCommand {
            id: CommandId::from_bytes(id(3)),
            operation: SubmittedPageOperation::BeginStroke { stroke_id },
        };
        let first = journal
            .submit(participant(ParticipantRole::Editor), begin.clone())
            .unwrap();
        assert_eq!(first.operation.revision, 1);
        assert!(first.newly_applied);
        assert_eq!(
            journal
                .submit(participant(ParticipantRole::Editor), begin)
                .unwrap()
                .operation,
            first.operation
        );
        journal
            .submit(
                participant(ParticipantRole::Editor),
                PageCommand {
                    id: CommandId::from_bytes(id(4)),
                    operation: SubmittedPageOperation::AppendStrokePoints {
                        stroke_id,
                        first_point: 0,
                        points: vec![point(1.0), point(2.0)],
                    },
                },
            )
            .unwrap();
        journal
            .submit(
                participant(ParticipantRole::Editor),
                PageCommand {
                    id: CommandId::from_bytes(id(5)),
                    operation: SubmittedPageOperation::CommitStroke { stroke_id },
                },
            )
            .unwrap();
        assert_eq!(journal.snapshot().revision, 3);
        assert_eq!(journal.snapshot().strokes[0].points.len(), 2);
        assert!(journal.snapshot().active_strokes.is_empty());
    }

    #[test]
    fn cancelled_stroke_releases_its_identifier_and_points() {
        let mut journal = journal();
        let stroke_id = StrokeId::from_bytes(id(20));
        for (command_id, operation) in [
            (21, SubmittedPageOperation::BeginStroke { stroke_id }),
            (
                22,
                SubmittedPageOperation::AppendStrokePoints {
                    stroke_id,
                    first_point: 0,
                    points: vec![point(1.0)],
                },
            ),
            (23, SubmittedPageOperation::CancelStroke { stroke_id }),
        ] {
            journal
                .submit(
                    participant(ParticipantRole::Editor),
                    PageCommand {
                        id: CommandId::from_bytes(id(command_id)),
                        operation,
                    },
                )
                .unwrap();
        }
        assert!(journal.snapshot().strokes.is_empty());
        assert!(journal.snapshot().active_strokes.is_empty());
        journal
            .submit(
                participant(ParticipantRole::Editor),
                PageCommand {
                    id: CommandId::from_bytes(id(24)),
                    operation: SubmittedPageOperation::BeginStroke { stroke_id },
                },
            )
            .unwrap();
    }

    #[test]
    fn editor_cannot_replace_another_participants_stroke() {
        let mut journal = journal();
        journal.snapshot.strokes.push(SharedStroke {
            id: StrokeId::from_bytes(id(8)),
            author: ParticipantId::from_bytes(id(9)),
            color: Color::Red,
            points: vec![point(1.0)],
        });
        journal.revalidate().unwrap();
        let error = journal
            .submit(
                participant(ParticipantRole::Editor),
                PageCommand {
                    id: CommandId::from_bytes(id(10)),
                    operation: SubmittedPageOperation::ReplaceStrokes {
                        replacements: vec![StrokeReplacement {
                            removed: StrokeId::from_bytes(id(8)),
                            fragments: Vec::new(),
                        }],
                    },
                },
            )
            .unwrap_err();
        assert_eq!(error, JournalError::Unauthorized);
    }

    #[test]
    fn owner_can_erase_any_participants_stroke() {
        let mut journal = journal();
        journal.snapshot.strokes.push(SharedStroke {
            id: StrokeId::from_bytes(id(8)),
            author: ParticipantId::from_bytes(id(9)),
            color: Color::Red,
            points: vec![point(1.0)],
        });
        journal.revalidate().unwrap();
        journal
            .submit(
                participant(ParticipantRole::Owner),
                PageCommand {
                    id: CommandId::from_bytes(id(10)),
                    operation: SubmittedPageOperation::ReplaceStrokes {
                        replacements: vec![StrokeReplacement {
                            removed: StrokeId::from_bytes(id(8)),
                            fragments: Vec::new(),
                        }],
                    },
                },
            )
            .unwrap();
        assert!(journal.snapshot().strokes.is_empty());
    }
}
