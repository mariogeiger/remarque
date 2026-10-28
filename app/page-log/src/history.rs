use crate::{
    CommandId, HistoryAvailability, JournalError, Participant, ParticipantId, ParticipantRole,
    SharedStroke, StrokeId,
};
use remarque_core::erase_strokes::erase_stroke;
use remarque_core::view_transform::Point;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAXIMUM_HISTORY_ACTIONS: usize = 50_000;
const MAXIMUM_ERASER_POINTS_PER_ACTION: usize = 2_048;
const MAXIMUM_RECORDED_ERASER_POINTS: usize = 250_000;
const MAXIMUM_RECORDED_STROKE_POINTS: usize = 2_000_000;
const MAXIMUM_RECORDED_ERASER_TARGETS: usize = 250_000;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PageHistory {
    baseline: Vec<SharedStroke>,
    actions: Vec<RecordedPageAction>,
    participants: BTreeMap<ParticipantId, ParticipantActionStacks>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RecordedPageAction {
    id: CommandId,
    actor: ParticipantId,
    active: bool,
    edit: RecordedPageEdit,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum RecordedPageEdit {
    Draw(SharedStroke),
    Erase {
        centerline: Vec<Point>,
        width: f64,
        targets: Vec<StrokeId>,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ParticipantActionStacks {
    undo: Vec<CommandId>,
    redo: Vec<CommandId>,
}

impl PageHistory {
    pub(crate) fn from_baseline(baseline: Vec<SharedStroke>) -> Self {
        Self {
            baseline,
            actions: Vec::new(),
            participants: BTreeMap::new(),
        }
    }

    pub(crate) fn initialize_legacy_baseline(&mut self, strokes: &[SharedStroke]) {
        if self.baseline.is_empty() && self.actions.is_empty() && self.participants.is_empty() {
            self.baseline = strokes.to_vec();
        }
    }

    pub(crate) fn availability(&self, participant: ParticipantId) -> HistoryAvailability {
        self.participants
            .get(&participant)
            .map_or_else(HistoryAvailability::default, |stacks| HistoryAvailability {
                can_undo: !stacks.undo.is_empty(),
                can_redo: !stacks.redo.is_empty(),
            })
    }

    pub(crate) fn record_draw(
        &mut self,
        id: CommandId,
        actor: ParticipantId,
        stroke: SharedStroke,
    ) -> Result<(), JournalError> {
        self.ensure_action_capacity()?;
        let recorded_points = self
            .root_strokes()
            .try_fold(stroke.points.len(), |points, (recorded, _)| {
                points.checked_add(recorded.points.len())
            })
            .ok_or(JournalError::TooManyActions)?;
        if recorded_points > MAXIMUM_RECORDED_STROKE_POINTS {
            return Err(JournalError::TooManyActions);
        }
        self.discard_redo(actor);
        self.actions.push(RecordedPageAction {
            id,
            actor,
            active: true,
            edit: RecordedPageEdit::Draw(stroke),
        });
        self.participants.entry(actor).or_default().undo.push(id);
        Ok(())
    }

    pub(crate) fn record_erase(
        &mut self,
        id: CommandId,
        participant: Participant,
        centerline: Vec<Point>,
        width: f64,
    ) -> Result<bool, JournalError> {
        self.validate_new_eraser(&centerline, width)?;
        self.ensure_action_capacity()?;
        let targets = self.eraser_targets(participant, &centerline, width);
        if targets.is_empty() {
            return Ok(false);
        }
        let recorded_targets = self
            .actions
            .iter()
            .filter_map(|action| match &action.edit {
                RecordedPageEdit::Erase { targets, .. } => Some(targets.len()),
                RecordedPageEdit::Draw(_) => None,
            })
            .try_fold(targets.len(), usize::checked_add)
            .ok_or(JournalError::TooManyActions)?;
        if recorded_targets > MAXIMUM_RECORDED_ERASER_TARGETS {
            return Err(JournalError::TooManyActions);
        }
        self.discard_redo(participant.id);
        self.actions.push(RecordedPageAction {
            id,
            actor: participant.id,
            active: true,
            edit: RecordedPageEdit::Erase {
                centerline,
                width,
                targets,
            },
        });
        self.participants
            .entry(participant.id)
            .or_default()
            .undo
            .push(id);
        Ok(true)
    }

    pub(crate) fn undo(&mut self, participant: ParticipantId) -> Result<(), JournalError> {
        let id = self
            .participants
            .get_mut(&participant)
            .and_then(|stacks| stacks.undo.pop())
            .ok_or(JournalError::NothingToUndo)?;
        let action = self.action_mut(id).ok_or(JournalError::InvalidPage)?;
        if action.actor != participant || !action.active {
            return Err(JournalError::InvalidPage);
        }
        action.active = false;
        self.participants
            .get_mut(&participant)
            .expect("an undo stack was just popped")
            .redo
            .push(id);
        Ok(())
    }

    pub(crate) fn redo(&mut self, participant: ParticipantId) -> Result<(), JournalError> {
        let id = self
            .participants
            .get_mut(&participant)
            .and_then(|stacks| stacks.redo.pop())
            .ok_or(JournalError::NothingToRedo)?;
        let action = self.action_mut(id).ok_or(JournalError::InvalidPage)?;
        if action.actor != participant || action.active {
            return Err(JournalError::InvalidPage);
        }
        action.active = true;
        self.participants
            .get_mut(&participant)
            .expect("a redo stack was just popped")
            .undo
            .push(id);
        Ok(())
    }

    pub(crate) fn rendered_strokes(&self) -> Vec<SharedStroke> {
        self.root_strokes()
            .filter(|(_, active)| *active)
            .flat_map(|(stroke, _)| self.render_stroke(stroke))
            .collect()
    }

    pub(crate) fn validate(&self, visible_strokes: &[SharedStroke]) -> Result<(), JournalError> {
        if self.actions.len() > MAXIMUM_HISTORY_ACTIONS {
            return Err(JournalError::TooManyActions);
        }
        let mut root_ids = BTreeSet::new();
        for stroke in &self.baseline {
            if stroke.points.is_empty()
                || stroke
                    .points
                    .iter()
                    .any(|point| !point.x.is_finite() || !point.y.is_finite())
                || !root_ids.insert(stroke.id)
            {
                return Err(JournalError::InvalidPage);
            }
        }
        let mut action_ids = BTreeSet::new();
        let mut recorded_eraser_points = 0usize;
        let mut recorded_stroke_points = 0usize;
        let mut recorded_eraser_targets = 0usize;
        for action in &self.actions {
            if !action_ids.insert(action.id) {
                return Err(JournalError::InvalidPage);
            }
            match &action.edit {
                RecordedPageEdit::Draw(stroke) => {
                    if stroke.author != action.actor
                        || stroke.points.is_empty()
                        || stroke
                            .points
                            .iter()
                            .any(|point| !point.x.is_finite() || !point.y.is_finite())
                        || !root_ids.insert(stroke.id)
                    {
                        return Err(JournalError::InvalidPage);
                    }
                    recorded_stroke_points = recorded_stroke_points
                        .checked_add(stroke.points.len())
                        .ok_or(JournalError::TooManyActions)?;
                }
                RecordedPageEdit::Erase {
                    centerline,
                    width,
                    targets,
                } => {
                    validate_eraser(centerline, *width)?;
                    if targets.is_empty()
                        || targets.iter().copied().collect::<BTreeSet<_>>().len() != targets.len()
                    {
                        return Err(JournalError::InvalidPage);
                    }
                    recorded_eraser_points = recorded_eraser_points
                        .checked_add(centerline.len())
                        .ok_or(JournalError::TooManyActions)?;
                    recorded_eraser_targets = recorded_eraser_targets
                        .checked_add(targets.len())
                        .ok_or(JournalError::TooManyActions)?;
                }
            }
        }
        recorded_stroke_points = self
            .baseline
            .iter()
            .try_fold(recorded_stroke_points, |points, stroke| {
                points.checked_add(stroke.points.len())
            })
            .ok_or(JournalError::TooManyActions)?;
        if recorded_eraser_points > MAXIMUM_RECORDED_ERASER_POINTS {
            return Err(JournalError::TooManyActions);
        }
        if recorded_stroke_points > MAXIMUM_RECORDED_STROKE_POINTS
            || recorded_eraser_targets > MAXIMUM_RECORDED_ERASER_TARGETS
        {
            return Err(JournalError::TooManyActions);
        }
        for action in &self.actions {
            if let RecordedPageEdit::Erase { targets, .. } = &action.edit
                && targets.iter().any(|target| !root_ids.contains(target))
            {
                return Err(JournalError::InvalidPage);
            }
        }
        let mut stacked_actions = BTreeSet::new();
        for (participant, stacks) in &self.participants {
            for (id, expected_active) in stacks
                .undo
                .iter()
                .map(|id| (id, true))
                .chain(stacks.redo.iter().map(|id| (id, false)))
            {
                if !stacked_actions.insert(*id) {
                    return Err(JournalError::InvalidPage);
                }
                let action = self.action(*id).ok_or(JournalError::InvalidPage)?;
                if action.actor != *participant || action.active != expected_active {
                    return Err(JournalError::InvalidPage);
                }
            }
        }
        for action in &self.actions {
            if action.active && !stacked_actions.contains(&action.id) {
                return Err(JournalError::InvalidPage);
            }
        }
        if self.rendered_strokes() != visible_strokes {
            return Err(JournalError::InvalidPage);
        }
        Ok(())
    }

    fn ensure_action_capacity(&self) -> Result<(), JournalError> {
        if self.actions.len() >= MAXIMUM_HISTORY_ACTIONS {
            Err(JournalError::TooManyActions)
        } else {
            Ok(())
        }
    }

    fn validate_new_eraser(&self, centerline: &[Point], width: f64) -> Result<(), JournalError> {
        validate_eraser(centerline, width)?;
        let points = self
            .actions
            .iter()
            .filter_map(|action| match &action.edit {
                RecordedPageEdit::Erase { centerline, .. } => Some(centerline.len()),
                RecordedPageEdit::Draw(_) => None,
            })
            .try_fold(centerline.len(), usize::checked_add)
            .ok_or(JournalError::TooManyActions)?;
        if points > MAXIMUM_RECORDED_ERASER_POINTS {
            Err(JournalError::TooManyActions)
        } else {
            Ok(())
        }
    }

    fn discard_redo(&mut self, participant: ParticipantId) {
        if let Some(stacks) = self.participants.get_mut(&participant) {
            stacks.redo.clear();
        }
    }

    fn eraser_targets(
        &self,
        participant: Participant,
        centerline: &[Point],
        width: f64,
    ) -> Vec<StrokeId> {
        self.root_strokes()
            .filter(|(stroke, active)| {
                *active
                    && (participant.role == ParticipantRole::Owner
                        || stroke.author == participant.id)
            })
            .filter_map(|(stroke, _)| {
                self.render_stroke(stroke)
                    .iter()
                    .any(|fragment| {
                        let surviving = erase_stroke(&fragment.points, centerline, width);
                        surviving.len() != 1 || surviving[0] != fragment.points
                    })
                    .then_some(stroke.id)
            })
            .collect()
    }

    fn root_strokes(&self) -> impl Iterator<Item = (&SharedStroke, bool)> {
        self.baseline
            .iter()
            .map(|stroke| (stroke, true))
            .chain(self.actions.iter().filter_map(|action| match &action.edit {
                RecordedPageEdit::Draw(stroke) => Some((stroke, action.active)),
                RecordedPageEdit::Erase { .. } => None,
            }))
    }

    fn render_stroke(&self, root: &SharedStroke) -> Vec<SharedStroke> {
        let mut fragments = vec![root.points.clone()];
        let mut eraser_ids = Vec::new();
        for action in &self.actions {
            let RecordedPageEdit::Erase {
                centerline,
                width,
                targets,
            } = &action.edit
            else {
                continue;
            };
            if !action.active || !targets.contains(&root.id) {
                continue;
            }
            fragments = fragments
                .into_iter()
                .flat_map(|fragment| erase_stroke(&fragment, centerline, *width))
                .collect();
            eraser_ids.push(action.id);
        }
        if eraser_ids.is_empty() {
            return vec![root.clone()];
        }
        fragments
            .into_iter()
            .enumerate()
            .map(|(index, points)| SharedStroke {
                id: rendered_fragment_id(root.id, &eraser_ids, index),
                author: root.author,
                color: root.color,
                points,
            })
            .collect()
    }

    fn action(&self, id: CommandId) -> Option<&RecordedPageAction> {
        self.actions.iter().find(|action| action.id == id)
    }

    fn action_mut(&mut self, id: CommandId) -> Option<&mut RecordedPageAction> {
        self.actions.iter_mut().find(|action| action.id == id)
    }
}

fn validate_eraser(centerline: &[Point], width: f64) -> Result<(), JournalError> {
    if centerline.is_empty()
        || centerline.len() > MAXIMUM_ERASER_POINTS_PER_ACTION
        || !width.is_finite()
        || width <= 0.0
        || centerline
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        Err(JournalError::InvalidEraser)
    } else {
        Ok(())
    }
}

fn rendered_fragment_id(root: StrokeId, erasers: &[CommandId], fragment_index: usize) -> StrokeId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"remarque-rendered-fragment-v1");
    hasher.update(&root.bytes());
    for eraser in erasers {
        hasher.update(&eraser.bytes());
    }
    hasher.update(&fragment_index.to_le_bytes());
    StrokeId::from_bytes(hasher.finalize().as_bytes()[..16].try_into().unwrap())
}
