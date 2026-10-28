use crate::stroke::Stroke;
use remarque_document::write_json_atomically;
use remarque_page_log::{HistoryAvailability, PageIdentity};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FORMAT_VERSION: u32 = 1;
const MAXIMUM_ACTIONS_PER_PAGE: usize = 1_000;
const MAXIMUM_PAGES_WITH_HISTORY: usize = 10_000;

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PageEditHistories {
    format_version: u32,
    pages: Vec<PageEditHistory>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PageEditHistory {
    identity: PageIdentity,
    undo: Vec<StrokeSequenceReplacement>,
    redo: Vec<StrokeSequenceReplacement>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StrokeSequenceReplacement {
    start: usize,
    removed: Vec<Stroke>,
    inserted: Vec<Stroke>,
}

impl Default for PageEditHistories {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            pages: Vec::new(),
        }
    }
}

impl PageEditHistories {
    pub(crate) fn restore(state_path: &Path) -> io::Result<Self> {
        let path = histories_path(state_path);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let histories: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        histories.validate()?;
        Ok(histories)
    }

    pub(crate) fn save(&self, state_path: &Path) -> io::Result<()> {
        self.validate()?;
        write_json_atomically(&histories_path(state_path), self)
    }

    pub(crate) fn availability(&self, identity: &PageIdentity) -> HistoryAvailability {
        self.history(identity)
            .map_or_else(HistoryAvailability::default, |history| {
                HistoryAvailability {
                    can_undo: !history.undo.is_empty(),
                    can_redo: !history.redo.is_empty(),
                }
            })
    }

    pub(crate) fn record(&mut self, identity: PageIdentity, before: &[Stroke], after: &[Stroke]) {
        let Some(replacement) = StrokeSequenceReplacement::between(before, after) else {
            return;
        };
        let history = self.history_mut_or_insert(identity);
        history.redo.clear();
        history.undo.push(replacement);
        if history.undo.len() > MAXIMUM_ACTIONS_PER_PAGE {
            history.undo.remove(0);
        }
    }

    pub(crate) fn insert_page(&mut self, document_id: &str, page_index: u32) -> io::Result<()> {
        for history in self.pages.iter_mut().filter(|history| {
            history.identity.document_id == document_id && history.identity.page_index >= page_index
        }) {
            history.identity.page_index =
                history.identity.page_index.checked_add(1).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "page history index overflowed")
                })?;
        }
        Ok(())
    }

    pub(crate) fn undo(
        &mut self,
        identity: &PageIdentity,
        strokes: &mut Vec<Stroke>,
    ) -> io::Result<bool> {
        let Some(history) = self.history_mut(identity) else {
            return Ok(false);
        };
        let Some(action) = history.undo.last().cloned() else {
            return Ok(false);
        };
        action.apply_reverse(strokes)?;
        history.undo.pop();
        history.redo.push(action);
        Ok(true)
    }

    pub(crate) fn redo(
        &mut self,
        identity: &PageIdentity,
        strokes: &mut Vec<Stroke>,
    ) -> io::Result<bool> {
        let Some(history) = self.history_mut(identity) else {
            return Ok(false);
        };
        let Some(action) = history.redo.last().cloned() else {
            return Ok(false);
        };
        action.apply_forward(strokes)?;
        history.redo.pop();
        history.undo.push(action);
        Ok(true)
    }

    #[cfg(feature = "takeover")]
    pub(crate) fn clear(&mut self, identity: &PageIdentity) {
        self.pages.retain(|history| history.identity != *identity);
    }

    fn history(&self, identity: &PageIdentity) -> Option<&PageEditHistory> {
        self.pages
            .iter()
            .find(|history| history.identity == *identity)
    }

    fn history_mut(&mut self, identity: &PageIdentity) -> Option<&mut PageEditHistory> {
        self.pages
            .iter_mut()
            .find(|history| history.identity == *identity)
    }

    fn history_mut_or_insert(&mut self, identity: PageIdentity) -> &mut PageEditHistory {
        if let Some(index) = self
            .pages
            .iter()
            .position(|history| history.identity == identity)
        {
            return &mut self.pages[index];
        }
        if self.pages.len() >= MAXIMUM_PAGES_WITH_HISTORY {
            self.pages.remove(0);
        }
        self.pages.push(PageEditHistory {
            identity,
            undo: Vec::new(),
            redo: Vec::new(),
        });
        self.pages.last_mut().unwrap()
    }

    fn validate(&self) -> io::Result<()> {
        if self.format_version != FORMAT_VERSION
            || self.pages.len() > MAXIMUM_PAGES_WITH_HISTORY
            || self.pages.iter().any(|history| {
                history.undo.len() > MAXIMUM_ACTIONS_PER_PAGE
                    || history.redo.len() > MAXIMUM_ACTIONS_PER_PAGE
            })
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "page edit history is invalid",
            ))
        } else {
            Ok(())
        }
    }
}

impl StrokeSequenceReplacement {
    fn between(before: &[Stroke], after: &[Stroke]) -> Option<Self> {
        let prefix = before
            .iter()
            .zip(after)
            .take_while(|(left, right)| left == right)
            .count();
        let suffix = before[prefix..]
            .iter()
            .rev()
            .zip(after[prefix..].iter().rev())
            .take_while(|(left, right)| left == right)
            .count();
        let removed_end = before.len() - suffix;
        let inserted_end = after.len() - suffix;
        if prefix == removed_end && prefix == inserted_end {
            return None;
        }
        Some(Self {
            start: prefix,
            removed: before[prefix..removed_end].to_vec(),
            inserted: after[prefix..inserted_end].to_vec(),
        })
    }

    fn apply_forward(&self, strokes: &mut Vec<Stroke>) -> io::Result<()> {
        replace_matching(strokes, self.start, &self.removed, &self.inserted)
    }

    fn apply_reverse(&self, strokes: &mut Vec<Stroke>) -> io::Result<()> {
        replace_matching(strokes, self.start, &self.inserted, &self.removed)
    }
}

fn replace_matching(
    strokes: &mut Vec<Stroke>,
    start: usize,
    expected: &[Stroke],
    replacement: &[Stroke],
) -> io::Result<()> {
    let end = start
        .checked_add(expected.len())
        .ok_or_else(incompatible_history)?;
    if strokes.get(start..end) != Some(expected) {
        return Err(incompatible_history());
    }
    strokes.splice(start..end, replacement.iter().cloned());
    Ok(())
}

fn incompatible_history() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "page strokes no longer match their edit history",
    )
}

fn histories_path(state_path: &Path) -> PathBuf {
    state_path.with_extension("histories.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use remarque_core::color::Color;
    use remarque_core::stroke::StrokePoint;

    fn identity() -> PageIdentity {
        PageIdentity {
            document_id: "notebook-1".to_owned(),
            page_index: 0,
        }
    }

    fn stroke(x: f32) -> Stroke {
        Stroke {
            color: Color::Black,
            points: vec![StrokePoint {
                x,
                y: 2.0,
                two_segment_distance_quarters: 0,
                width_quarter_pixels: 8,
                direction: 0,
                pressure: 0,
            }],
        }
    }

    #[test]
    fn stores_only_the_changed_middle_of_a_stroke_sequence() {
        let before = vec![stroke(1.0), stroke(2.0), stroke(3.0)];
        let after = vec![stroke(1.0), stroke(20.0), stroke(21.0), stroke(3.0)];
        let replacement = StrokeSequenceReplacement::between(&before, &after).unwrap();
        assert_eq!(replacement.start, 1);
        assert_eq!(replacement.removed, vec![stroke(2.0)]);
        assert_eq!(replacement.inserted, vec![stroke(20.0), stroke(21.0)]);
    }

    #[test]
    fn undo_and_redo_restore_exact_stroke_sequences() {
        let mut histories = PageEditHistories::default();
        let before = vec![stroke(1.0)];
        let after = vec![stroke(1.0), stroke(2.0)];
        histories.record(identity(), &before, &after);
        let mut current = after.clone();
        assert!(histories.undo(&identity(), &mut current).unwrap());
        assert_eq!(current, before);
        assert!(histories.redo(&identity(), &mut current).unwrap());
        assert_eq!(current, after);
    }

    #[test]
    fn history_survives_storage_and_a_new_edit_discards_redo() {
        let directory = std::env::temp_dir().join(format!(
            "remarque-page-histories-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let state_path = directory.join("document-library.json");
        let before = vec![stroke(1.0)];
        let after = vec![stroke(1.0), stroke(2.0)];
        let mut histories = PageEditHistories::default();
        histories.record(identity(), &before, &after);
        histories.save(&state_path).unwrap();

        let mut restored = PageEditHistories::restore(&state_path).unwrap();
        assert!(restored.availability(&identity()).can_undo);
        let mut current = after;
        restored.undo(&identity(), &mut current).unwrap();
        assert!(restored.availability(&identity()).can_redo);
        let edited = vec![stroke(1.0), stroke(3.0)];
        restored.record(identity(), &current, &edited);
        assert!(!restored.availability(&identity()).can_redo);

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn inserting_a_page_preserves_histories_for_shifted_pages() {
        let mut histories = PageEditHistories::default();
        let mut shifted = identity();
        shifted.page_index = 2;
        histories.record(shifted.clone(), &[], &[stroke(2.0)]);

        histories.insert_page("notebook-1", 1).unwrap();

        assert!(!histories.availability(&shifted).can_undo);
        shifted.page_index = 3;
        assert!(histories.availability(&shifted).can_undo);
    }
}
