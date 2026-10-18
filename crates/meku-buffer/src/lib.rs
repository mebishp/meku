//! `meku-buffer`: open-file state, GPUI-free.
//!
//! Single owner of the [`ropey::Rope`]; edits bump a monotonic version used
//! to drop stale background parses. Never touches the window system.

use std::fs::File;
use std::io::BufReader;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ropey::{LineType, Rope};

/// Monotonic edit counter; background tasks check it to discard stale work.
pub type BufferVersion = u64;

/// Undo/redo entries kept per buffer before the oldest are dropped.
const UNDO_LIMIT: usize = 500;

#[derive(Debug, Clone)]
struct Snapshot {
    text: Rope,
    caret: usize,
    anchor: usize,
}

/// An open file: text, caret/selection, IME composition, undo history.
#[derive(Debug)]
pub struct OpenBuffer {
    /// Path relative to the mekuto root.
    pub rel: PathBuf,
    text: Rope,
    pub version: BufferVersion,
    pub dirty: bool,
    pub disk_mtime: Option<SystemTime>,
    caret: usize,
    anchor: usize,
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// True while consecutive single-character typing may merge into one undo step.
    coalescing: bool,
}

impl OpenBuffer {
    /// Stream a file into a rope without an intermediate whole-file String.
    pub fn load(rel: PathBuf, abs: &Path) -> anyhow::Result<Self> {
        let file = File::open(abs)?;
        let disk_mtime = file.metadata()?.modified().ok();
        let text = Rope::from_reader(BufReader::new(file))?;
        Ok(Self::with_text(rel, text, disk_mtime))
    }

    pub fn from_text(rel: PathBuf, text: String) -> Self {
        Self::with_text(rel, Rope::from_str(&text), None)
    }

    fn with_text(rel: PathBuf, text: Rope, disk_mtime: Option<SystemTime>) -> Self {
        Self {
            rel,
            text,
            version: 0,
            dirty: false,
            disk_mtime,
            caret: 0,
            anchor: 0,
            marked: None,
            undo: Vec::new(),
            redo: Vec::new(),
            coalescing: false,
        }
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }

    pub fn len_bytes(&self) -> usize {
        self.text.len()
    }

    pub fn line_count(&self) -> usize {
        self.text.len_lines(LineType::LF)
    }

    /// Apply an edit over a **byte** range (parser-style offsets) and return
    /// the new version. Coalescing of undo steps happens above this layer.
    pub fn apply_edit(&mut self, range: Range<usize>, insert: &str) -> BufferVersion {
        debug_assert!(
            range.start <= range.end
                && range.end <= self.text.len()
                && self.text.is_char_boundary(range.start)
                && self.text.is_char_boundary(range.end),
            "edit range must sit on char boundaries"
        );
        self.text.remove(range.clone());
        self.text.insert(range.start, insert);
        self.version += 1;
        self.dirty = true;
        self.version
    }

    // -- caret and selection ------------------------------------------------

    /// Caret byte offset (the moving end of the selection).
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Anchor byte offset (the fixed end of the selection).
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// Selected byte range, ordered. Empty when there is no selection.
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    /// Move the caret to `to` (snapped to a char boundary). With `extend`,
    /// the anchor stays put and the selection grows; otherwise it collapses.
    pub fn move_caret(&mut self, to: usize, extend: bool) {
        let to = self.snap(to.min(self.text.len()));
        self.caret = to;
        if !extend {
            self.anchor = to;
        }
        self.coalescing = false;
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
        self.coalescing = false;
    }

    /// Previous char boundary before `i` (0 at the start).
    pub fn prev_boundary(&self, i: usize) -> usize {
        let mut j = i.min(self.text.len());
        if j == 0 {
            return 0;
        }
        j -= 1;
        while j > 0 && !self.text.is_char_boundary(j) {
            j -= 1;
        }
        j
    }

    /// Next char boundary after `i` (the end at the end).
    pub fn next_boundary(&self, i: usize) -> usize {
        let len = self.text.len();
        let mut j = (i + 1).min(len);
        while j < len && !self.text.is_char_boundary(j) {
            j += 1;
        }
        j
    }

    fn snap(&self, i: usize) -> usize {
        let mut j = i.min(self.text.len());
        while j > 0 && !self.text.is_char_boundary(j) {
            j -= 1;
        }
        j
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend && self.selection().start != self.selection().end {
            let start = self.selection().start;
            self.move_caret(start, false);
        } else {
            let to = self.prev_boundary(self.caret);
            self.move_caret(to, extend);
        }
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend && self.selection().start != self.selection().end {
            let end = self.selection().end;
            self.move_caret(end, false);
        } else {
            let to = self.next_boundary(self.caret);
            self.move_caret(to, extend);
        }
    }

    // -- editing ------------------------------------------------------------

    /// Replace `range` with `insert` as one user edit. Records an undo step
    /// unless it continues a typing run; `typing` marks single-character input.
    pub fn replace(&mut self, range: Range<usize>, insert: &str, typing: bool) {
        let range = self.snap(range.start)..self.snap(range.end.max(range.start));
        if !(typing && self.coalescing) {
            self.push_undo();
        }
        self.redo.clear();
        self.apply_edit(range.clone(), insert);
        let caret = range.start + insert.len();
        self.caret = caret;
        self.anchor = caret;
        self.marked = None;
        self.coalescing = typing;
    }

    /// Replace the selection (or insert at the caret) with `text`.
    pub fn insert(&mut self, text: &str) {
        let typing = text.chars().count() == 1 && text != "\n";
        let range = self.selection();
        self.replace(range, text, typing);
    }

    pub fn delete_backward(&mut self) {
        if self.selection().start != self.selection().end {
            let range = self.selection();
            self.replace(range, "", false);
        } else if self.caret > 0 {
            let start = self.prev_boundary(self.caret);
            let end = self.caret;
            self.replace(start..end, "", false);
        }
    }

    pub fn delete_forward(&mut self) {
        if self.selection().start != self.selection().end {
            let range = self.selection();
            self.replace(range, "", false);
        } else if self.caret < self.text.len() {
            let start = self.caret;
            let end = self.next_boundary(self.caret);
            self.replace(start..end, "", false);
        }
    }

    /// Text currently selected, if any.
    pub fn selected_text(&self) -> String {
        let r = self.selection();
        self.text.slice(r).to_string()
    }

    fn push_undo(&mut self) {
        self.undo.push(Snapshot {
            text: self.text.clone(),
            caret: self.caret,
            anchor: self.anchor,
        });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    /// Undo the last user edit. Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(snap) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot {
            text: self.text.clone(),
            caret: self.caret,
            anchor: self.anchor,
        });
        self.restore(snap);
        true
    }

    /// Redo the last undone edit. Returns false when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot {
            text: self.text.clone(),
            caret: self.caret,
            anchor: self.anchor,
        });
        self.restore(snap);
        true
    }

    fn restore(&mut self, snap: Snapshot) {
        self.text = snap.text;
        self.caret = snap.caret.min(self.text.len());
        self.anchor = snap.anchor.min(self.text.len());
        self.version += 1;
        self.dirty = true;
        self.marked = None;
        self.coalescing = false;
    }

    // -- IME composition ----------------------------------------------------

    /// Byte range of the current IME composition, if one is in progress.
    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn set_marked(&mut self, range: Option<Range<usize>>) {
        self.marked = range;
    }

    /// Whole text as an owned string (cheap to call for the editor view).
    pub fn text_string(&self) -> String {
        self.text.to_string()
    }

    pub fn mark_saved(&mut self, mtime: SystemTime) {
        self.dirty = false;
        self.disk_mtime = Some(mtime);
    }

    /// Clone out the text for a background parse. The version lets the
    /// parser result be discarded if another edit landed meanwhile.
    pub fn snapshot(&self) -> (BufferVersion, String) {
        (self.version, self.text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn buf(text: &str) -> OpenBuffer {
        OpenBuffer::from_text(PathBuf::from("a.md"), text.to_string())
    }

    #[test]
    fn edit_bumps_version_and_dirties() {
        let mut b = buf("hello\n");
        assert_eq!(b.version, 0);
        assert!(!b.dirty);
        let v = b.apply_edit(0..0, "# ");
        assert_eq!(v, 1);
        assert_eq!(b.version, 1);
        assert!(b.dirty);
        assert_eq!(b.snapshot().1, "# hello\n");
    }

    #[test]
    fn edit_replaces_range() {
        let mut b = buf("hello world\n");
        b.apply_edit(6..11, "meku");
        assert_eq!(b.snapshot().1, "hello meku\n");
    }

    #[test]
    fn multibyte_offsets_are_byte_based() {
        let mut b = buf("héllo\n");
        // 'é' is 2 bytes: byte range 1..3 covers it.
        b.apply_edit(1..3, "e");
        assert_eq!(b.snapshot().1, "hello\n");
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let mut b = buf("x\n");
        b.apply_edit(0..0, "y");
        assert!(b.dirty);
        b.mark_saved(UNIX_EPOCH);
        assert!(!b.dirty);
        assert_eq!(b.disk_mtime, Some(UNIX_EPOCH));
    }

    #[test]
    fn snapshot_is_isolated_from_later_edits() {
        let mut b = buf("one\n");
        let (v, text) = b.snapshot();
        b.apply_edit(0..0, "two\n");
        assert_eq!(v, 0);
        assert_eq!(text, "one\n");
        assert_eq!(b.version, 1);
    }

    #[test]
    fn load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().join("note.md");
        std::fs::write(&abs, "# Title\n\nbody").unwrap();
        let b = OpenBuffer::load(PathBuf::from("note.md"), &abs).unwrap();
        assert!(!b.dirty);
        assert!(b.disk_mtime.is_some());
        assert_eq!(b.snapshot().1, "# Title\n\nbody");
        assert_eq!(b.line_count(), 3);
    }

    #[test]
    fn typing_run_merges_into_one_undo_step() {
        let mut b = buf("");
        for c in "abc".chars() {
            b.insert(&c.to_string());
        }
        assert_eq!(b.text_string(), "abc");
        assert!(b.undo());
        assert_eq!(b.text_string(), "");
        assert!(!b.undo());
    }

    #[test]
    fn movement_breaks_typing_coalescing() {
        let mut b = buf("");
        b.insert("a");
        b.move_caret(0, false);
        b.move_caret(1, false);
        b.insert("b");
        assert_eq!(b.text_string(), "ab");
        assert!(b.undo());
        assert_eq!(b.text_string(), "a");
    }

    #[test]
    fn undo_then_redo_restores_text_and_caret() {
        let mut b = buf("hi");
        b.move_caret(2, false);
        b.insert("!");
        assert!(b.undo());
        assert_eq!((b.text_string().as_str(), b.caret()), ("hi", 2));
        assert!(b.redo());
        assert_eq!((b.text_string().as_str(), b.caret()), ("hi!", 3));
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut b = buf("");
        b.insert("x");
        b.move_caret(1, false);
        b.delete_backward();
        assert!(b.undo());
        b.move_caret(0, false);
        b.insert("y");
        assert!(!b.redo());
    }

    #[test]
    fn backspace_removes_whole_multibyte_char() {
        let mut b = buf("aé");
        b.move_caret(3, false);
        b.delete_backward();
        assert_eq!(b.text_string(), "a");
    }

    #[test]
    fn selection_is_replaced_by_insert() {
        let mut b = buf("hello world");
        b.move_caret(0, false);
        b.move_caret(5, true);
        assert_eq!(b.selection(), 0..5);
        b.insert("bye");
        assert_eq!(b.text_string(), "bye world");
        assert_eq!(b.caret(), 3);
    }

    #[test]
    fn select_all_then_delete_clears_buffer() {
        let mut b = buf("text\nmore");
        b.select_all();
        b.delete_backward();
        assert_eq!(b.text_string(), "");
        assert!(b.dirty);
    }

    #[test]
    fn delete_forward_and_arrow_collapse_selection() {
        let mut b = buf("abc");
        b.move_caret(0, false);
        b.delete_forward();
        assert_eq!(b.text_string(), "bc");
        b.move_caret(0, true);
        b.move_right(false);
        assert_eq!(b.selection(), 1..1);
    }

    #[test]
    fn arrows_step_by_char_not_byte() {
        let mut b = buf("é!");
        b.move_caret(0, false);
        b.move_right(false);
        assert_eq!(b.caret(), 2);
        b.move_left(false);
        assert_eq!(b.caret(), 0);
    }

    #[test]
    fn marked_text_is_tracked_and_cleared_by_edit() {
        let mut b = buf("");
        b.set_marked(Some(0..2));
        assert_eq!(b.marked(), Some(0..2));
        b.insert("k");
        assert_eq!(b.marked(), None);
    }
}
