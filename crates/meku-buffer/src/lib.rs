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

/// Word classes for subword stepping: alphanumeric words, punctuation
/// runs of any length, and whitespace as the never-stop class.
const CHAR_WORD: u8 = 0;
fn char_class(c: char) -> u8 {
    if c.is_alphanumeric() {
        CHAR_WORD
    } else if c.is_whitespace() {
        2
    } else {
        1
    }
}

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

    // -- word and line navigation ---------------------------------------------

    /// Byte index of the start of the `LineType::LF` line containing
    /// `caret` (or up to `caret` — text may end without a newline).
    fn line_start_byte(&self, caret: usize) -> usize {
        let line = self
            .text
            .byte_to_line_idx(caret.min(self.text.len()), LineType::LF);
        self.text.line_to_byte_idx(line, LineType::LF)
    }
    /// The char ending at byte `i`, plus its start byte. `None` at the
    /// text start.
    fn char_before(&self, i: usize) -> Option<(char, usize)> {
        if self.text.len() == 0 || i == 0 {
            return None;
        }
        let start = self.text.floor_char_boundary(i - 1);
        let c = self.text.get_char(start).ok()?;
        Some((c, start))
    }

    /// The char starting at byte `i`, plus its end byte. `None` at the
    /// text end.
    fn char_after(&self, i: usize) -> Option<(char, usize)> {
        let mut start = i.min(self.text.len());
        while start < self.text.len() && !self.text.is_char_boundary(start) {
            start += 1;
        }
        let c = self.text.get_char(start).ok()?;
        Some((c, start + c.len_utf8()))
    }

    /// A word boundary sits between `a` (left) and `b` (right) when the
    /// left char is lower-case or a digit and the right char upper-case
    /// (`camelCase` splits into `camel` + `Case`). Class differences
    /// (word/punctuation/whitespace) are handled by the scanning loops.
    fn subword_boundary(a: char, b: char) -> bool {
        (a.is_lowercase() || a.is_ascii_digit()) && b.is_uppercase()
    }

    /// Start of the subword run to the left of `from`: skips whitespace,
    /// then walks back over a same-class run. Stops at camel humps, so
    /// `camelCase|` lands before `Case`, then before `camel`. A punctuation
    /// run (`**`, `]]`) counts as one word.
    pub fn word_left(&self, from: usize) -> usize {
        if self.text.len() == 0 {
            return 0;
        }
        let mut i = from.min(self.text.len());
        // Whitespace (newlines included) never forms a stop — walk it off.
        while i > 0 {
            let Some((c, start)) = self.char_before(i) else {
                break;
            };
            if !c.is_whitespace() {
                break;
            }
            i = start;
        }
        let Some((c1, run_end)) = self.char_before(i) else {
            return 0;
        };
        let cls = char_class(c1);
        let mut dest = run_end;
        let mut left = c1;
        while dest > 0 {
            let (p, start) = match self.char_before(dest) {
                Some(x) => x,
                None => break,
            };
            if char_class(p) != cls || (cls == CHAR_WORD && Self::subword_boundary(p, left)) {
                break;
            }
            dest = start;
            left = p;
        }
        dest
    }

    /// End of the subword run to the right of `from` — the mirror of
    /// [`OpenBuffer::word_left`]. Whitespace is skipped; `camelCase|` from
    /// before `camel` stops before `Case`. A punctuation run counts as
    /// one word.
    pub fn word_right(&self, from: usize) -> usize {
        let len = self.text.len();
        if from >= len {
            return len;
        }
        let mut i = from;
        while i < len {
            let Some((c, _)) = self.char_after(i) else {
                break;
            };
            if !c.is_whitespace() {
                break;
            }
            i += c.len_utf8();
        }
        let Some((c1, end)) = self.char_after(i) else {
            return len;
        };
        let cls = char_class(c1);
        let mut last = c1;
        let mut end = end;
        while end < len {
            let (c, next_end) = match self.char_after(end) {
                Some(x) => x,
                None => break,
            };
            if char_class(c) != cls || (cls == CHAR_WORD && Self::subword_boundary(last, c)) {
                break;
            }
            last = c;
            end = next_end;
        }
        end
    }

    /// Delete from the caret to the start of the previous subword
    /// (`Ctrl+Backspace`). A non-empty selection is deleted whole.
    pub fn delete_word_backward(&mut self) {
        let r = self.selection();
        if r.start != r.end {
            self.replace(r, "", false);
            return;
        }
        let start = self.word_left(self.caret);
        if start < self.caret {
            self.replace(start..self.caret, "", false);
        }
    }

    /// Delete from the caret to the start of the next subword
    /// (`Ctrl+Delete`), stopping at the following space.
    pub fn delete_word_forward(&mut self) {
        let r = self.selection();
        if r.start != r.end {
            self.replace(r, "", false);
            return;
        }
        let end = self.word_right(self.caret);
        if end > self.caret {
            self.replace(self.caret..end, "", false);
        }
    }

    /// Delete everything from the caret back to the start of its line,
    /// whitespace included (`Ctrl+Shift+Backspace`). At the line start the
    /// preceding newline goes instead (the line joins the one above).
    pub fn delete_to_line_start(&mut self) {
        let r = self.selection();
        if r.start != r.end {
            self.replace(r, "", false);
            return;
        }
        let line_start = self.line_start_byte(self.caret);
        if self.caret > line_start {
            self.replace(line_start..self.caret, "", false);
        } else if self.caret > 0 {
            self.replace(self.prev_boundary(self.caret)..self.caret, "", false);
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

    // -- word and line navigation -----------------------------------

    #[test]
    fn word_left_steps_through_words_and_punct_runs() {
        let b = buf("# Hello **world");
        // Indexes: '#'=0 ' '=1 "Hello"=2..7 ' '=7 "**"=8..10 "world"=10..15
        assert_eq!(b.word_left(15), 10, "back to word start");
        assert_eq!(b.word_left(10), 8, "punctuation run moves as one");
        assert_eq!(b.word_left(8), 2, "skips the space behind **");
        assert_eq!(b.word_left(2), 0, "# is the last run");
        assert_eq!(b.word_left(0), 0);
    }

    #[test]
    fn word_left_skips_whitespace_to_next_word() {
        let b = buf("one two");
        assert_eq!(b.word_left(5), 4, "mid-'two' lands at the run start");
        assert_eq!(b.word_left(4), 0, "at word start crosses to the run before");
        assert_eq!(b.word_left(7), 4, "end of text goes to the same run start");
        assert_eq!(b.word_left(0), 0);
    }

    #[test]
    fn word_left_respects_camel_humps() {
        let b = buf("camelCase");
        assert_eq!(b.word_left(9), 5, "Case starts at the hump");
        assert_eq!(b.word_left(5), 0);
    }

    #[test]
    fn word_right_mirrors_word_left() {
        let b = buf("# Hello **world");
        assert_eq!(b.word_right(0), 1);
        assert_eq!(b.word_right(1), 7);
        assert_eq!(b.word_right(7), 10, "the whitespace joins the ** step");
        assert_eq!(b.word_right(8), 10);
        assert_eq!(b.word_right(10), 15);
        assert_eq!(b.word_right(15), 15);
    }

    #[test]
    fn word_right_stops_at_camel_humps() {
        let b = buf("camelCase");
        assert_eq!(b.word_right(0), 5);
        assert_eq!(b.word_right(5), 9);
    }

    #[test]
    fn word_moves_cross_lines_through_newlines() {
        let b = buf("one\n\ntwo");
        // From the blank line the caret walks back to "one"; the next word
        // on the right is "two".
        assert_eq!(b.word_left(5), 0);
        assert_eq!(b.word_right(0), 3, "end of 'one'");
        assert_eq!(b.word_right(3), 8, "through the breaks into 'two'");
    }

    #[test]
    fn delete_word_backward_removes_the_word_so_far() {
        let mut b = buf("one two\n");
        b.move_caret(7, false);
        b.delete_word_backward();
        assert_eq!(b.text_string(), "one \n");
        assert_eq!(b.caret(), 4);
    }

    #[test]
    fn delete_word_forward_stops_before_its_space() {
        let mut b = buf("one two");
        b.delete_word_forward();
        assert_eq!(b.text_string(), " two");
        b.delete_word_forward();
        assert_eq!(b.text_string(), "", "the gap goes with the second delete");
    }

    #[test]
    fn delete_to_line_start_keeps_newline_of_other_lines() {
        let mut b = buf("   kept\n  indented more\nafter");
        // Caret at byte 19: start of "more" on the indent-2 line.
        b.move_caret(19, false);
        b.delete_to_line_start();
        assert_eq!(b.text_string(), "   kept\nmore\nafter");
    }

    #[test]
    fn delete_to_line_start_at_line_start_joins_lines() {
        let mut b = buf("above\nbelow");
        b.move_caret(6, false);
        b.delete_to_line_start();
        assert_eq!(b.text_string(), "abovebelow");
    }

    #[test]
    fn long_document_word_stepping_matches_naive_scan() {
        // Chunks boundaries (rope chunks are bounded) must not break the scan.
        let mut s = String::new();
        for i in 0..600 {
            s.push_str(&format!("word{i} "));
        }
        s.push_str("camelTail");
        let b = buf(&s);
        assert_eq!(b.word_left(s.len()), 4695, "start of the 'T' hump (len-4)");
        assert_eq!(
            b.word_left(4695),
            4690,
            "next run is 'word…' + space (len-9)"
        );
        assert_eq!(b.word_right(4695), s.len());
    }
}
