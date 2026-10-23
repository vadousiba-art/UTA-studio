//! Snapshot-based undo/redo.
//!
//! Project documents are small (the audio lives in files), so storing whole
//! snapshots is simple, obviously correct and cheap: a 50-track project is a
//! few hundred KB. History depth is bounded.

use std::collections::VecDeque;

use crate::model::Project;

#[derive(Debug, Clone)]
struct Entry {
    label: String,
    before: Project,
}

#[derive(Debug, Clone)]
pub struct History {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(200)
    }
}

impl History {
    pub fn new(limit: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Record the state *before* an edit. Clears the redo stack.
    pub fn record(&mut self, label: impl Into<String>, before: Project) {
        self.undo.push_back(Entry {
            label: label.into(),
            before,
        });
        while self.undo.len() > self.limit {
            self.undo.pop_front();
        }
        self.redo.clear();
    }

    /// Undo: returns the label and swaps `current` with the previous state.
    pub fn undo(&mut self, current: &mut Project) -> Option<String> {
        let e = self.undo.pop_back()?;
        let after = std::mem::replace(current, e.before);
        self.redo.push(Entry {
            label: e.label.clone(),
            before: after,
        });
        Some(e.label)
    }

    pub fn redo(&mut self, current: &mut Project) -> Option<String> {
        let e = self.redo.pop()?;
        let before = std::mem::replace(current, e.before);
        self.undo.push_back(Entry {
            label: e.label.clone(),
            before,
        });
        Some(e.label)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|e| e.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit;

    #[test]
    fn undo_redo_cycle() {
        let mut p = Project::new("h", 48_000);
        let mut h = History::default();
        h.record("add track", p.clone());
        edit::add_track(&mut p, None);
        h.record("add track", p.clone());
        edit::add_track(&mut p, None);
        assert_eq!(p.tracks.len(), 2);
        assert_eq!(h.undo(&mut p).as_deref(), Some("add track"));
        assert_eq!(p.tracks.len(), 1);
        h.undo(&mut p);
        assert_eq!(p.tracks.len(), 0);
        assert!(h.undo(&mut p).is_none());
        h.redo(&mut p);
        h.redo(&mut p);
        assert_eq!(p.tracks.len(), 2);
        assert!(!h.can_redo());
        // a new edit after undo clears redo
        h.undo(&mut p);
        h.record("rename", p.clone());
        p.name = "x".into();
        assert!(!h.can_redo());
    }

    #[test]
    fn bounded() {
        let mut p = Project::new("h", 48_000);
        let mut h = History::new(3);
        for _ in 0..10 {
            h.record("e", p.clone());
            edit::add_track(&mut p, None);
        }
        let mut n = 0;
        while h.undo(&mut p).is_some() {
            n += 1;
        }
        assert_eq!(n, 3);
        assert_eq!(p.tracks.len(), 7);
    }
}
