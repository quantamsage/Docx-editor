//! Undo and redo.
//!
//! Before an edit batch is applied, the document's state is recorded as a
//! [`Snapshot`]; undoing restores the latest one (keeping the current state
//! for redo). Consecutive batches with the same group (e.g. typing in one
//! paragraph) form one undo step. The history is bounded by a number of
//! steps and a memory budget; the oldest steps are dropped first.

use crate::document::{Document, Snapshot};
use crate::error::Result;

/// Most undo steps kept.
pub const MAX_STEPS: usize = 200;
/// Most memory used by undo and redo snapshots together.
pub const MAX_BYTES: usize = 96 << 20;

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Group of the last recorded batch, while further batches of the same
    /// group may join its step.
    group: Option<String>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the state before an edit batch. A batch in the same group
    /// as the previous one joins its step (nothing is recorded). Returns
    /// whether a step was recorded, so that it can be dropped with
    /// [`History::forget_last`] if the batch fails.
    pub fn record(&mut self, doc: &Document, group: Option<&str>) -> bool {
        let joins = group.is_some() && self.group.as_deref() == group && !self.undo.is_empty();
        self.group = group.map(str::to_string);
        if joins {
            self.redo.clear();
            return false;
        }
        self.undo.push(doc.snapshot());
        self.redo.clear();
        self.trim();
        true
    }

    /// Drops the step recorded for a batch that failed.
    pub fn forget_last(&mut self) {
        self.undo.pop();
        self.group = None;
    }

    /// Undoes the last step. Returns false when there is nothing to undo.
    pub fn undo(&mut self, doc: &mut Document) -> Result<bool> {
        let Some(previous) = self.undo.pop() else { return Ok(false) };
        let current = doc.snapshot();
        if let Err(e) = doc.restore(&previous) {
            self.undo.push(previous);
            return Err(e);
        }
        self.redo.push(current);
        self.group = None;
        Ok(true)
    }

    /// Redoes the last undone step. Returns false when there is nothing to redo.
    pub fn redo(&mut self, doc: &mut Document) -> Result<bool> {
        let Some(next) = self.redo.pop() else { return Ok(false) };
        let current = doc.snapshot();
        if let Err(e) = doc.restore(&next) {
            self.redo.push(next);
            return Err(e);
        }
        self.undo.push(current);
        self.group = None;
        self.trim();
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Ends the current group: the next batch starts a new step.
    pub fn break_group(&mut self) {
        self.group = None;
    }

    fn trim(&mut self) {
        let mut bytes: usize = self.undo.iter().chain(&self.redo).map(Snapshot::size).sum();
        while self.undo.len() > 1 && (self.undo.len() > MAX_STEPS || bytes > MAX_BYTES) {
            bytes -= self.undo.remove(0).size();
        }
    }
}
