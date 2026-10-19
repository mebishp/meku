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

/// Case transforms for [`OpenBuffer::transform_case`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseKind {
    Upper,
    Lower,
    Title,
    Toggle,
}

/// One rewritten line: `(old start, old len, new start, (cut_at, removed, added))`.
type LineSplice = (usize, usize, usize, (usize, usize, usize));

/// Deepest list nesting level (0-based): bullets and tasks stop at three
/// levels. Matches `meku_markdown::live::MAX_LIST_DEPTH`.
const MAX_LIST_DEPTH: usize = 2;

/// Target of [`OpenBuffer::convert_list`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Ordered,
    Task,
}

/// What kind of markdown line marker a line carries, as far as plain-text
/// pattern matching can tell (list bullets, tasks, ordered numbers, quotes).
#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkerKind {
    Bullet(char),
    Task(char),
    Ordered(u32, char),
    /// Quote depth; the prefix bytes are re-taken from the line.
    Quote(usize),
    /// A list item directly inside a quote (`> - item`); the inner marker's
    /// exact byte length rides along (digits may differ from `n`'s).
    QuoteList {
        depth: usize,
        list: Box<(MarkerKind, usize)>,
    },
}

/// Parsed leading marker of one line: `indent` whitespace, then marker bytes
/// up to (excluding) the item's own content.
#[derive(Debug)]
struct Marker {
    indent: usize,
    prefix_len: usize,
    kind: MarkerKind,
}

/// Text (without the newline) that the next line starts with after Enter on
/// a line carrying `marker`. Tasks restart unchecked; ordered numbers count
/// up; a callout's `[!NOTE]` tag is content and is never repeated.
fn continuation_text(marker: &Marker, line: &str) -> String {
    let indent = &line[..marker.indent];
    let inner_text = |kind: &MarkerKind| -> String {
        match kind {
            MarkerKind::Bullet(c) => format!("{c} "),
            MarkerKind::Task(_) => "[ ] ".to_string(),
            MarkerKind::Ordered(n, sep) => format!("{}{sep} ", n + 1),
            _ => String::new(),
        }
    };
    match &marker.kind {
        MarkerKind::Bullet(c) => format!("{indent}{c} "),
        MarkerKind::Task(c) => format!("{indent}{c} [ ] "),
        MarkerKind::Ordered(n, sep) => format!("{indent}{}{sep} ", n + 1),
        MarkerKind::Quote(_) => format!("{indent}{}", &line[marker.indent..marker.prefix_len]),
        MarkerKind::QuoteList { list, .. } => {
            let (kind, len) = &**list;
            let quote = &line[marker.indent..marker.prefix_len - len];
            format!("{indent}{quote}{}", inner_text(kind))
        }
    }
}

/// Recognize the leading marker of a (newline-stripped) line. Returns
/// `None` for prose, headings, fences and anything else that has no
/// continuation semantics.
fn parse_marker(line: &str) -> Option<Marker> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];

    // Quotes (possibly stacked `>>`), each `>` optionally followed by spaces.
    let mut scan = rest;
    let mut depth = 0usize;
    while let Some(tail) = scan.strip_prefix('>') {
        depth += 1;
        scan = tail.trim_start_matches(' ');
    }
    if depth == 0 {
        return parse_list_marker(rest).map(|(len, kind)| Marker {
            indent,
            prefix_len: indent + len,
            kind,
        });
    }
    let quote_len = rest.len() - scan.len();
    let kind = match parse_list_marker(scan) {
        Some((len, list)) => MarkerKind::QuoteList {
            depth,
            list: Box::new((list, len)),
        },
        None => MarkerKind::Quote(depth),
    };
    Some(Marker {
        indent,
        prefix_len: indent + quote_len + kind.marker_len(),
        kind,
    })
}

impl MarkerKind {
    /// Byte length of the marker itself, excluding any quote/indent prefix.
    /// Only meaningful for list markers nested in a quote (`QuoteList`); the
    /// flat variants get their length straight from `parse_list_marker`.
    fn marker_len(&self) -> usize {
        match self {
            MarkerKind::QuoteList { list, .. } => list.1,
            _ => 0,
        }
    }
}

/// A `- `/`* `/`+ ` bullet, `- [x] `/`- [ ] ` task or `12. `/`12) ` ordered
/// item at the start of `rest` (after any indent). Returns the byte length
/// of the marker (trailing spaces included) and its kind.
fn parse_list_marker(rest: &str) -> Option<(usize, MarkerKind)> {
    let bytes = rest.as_bytes();
    if matches!(bytes.first(), Some(b'-') | Some(b'*') | Some(b'+')) {
        let bullet = bytes[0] as char;
        let after_bullet = &rest[1..];
        return if let Some(space) = after_bullet.strip_prefix(' ') {
            let task = space
                .strip_prefix("[ ]")
                .or_else(|| space.strip_prefix("[x]"))
                .or_else(|| space.strip_prefix("[X]"));
            if let Some(task_tail) = task {
                let len = 5 + usize::from(task_tail.starts_with(' '));
                Some((len, MarkerKind::Task(bullet)))
            } else {
                Some((2, MarkerKind::Bullet(bullet)))
            }
        } else {
            None
        };
    }
    // Ordered: digits, `.` or `)`, at least one space.
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let sep = match bytes.get(digits) {
        Some(b'.') => '.',
        Some(b')') => ')',
        _ => return None,
    };
    let after = &rest[digits + 1..];
    if !after.starts_with(' ') {
        return None;
    }
    let spaces = after.len() - after.trim_start().len();
    let mut num = 0u32;
    for d in rest[..digits].bytes() {
        num = num * 10 + u32::from(d - b'0');
    }
    Some((digits + 1 + spaces, MarkerKind::Ordered(num, sep)))
}

/// Content hash of a string, comparable with [`OpenBuffer::disk_hash`].
pub fn hash_text(text: &str) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(text.as_bytes());
    h.finish()
}

fn rope_hash(text: &Rope) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for chunk in text.chunks() {
        h.write(chunk.as_bytes());
    }
    h.finish()
}

/// A single-token `http(s)://` / `mailto:` destination.
fn looks_like_url(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && !s.contains(char::is_whitespace)
        && (s.starts_with("http://") || s.starts_with("https://") || s.starts_with("mailto:"))
}

/// All-whitespace tail of a (newline-stripped) line.
fn blank_tail(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// Upper-case the first alphanumeric of every word; the rest goes lower.
fn to_title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_boundary = true;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if at_boundary {
                out.extend(c.to_uppercase());
                at_boundary = false;
            } else {
                out.extend(c.to_lowercase());
            }
        } else {
            at_boundary = true;
            out.push(c);
        }
    }
    out
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
    /// Hash of the text last read from or written to disk, so a watcher
    /// event for our own save is recognised by content, however much has
    /// been typed since.
    pub disk_hash: Option<u64>,
    caret: usize,
    anchor: usize,
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// True while consecutive single-character typing may merge into one undo step.
    coalescing: bool,
    /// Auto-pairing of brackets/quotes/markers while typing (settings toggle).
    pub auto_close: bool,
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
        let disk_hash = disk_mtime.map(|_| rope_hash(&text));
        Self {
            rel,
            text,
            version: 0,
            dirty: false,
            disk_mtime,
            disk_hash,
            caret: 0,
            anchor: 0,
            marked: None,
            undo: Vec::new(),
            redo: Vec::new(),
            coalescing: false,
            auto_close: true,
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

    // -- line editing ----------------------------------------------------------

    /// Byte span of the `LineType::LF` line `idx`, trailing newline included
    /// (the last line may lack one).
    fn line_span(&self, idx: usize) -> Range<usize> {
        let start = self.text.line_to_byte_idx(idx, LineType::LF);
        let end = if idx + 1 < self.text.len_lines(LineType::LF) {
            self.text.line_to_byte_idx(idx + 1, LineType::LF)
        } else {
            self.text.len()
        };
        start..end
    }

    /// Lines touched by the selection: the caret's line when collapsed,
    /// otherwise every line from the selection start to the char before its
    /// end (a selection ending exactly at a line start does not claim that
    /// line).
    fn selected_lines(&self) -> Range<usize> {
        let r = self.selection();
        let first = self
            .text
            .byte_to_line_idx(r.start.min(self.text.len()), LineType::LF);
        let last = if r.start == r.end {
            first
        } else {
            self.text
                .byte_to_line_idx((r.end - 1).min(self.text.len()), LineType::LF)
        };
        first..last + 1
    }

    /// One user-level edit over a byte range with an explicit caret/anchor
    /// outcome (unlike [`OpenBuffer::replace`], which collapses onto the
    /// insertion end). Always one undo step.
    fn edit_region(&mut self, range: Range<usize>, insert: &str, caret: usize, anchor: usize) {
        if range.start == range.end && insert.is_empty() {
            return;
        }
        self.push_undo();
        self.redo.clear();
        self.apply_edit(range, insert);
        self.caret = caret.min(self.text.len());
        self.anchor = anchor.min(self.text.len());
        self.marked = None;
        self.coalescing = false;
    }

    /// Swap the selected line block with the line above it (`Alt+Up`). The
    /// caret and selection ride along inside the block. No-op on the first
    /// line.
    pub fn move_line_up(&mut self) {
        let lines = self.selected_lines();
        if lines.start == 0 {
            return;
        }
        let prev = self.line_span(lines.start - 1);
        let block_start = prev.end;
        let block_end = self.line_span(lines.end - 1).end;
        let region_start = prev.start;
        let prev_text = self.text.slice(prev).to_string();
        let block_text = self.text.slice(block_start..block_end).to_string();
        let region_ends_nl = block_text.ends_with('\n');
        let pb = prev_text.strip_suffix('\n').unwrap_or(&prev_text);
        let bb = block_text.strip_suffix('\n').unwrap_or(&block_text);
        let tail = if region_ends_nl { "\n" } else { "" };
        let new = format!("{bb}\n{pb}{tail}");
        // The block keeps its content and now starts where the previous
        // line did; a last line without a newline gains its separator after
        // the content, so inner positions shift purely by the block move.
        let map = |p: usize| -> usize {
            if p < block_start {
                p
            } else if p <= block_end {
                region_start + (p - block_start)
            } else {
                p
            }
        };
        let (caret, anchor) = (map(self.caret), map(self.anchor));
        self.edit_region(region_start..block_end, &new, caret, anchor);
    }

    /// Swap the selected line block with the line below it (`Alt+Down`).
    /// No-op when the block already ends at the document's last line, or
    /// when the only line below is the phantom empty line after a trailing
    /// newline.
    pub fn move_line_down(&mut self) {
        let lines = self.selected_lines();
        let last = self.text.len_lines(LineType::LF) - 1;
        if lines.end > last {
            return;
        }
        let block_start = self.line_span(lines.start).start;
        let next = self.line_span(lines.end);
        if next.start == next.end && lines.end + 1 > last {
            return;
        }
        let block_end = next.start;
        let block_text = self.text.slice(block_start..block_end).to_string();
        let next_text = self.text.slice(next.clone()).to_string();
        let region_ends_nl = next_text.ends_with('\n');
        let bb = block_text.strip_suffix('\n').unwrap_or(&block_text);
        let nb = next_text.strip_suffix('\n').unwrap_or(&next_text);
        let tail = if region_ends_nl { "\n" } else { "" };
        let new = format!("{nb}\n{bb}{tail}");
        let shift = nb.len() + 1;
        let map = |p: usize| -> usize {
            if p < block_start {
                p
            } else if p <= block_end {
                block_start + shift + (p - block_start)
            } else {
                p
            }
        };
        let (caret, anchor) = (map(self.caret), map(self.anchor));
        self.edit_region(block_start..next.end, &new, caret, anchor);
    }

    /// Copy the selected line block below itself (`Ctrl+Shift+D`); the caret
    /// and selection ride into the copy at the same offset.
    pub fn duplicate_line(&mut self) {
        let lines = self.selected_lines();
        let block_start = self.line_span(lines.start).start;
        let block_end = self.line_span(lines.end - 1).end;
        let copy = self.text.slice(block_start..block_end).to_string();
        // A final block without a newline needs one of its own so the copy
        // becomes a line of its own.
        let spaced = copy.ends_with('\n');
        let insert = if spaced {
            copy.clone()
        } else {
            format!("\n{copy}")
        };
        let content_start = block_end + usize::from(!spaced);
        let map = |p: usize| -> usize {
            if p < block_start {
                p
            } else if p <= block_end {
                content_start + (p - block_start)
            } else {
                p
            }
        };
        let (caret, anchor) = (map(self.caret), map(self.anchor));
        self.edit_region(block_end..block_end, &insert, caret, anchor);
    }

    /// Delete the selected line block, newlines included (`Ctrl+Shift+K`).
    /// The caret lands where the block began.
    pub fn delete_line(&mut self) {
        if self.text.len() == 0 {
            return;
        }
        let lines = self.selected_lines();
        let block_start = self.line_span(lines.start).start;
        let block_end = self.line_span(lines.end - 1).end;
        self.edit_region(block_start..block_end, "", block_start, block_start);
    }

    /// Insert an empty line below the caret's line (`Ctrl+Enter`) and move
    /// the caret onto it.
    pub fn insert_line_below(&mut self) {
        let at = self.line_span(self.caret_line()).end;
        self.edit_region(at..at, "\n", at, at);
    }

    /// Insert an empty line above the caret's line (`Ctrl+Shift+Enter`) and
    /// move the caret onto it.
    pub fn insert_line_above(&mut self) {
        let at = self.line_span(self.caret_line()).start;
        self.edit_region(at..at, "\n", at, at);
    }

    fn caret_line(&self) -> usize {
        self.text
            .byte_to_line_idx(self.caret.min(self.text.len()), LineType::LF)
    }

    /// Merge the caret's line with the one below (`Ctrl+J`): the newline and
    /// the next line's indentation go away; a single space is kept at the
    /// seam unless either side is empty or already has whitespace there.
    pub fn join_lines(&mut self) {
        let idx = self.caret_line();
        if idx + 1 >= self.text.len_lines(LineType::LF) {
            return;
        }
        let cur = self.line_span(idx);
        let next = self.line_span(idx + 1);
        let cur_text = self.text.slice(cur.clone()).to_string();
        let next_text = self.text.slice(next.clone()).to_string();
        let c = cur_text.strip_suffix('\n').unwrap_or(&cur_text);
        let n = next_text.strip_suffix('\n').unwrap_or(&next_text);
        let lead = n.len() - n.trim_start().len();
        let seam_start = cur.start + c.len();
        let seam_end = next.start + lead;
        let sep = if c.is_empty() || n.is_empty() || c.ends_with(char::is_whitespace) {
            ""
        } else {
            " "
        };
        let caret = seam_start + sep.len();
        self.edit_region(seam_start..seam_end, sep, caret, caret);
    }

    /// Sort the selected lines (byte order, stable). No-op without a
    /// selection. A palette command in v0.1 — no default key.
    pub fn sort_selected_lines(&mut self) {
        let r = self.selection();
        if r.start == r.end {
            return;
        }
        let lines = self.selected_lines();
        let block_start = self.line_span(lines.start).start;
        let block_end = self.line_span(lines.end - 1).end;
        let text = self.text.slice(block_start..block_end).to_string();
        let (suffix, body) = match text.strip_suffix('\n') {
            Some(body) => ("\n", body),
            None => ("", &text[..]),
        };
        let mut ls: Vec<&str> = body.split('\n').collect();
        ls.sort();
        let new = format!("{}{}{}", ls.join("\n"), suffix, "");
        let caret = block_start + new.len();
        self.edit_region(block_start..block_end, &new, caret, block_start);
    }

    /// Case transforms for the selection, or the alphanumeric word the caret
    /// sits in when the selection is collapsed. A palette command in v0.1.
    pub fn transform_case(&mut self, kind: CaseKind) {
        let r = self.selection();
        let range = if r.start != r.end {
            r
        } else {
            let mut s = self.caret;
            let mut e = self.caret;
            while let Some((c, start)) = self.char_before(s) {
                if char_class(c) != CHAR_WORD {
                    break;
                }
                s = start;
            }
            while let Some((c, end)) = self.char_after(e) {
                if char_class(c) != CHAR_WORD {
                    break;
                }
                e = end;
            }
            if s == e {
                return;
            }
            s..e
        };
        let text = self.text.slice(range.clone()).to_string();
        let new = match kind {
            CaseKind::Upper => text.to_uppercase(),
            CaseKind::Lower => text.to_lowercase(),
            CaseKind::Title => to_title_case(&text),
            CaseKind::Toggle => text
                .chars()
                .map(|c| {
                    if c.is_uppercase() {
                        c.to_lowercase().to_string()
                    } else {
                        c.to_uppercase().to_string()
                    }
                })
                .collect(),
        };
        let end = range.start + new.len();
        self.edit_region(range.clone(), &new, end, range.start);
    }

    /// Toggle `prefix` line comments over the selected lines (or the caret's
    /// line): commented → uncommented, otherwise commented. Blank lines are
    /// left alone. The caller picks the prefix from the fence language.
    pub fn toggle_line_comment(&mut self, prefix: &str) {
        if prefix.is_empty() {
            return;
        }
        let lines = self.selected_lines();
        let spans: Vec<Range<usize>> = (lines.start..lines.end)
            .map(|i| self.line_span(i))
            .collect();
        let contents: Vec<String> = spans
            .iter()
            .map(|sp| self.text.slice(sp.clone()).to_string())
            .collect();
        let commented = contents.iter().all(|line| {
            let t = line.trim_end_matches('\n').trim_start();
            t.is_empty() || t.starts_with(prefix)
        });
        // Per-line byte deltas, applied bottom-up so offsets stay valid.
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        if commented {
            for (sp, line) in spans.iter().zip(&contents) {
                let body = line.trim_end_matches('\n');
                let indent = body.len() - body.trim_start().len();
                let rest = &body[indent..];
                if rest.is_empty() {
                    continue;
                }
                let mut cut = indent + prefix.len();
                if rest[prefix.len()..].starts_with(' ') {
                    cut += 1;
                }
                edits.push((sp.start + indent, sp.start + cut, String::new()));
            }
        } else {
            for (sp, line) in spans.iter().zip(&contents) {
                let body = line.trim_end_matches('\n');
                if body.trim().is_empty() {
                    continue;
                }
                let indent = body.len() - body.trim_start().len();
                edits.push((sp.start + indent, sp.start + indent, format!("{prefix} ")));
            }
        }
        if edits.is_empty() {
            return;
        }
        self.push_undo();
        self.redo.clear();
        for (start, end, insert) in edits.iter().rev() {
            self.apply_edit(*start..*end, insert);
        }
        // Map the caret/anchor through the cumulative deltas of the edits
        // before their line.
        let map = |p: usize, edits: &[(usize, usize, String)]| -> usize {
            let mut out = p;
            for (start, end, insert) in edits {
                if *start < p || (*start == p && *end == *start && !insert.is_empty()) {
                    out += insert.len().saturating_sub(end - start);
                }
            }
            out
        };
        let caret = map(self.caret.min(self.text.len()), &edits);
        let anchor = map(self.anchor.min(self.text.len()), &edits);
        self.caret = caret.min(self.text.len());
        self.anchor = anchor.min(self.text.len());
        self.marked = None;
        self.coalescing = false;
    }

    // -- enter continuations --------------------------------------------------

    /// Text of the `LineType::LF` line around the caret, newline stripped.
    /// Returns the line's start byte and its body.
    fn line_body(&self) -> (usize, String) {
        let span = self.line_span(self.caret_line());
        let s = self.text.slice(span.start..span.end).to_string();
        let body = s.strip_suffix('\n').unwrap_or(&s).to_string();
        (span.start, body)
    }

    /// Type-smart Enter: bullets/tasks/quotes continue themselves, ordered
    /// lists count up (and renumber the whole block), an empty item exits,
    /// prose/headings/fences fall back to a plain newline. Selections are
    /// replaced first, like typing.
    pub fn smart_enter(&mut self) {
        if let Some(r) = self.nonempty_selection() {
            self.replace(r, "\n", false);
            return;
        }
        let (line_start, line) = self.line_body();
        let Some(marker) = parse_marker(&line) else {
            self.insert("\n");
            return;
        };
        let caret_in = self.caret - line_start;
        let content = &line[marker.prefix_len..];

        // Empty item: the marker alone on the line ends the list/quote.
        if blank_tail(content) {
            self.edit_region(
                line_start + marker.indent..line_start + marker.prefix_len,
                "",
                line_start + marker.indent,
                line_start + marker.indent,
            );
            return;
        }
        // Caret inside the marker: the newline goes above the whole item.
        if caret_in <= marker.prefix_len {
            self.edit_region(
                line_start..line_start,
                "\n",
                self.caret + 1,
                self.anchor + 1,
            );
            return;
        }
        self.push_undo();
        self.redo.clear();
        let cont = continuation_text(&marker, &line);
        let insert = format!("\n{cont}");
        // The whitespace run between the split and the next word rides below,
        // not above: `- wash| dishes` becomes `- wash\n- dishes`.
        let rest = &line[caret_in..];
        let strip = rest.len() - rest.trim_start().len();
        let range = self.caret..self.caret + strip;
        self.apply_edit(range, &insert);
        let at = self.caret + insert.len();
        self.caret = at;
        self.anchor = at;
        self.marked = None;
        self.coalescing = false;
        if let MarkerKind::Ordered(..) = marker.kind {
            let idx = self.text.byte_to_line_idx(at, LineType::LF);
            self.renumber_ordered_from(idx);
        }
    }

    /// Soft line break (`Shift+Enter`): a raw newline that keeps the current
    /// item's indentation (lists ride at the content column, quotes repeat
    /// the `>` prefix so the block stays explicit), no new marker.
    pub fn soft_break(&mut self) {
        if let Some(r) = self.nonempty_selection() {
            self.replace(r, "\n", false);
            return;
        }
        let (_line_start, line) = self.line_body();
        let cont = match parse_marker(&line) {
            None => {
                self.insert("\n");
                return;
            }
            Some(ref marker) => match marker.kind {
                MarkerKind::Quote(_) | MarkerKind::QuoteList { .. } => {
                    continuation_text(marker, &line)
                }
                _ => line[..marker.prefix_len]
                    .chars()
                    .map(|c| if c.is_whitespace() { c } else { ' ' })
                    .collect::<String>(),
            },
        };
        let insert = format!("\n{cont}");
        self.replace(self.selection(), &insert, false);
    }

    fn nonempty_selection(&self) -> Option<Range<usize>> {
        let r = self.selection();
        (r.start != r.end).then_some(r)
    }

    /// Marker facts of line `idx`: `(indent, number, separator)` when the
    /// line carries an ordered item.
    fn ordered_info(&self, idx: usize) -> Option<(usize, u32, char)> {
        let span = self.line_span(idx);
        let s = self.text.slice(span.start..span.end).to_string();
        let body = s.strip_suffix('\n').unwrap_or(&s);
        let indent = body.len() - body.trim_start().len();
        match parse_list_marker(&body[indent..]) {
            Some((_, MarkerKind::Ordered(n, sep))) => Some((indent, n, sep)),
            _ => None,
        }
    }

    /// Body (newline stripped) of line `idx`.
    fn line_string(&self, idx: usize) -> String {
        let span = self.line_span(idx);
        let s = self.text.slice(span.start..span.end).to_string();
        s.strip_suffix('\n').unwrap_or(&s).to_string()
    }

    /// Renumber the ordered run at line `idx`'s indent: items keep the
    /// first line's number and count up from there, so an inserted or
    /// removed item straightens its neighbours. Deeper-indented lines
    /// (nested children) are stepped over; a blank line, a shallower line
    /// or a non-ordered line at the same indent ends the run. Raw edits —
    /// never a second undo step on its own.
    fn renumber_ordered_from(&mut self, idx: usize) {
        let Some((want_indent, _, _)) = self.ordered_info(idx) else {
            return;
        };
        let deeper = |this: &Self, i: usize| -> bool {
            let body = this.line_string(i);
            !blank_tail(&body) && body.len() - body.trim_start().len() > want_indent
        };
        let mut first = idx;
        let mut j = idx;
        while j > 0 {
            j -= 1;
            match self.ordered_info(j) {
                Some((indent, _, _)) if indent == want_indent => first = j,
                _ if deeper(self, j) => {}
                _ => break,
            }
        }
        let Some((_, base, _)) = self.ordered_info(first) else {
            return;
        };
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        let mut want = base;
        let mut i = first;
        let total = self.text.len_lines(LineType::LF);
        while i < total {
            match self.ordered_info(i) {
                Some((indent, n, _)) if indent == want_indent => {
                    if n != want {
                        let start = self.line_span(i).start;
                        let body = self.line_string(i);
                        let leading = body.len() - body.trim_start().len();
                        let digits = body[leading..]
                            .bytes()
                            .take_while(|b| b.is_ascii_digit())
                            .count();
                        edits.push((start + leading, start + leading + digits, want.to_string()));
                    }
                    match want.checked_add(1) {
                        Some(w) => want = w,
                        None => break,
                    }
                }
                _ if deeper(self, i) => {}
                _ => break,
            }
            i += 1;
        }
        for (start, end, digits) in edits.iter().rev() {
            self.apply_edit(*start..*end, digits);
        }
    }

    // -- indentation & list conversion (F5) ----------------------------------

    /// Rewrite a block of whole lines as one undo step. `f(i, body)` gets
    /// the block-relative index and the newline-stripped line and returns the
    /// new line plus the splice it applied `(cut_at, removed, added)` so the
    /// caret and anchor can be mapped through it.
    fn rewrite_lines(
        &mut self,
        lines: Range<usize>,
        mut f: impl FnMut(usize, &str) -> (String, usize, usize, usize),
    ) {
        let block_start = self.line_span(lines.start).start;
        let block_end = self.line_span(lines.end - 1).end;
        let old = self.text.slice(block_start..block_end).to_string();
        let ends_nl = old.ends_with('\n');
        let body = old.strip_suffix('\n').unwrap_or(&old);
        let mut out = String::with_capacity(old.len() + 16);
        // (old line start, old line len, new line start, splice)
        let mut spans: Vec<LineSplice> = Vec::new();
        let mut old_at = 0usize;
        for (i, line) in body.split('\n').enumerate() {
            let (new_line, cut, removed, added) = f(i, line);
            spans.push((old_at, line.len(), out.len(), (cut, removed, added)));
            out.push_str(&new_line);
            out.push('\n');
            old_at += line.len() + 1;
        }
        if !ends_nl {
            out.pop();
        }
        if out == old {
            return;
        }
        let map = |p: usize| -> usize {
            if p < block_start {
                return p;
            }
            if p > block_end {
                return p + out.len() - old.len();
            }
            let rel = p - block_start;
            for (os, ol, ns, (cut, removed, added)) in &spans {
                if rel <= os + ol {
                    let col = rel - os;
                    let col = if col < *cut || (col == *cut && *removed > 0) {
                        col
                    } else if col < cut + removed {
                        cut + added
                    } else {
                        col + added - removed
                    };
                    return block_start + ns + col;
                }
            }
            block_start + out.len()
        };
        let (caret, anchor) = (map(self.caret), map(self.anchor));
        self.edit_region(block_start..block_end, &out, caret, anchor);
    }

    /// Spaces one indent level adds to `body`: ordered items nest under
    /// their `1. ` text, everything else by two.
    fn indent_step(body: &str) -> usize {
        match parse_marker(body) {
            Some(Marker {
                kind: MarkerKind::Ordered(..),
                ..
            }) => 3,
            _ => 2,
        }
    }

    /// Lines the indent commands act on: the selected lines, plus — for a
    /// collapsed caret on a list item — the item's nested children.
    fn indent_target(&self) -> Range<usize> {
        let sel = self.selected_lines();
        if self.selection().start != self.selection().end {
            return sel;
        }
        let first = sel.start;
        let head = self.line_string(first);
        let Some(m) = parse_marker(&head) else {
            return sel;
        };
        let total = self.text.len_lines(LineType::LF);
        let mut end = first + 1;
        while end < total {
            let body = self.line_string(end);
            // A trailing phantom line (after the final newline) is not a child.
            if blank_tail(&body) || body.len() - body.trim_start().len() <= m.indent {
                break;
            }
            end += 1;
        }
        first..end
    }

    /// Uncapped nesting depth of every list item from the start of its block
    /// through `lines.end`, as `(line, depth)`, optionally with the target
    /// `lines` indented one level. Mirrors the renderer's rule: an item is
    /// one level below the nearest shallower open item.
    fn item_depths(&self, lines: &Range<usize>, bump: bool) -> Vec<(usize, usize)> {
        let is_item = |body: &str| {
            matches!(
                parse_marker(body).map(|m| m.kind),
                Some(MarkerKind::Bullet(_) | MarkerKind::Task(_) | MarkerKind::Ordered(..))
            )
        };
        // Back up over the list block the lines belong to.
        let mut start = lines.start;
        while start > 0 {
            let body = self.line_string(start - 1);
            let flush = body.len() == body.trim_start().len();
            if !blank_tail(&body) && flush && !is_item(&body) {
                break;
            }
            start -= 1;
        }
        let mut levels: Vec<usize> = Vec::new();
        let mut out = Vec::new();
        for i in start..lines.end {
            let body = self.line_string(i);
            if blank_tail(&body) {
                continue;
            }
            let mut indent = body.len() - body.trim_start().len();
            if bump && i >= lines.start {
                indent += Self::indent_step(&body);
            }
            if !is_item(&body) {
                if indent == 0 {
                    levels.clear();
                }
                continue;
            }
            while levels.last().is_some_and(|&top| top > indent) {
                levels.pop();
            }
            if levels.last().is_none_or(|&top| top < indent) {
                levels.push(indent);
            }
            if i >= lines.start {
                out.push((i, levels.len() - 1));
            }
        }
        out
    }

    /// Whether `Tab` may indent `lines`: the head item must really move one
    /// level deeper (it needs a sibling above to nest under) and nothing may
    /// end up past [`MAX_LIST_DEPTH`]. Non-list lines never block.
    fn indent_fits(&self, lines: &Range<usize>) -> bool {
        let before = self.item_depths(lines, false);
        let after = self.item_depths(lines, true);
        let Some(((_, was), (_, now))) = before.first().zip(after.first()) else {
            return true;
        };
        *now == was + 1 && after.iter().all(|(_, d)| *d <= MAX_LIST_DEPTH)
    }

    /// `Tab`: indent. A collapsed caret on prose/code just inserts a level
    /// at the caret; on a list item the whole item (children included)
    /// nests one level; a selection indents every line it touches. Inside
    /// code fences (`code`) a level is four spaces and lists are ignored.
    pub fn indent(&mut self, code: bool) {
        let collapsed = self.selection().start == self.selection().end;
        let on_item = !code && collapsed && parse_marker(&self.line_body().1).is_some();
        if collapsed && !on_item {
            self.insert(if code { "    " } else { "  " });
            return;
        }
        let lines = if code {
            self.selected_lines()
        } else {
            self.indent_target()
        };
        if !code && !self.indent_fits(&lines) {
            return; // already three levels deep
        }
        let was_ordered: Vec<bool> = (lines.start..lines.end)
            .map(|i| self.ordered_info(i).is_some())
            .collect();
        let above = lines
            .start
            .checked_sub(1)
            .and_then(|i| self.ordered_info(i));
        self.rewrite_lines(lines.clone(), |_, body| {
            if blank_tail(body) {
                return (body.to_string(), 0, 0, 0);
            }
            let n = if code { 4 } else { Self::indent_step(body) };
            (format!("{}{body}", " ".repeat(n)), 0, 0, n)
        });
        if code {
            return;
        }
        // The moved head starts a nested run at 1 unless it follows a
        // sibling at its new indent; then straighten both levels.
        if was_ordered.first() == Some(&true) {
            let head = self.ordered_info(lines.start);
            if let Some((indent, _, _)) = head {
                let continues = matches!(above, Some((a, _, _)) if a == indent);
                if !continues {
                    self.set_ordered_number(lines.start, 1);
                }
                self.renumber_ordered_from(lines.start);
            }
        }
        let after = lines
            .end
            .min(self.text.len_lines(LineType::LF).saturating_sub(1));
        self.renumber_ordered_from(after);
        if let Some(prev) = lines.start.checked_sub(1) {
            self.renumber_ordered_from(prev);
        }
    }

    /// `Shift+Tab`: outdent the target lines by one level (a tab or up to
    /// the line's step in spaces), then straighten any ordered runs.
    pub fn outdent(&mut self, code: bool) {
        let lines = if code {
            self.selected_lines()
        } else {
            self.indent_target()
        };
        let mut changed = false;
        self.rewrite_lines(lines.clone(), |_, body| {
            let step = if code { 4 } else { Self::indent_step(body) };
            let removed = if body.starts_with('\t') {
                1
            } else {
                body.bytes().take(step).take_while(|b| *b == b' ').count()
            };
            changed |= removed > 0;
            (body[removed..].to_string(), 0, removed, 0)
        });
        if code || !changed {
            return;
        }
        for i in [lines.start, lines.end.saturating_sub(1)] {
            if i < self.text.len_lines(LineType::LF) {
                self.renumber_ordered_from(i);
            }
        }
        let after = lines
            .end
            .min(self.text.len_lines(LineType::LF).saturating_sub(1));
        self.renumber_ordered_from(after);
    }

    /// Overwrite the digits of the ordered item on line `idx`.
    fn set_ordered_number(&mut self, idx: usize, n: u32) {
        let body = self.line_string(idx);
        let start = self.line_span(idx).start;
        let leading = body.len() - body.trim_start().len();
        let digits = body[leading..]
            .bytes()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits > 0 {
            self.apply_edit(start + leading..start + leading + digits, &n.to_string());
        }
    }

    // -- links (F9) ------------------------------------------------------------

    /// `Ctrl+K` on a fresh link: a selection becomes `[text](|)` (or
    /// `[|](url)` when the selection is itself a URL); with nothing selected
    /// an empty `[]()` appears with the caret in the label.
    pub fn insert_link(&mut self) {
        let sel = self.selection();
        let at = sel.start;
        if sel.start == sel.end {
            self.edit_region(at..at, "[]()", at + 1, at + 1);
            return;
        }
        let text = self.text.slice(sel.clone()).to_string();
        if looks_like_url(&text) {
            let caret = at + 1;
            self.edit_region(sel, &format!("[]({text})"), caret, caret);
        } else {
            let caret = at + 1 + text.len() + 2;
            self.edit_region(sel, &format!("[{text}]()"), caret, caret);
        }
    }

    /// Select a byte range (anchor at `start`, caret at `end`).
    pub fn select_range(&mut self, range: Range<usize>) {
        self.anchor = range.start.min(self.text.len());
        self.caret = range.end.min(self.text.len());
        self.coalescing = false;
    }

    /// Paste `text`. A URL pasted over a non-URL selection turns the
    /// selection into `[selection](url)`; anything else replaces as usual.
    pub fn paste(&mut self, text: &str) {
        let sel = self.selection();
        let url = text.trim();
        if sel.start != sel.end && looks_like_url(url) {
            let inner = self.text.slice(sel.clone()).to_string();
            if !looks_like_url(&inner) && !inner.contains('\n') {
                let insert = format!("[{inner}]({url})");
                let end = sel.start + insert.len();
                self.edit_region(sel, &insert, end, end);
                return;
            }
        }
        self.insert(text);
    }

    // -- auto-pairing (F6) ---------------------------------------------------

    fn ch_before(&self, at: usize) -> Option<char> {
        (at > 0).then(|| {
            let p = self.prev_boundary(at);
            self.text
                .slice(p..at)
                .to_string()
                .chars()
                .next()
                .unwrap_or(' ')
        })
    }

    fn ch_after(&self, at: usize) -> Option<char> {
        (at < self.text.len()).then(|| {
            let n = self.next_boundary(at);
            self.text
                .slice(at..n)
                .to_string()
                .chars()
                .next()
                .unwrap_or(' ')
        })
    }

    /// Insert `open + close` at the caret with the caret between them.
    fn insert_pair(&mut self, open: &str, close: &str) {
        let at = self.caret;
        let text = format!("{open}{close}");
        self.edit_region(at..at, &text, at + open.len(), at + open.len());
    }

    /// Smart typing for one character (IME-free input). Returns `true` when
    /// the character was fully handled — pair inserted, closer skipped over,
    /// or selection wrapped — and `false` when the caller should insert it
    /// as plain text. `code` is true inside a fenced block, where markdown
    /// markers (`* = ~`) never pair.
    pub fn type_char(&mut self, c: char, code: bool) -> bool {
        if !self.auto_close || self.marked.is_some() {
            return false;
        }
        let md_marker = matches!(c, '*' | '_' | '=' | '~') && !code;
        let sel = self.selection();
        if sel.start != sel.end {
            let close = match c {
                '[' => ']',
                '(' => ')',
                '{' => '}',
                '"' | '`' => c,
                '*' | '_' if md_marker => c,
                _ => return false,
            };
            let inner = self.text.slice(sel.clone()).to_string();
            let insert = format!("{c}{inner}{close}");
            let start = sel.start + c.len_utf8();
            let end = start + inner.len();
            self.edit_region(sel, &insert, end, start);
            return true;
        }
        let prev = self.ch_before(self.caret);
        let next = self.ch_after(self.caret);
        let alnum = |o: Option<char>| o.is_some_and(char::is_alphanumeric);
        match c {
            ')' | ']' | '}' if next == Some(c) => {
                self.move_right(false);
                true
            }
            '(' | '[' | '{' if !alnum(next) => {
                let close = match c {
                    '(' => ")",
                    '[' => "]",
                    _ => "}",
                };
                self.insert_pair(&c.to_string(), close);
                true
            }
            '"' | '\'' | '`' => {
                if next == Some(c) && (c != '\'' || !alnum(prev)) {
                    self.move_right(false);
                    true
                } else if prev != Some(c) && !alnum(prev) && !alnum(next) && !(c == '\'' && code) {
                    self.insert_pair(&c.to_string(), &c.to_string());
                    true
                } else {
                    false
                }
            }
            '*' if md_marker => {
                if next == Some('*') {
                    // `*|*` (empty pair) grows to `**|**`; after text the
                    // star in front is a closer, so this one skips over.
                    let before_prev = self.ch_before(self.prev_boundary(self.caret));
                    if prev == Some('*') && !alnum(before_prev) {
                        self.insert_pair("*", "*");
                    } else {
                        self.move_right(false);
                    }
                    return true;
                }
                let (line_start, body) = self.line_body();
                let at_line_start = body[..self.caret - line_start]
                    .chars()
                    .all(char::is_whitespace);
                if !at_line_start && !alnum(prev) && prev != Some('*') && !alnum(next) {
                    self.insert_pair("*", "*");
                    true
                } else {
                    false
                }
            }
            '=' | '~' if md_marker => {
                if next == Some(c) {
                    self.move_right(false);
                    true
                } else if prev == Some(c)
                    && !alnum(next)
                    && self.ch_before(self.prev_boundary(self.caret)) != Some(c)
                {
                    // The first marker char is already in the text.
                    let one = c.to_string();
                    self.insert_pair(&one, &format!("{one}{one}"));
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// True when the caret sits between the halves of an empty auto pair.
    fn inside_empty_pair(&self) -> bool {
        matches!(
            (self.ch_before(self.caret), self.ch_after(self.caret)),
            (Some('('), Some(')'))
                | (Some('['), Some(']'))
                | (Some('{'), Some('}'))
                | (Some('"'), Some('"'))
                | (Some('\''), Some('\''))
                | (Some('`'), Some('`'))
                | (Some('*'), Some('*'))
                | (Some('='), Some('='))
                | (Some('~'), Some('~'))
        )
    }

    /// `Backspace` with indent awareness: a collapsed caret sitting in a
    /// line's leading whitespace removes back to the previous 2-space stop
    /// (or one tab) instead of a single space. Everything else is a normal
    /// backward delete.
    pub fn backspace(&mut self) {
        if self.selection().start == self.selection().end && self.caret > 0 {
            if self.auto_close && self.inside_empty_pair() {
                self.replace(self.caret - 1..self.caret + 1, "", false);
                return;
            }
            let (line_start, body) = self.line_body();
            let col = self.caret - line_start;
            let lead = &body[..col.min(body.len())];
            if !lead.is_empty() && lead.bytes().all(|b| b == b' ' || b == b'\t') {
                let remove = if lead.ends_with('\t') {
                    1
                } else {
                    let spaces = lead.len() - lead.trim_end_matches(' ').len();
                    (spaces - 1) % 2 + 1
                };
                self.replace(self.caret - remove..self.caret, "", false);
                return;
            }
        }
        self.delete_backward();
    }

    /// Convert the selected lines to `kind`, or strip the marker when every
    /// non-blank line already is that kind (a toggle). Blank and quoted
    /// lines are left alone; numbering counts the converted lines from 1.
    pub fn convert_list(&mut self, kind: ListKind) {
        let lines = self.selected_lines();
        let matches_kind = |body: &str| -> bool {
            matches!(
                (parse_marker(body).map(|m| m.kind), kind),
                (Some(MarkerKind::Bullet(_)), ListKind::Bullet)
                    | (Some(MarkerKind::Ordered(..)), ListKind::Ordered)
                    | (Some(MarkerKind::Task(_)), ListKind::Task)
            )
        };
        let bodies: Vec<String> = lines.clone().map(|i| self.line_string(i)).collect();
        let eligible = |b: &String| !blank_tail(b) && !b.trim_start().starts_with('>');
        let strip = bodies
            .iter()
            .filter(|b| eligible(b))
            .all(|b| matches_kind(b))
            && bodies.iter().any(eligible);
        let mut number = 0u32;
        self.rewrite_lines(lines, |_, body| {
            if blank_tail(body) || body.trim_start().starts_with('>') {
                return (body.to_string(), 0, 0, 0);
            }
            let indent = body.len() - body.trim_start().len();
            let old_len = match parse_marker(body) {
                Some(Marker {
                    kind: MarkerKind::Bullet(_) | MarkerKind::Task(_) | MarkerKind::Ordered(..),
                    prefix_len,
                    ..
                }) => prefix_len - indent,
                _ => 0,
            };
            let new_marker = if strip {
                String::new()
            } else {
                match kind {
                    ListKind::Bullet => "- ".to_string(),
                    ListKind::Task => "- [ ] ".to_string(),
                    ListKind::Ordered => {
                        number += 1;
                        format!("{number}. ")
                    }
                }
            };
            let out = format!(
                "{}{new_marker}{}",
                &body[..indent],
                &body[indent + old_len..]
            );
            (out, indent, old_len, new_marker.len())
        });
        let idx = self.caret_line();
        self.renumber_ordered_from(idx);
    }

    /// Flip the task box whose `[ ] ` / `[x] ` starts at byte `at` (a
    /// checkbox click). Caret and selection stay where they are; one undo
    /// step. No-op when the bytes there are not a task box.
    pub fn toggle_task_box(&mut self, at: usize) {
        if at + 3 > self.text.len() {
            return;
        }
        let head = self.text.slice(at..at + 3).to_string();
        let flipped = match head.as_str() {
            "[ ]" => "[x]",
            "[x]" | "[X]" => "[ ]",
            _ => return,
        };
        let (caret, anchor) = (self.caret, self.anchor);
        self.edit_region(at..at + 3, flipped, caret, anchor);
    }

    /// Flip `[ ]`/`[x]` on every selected task line; bullets and plain
    /// lines become unchecked tasks first.
    pub fn toggle_task(&mut self) {
        let lines = self.selected_lines();
        self.rewrite_lines(lines, |_, body| {
            if blank_tail(body) {
                return (body.to_string(), 0, 0, 0);
            }
            let indent = body.len() - body.trim_start().len();
            match parse_marker(body) {
                Some(Marker {
                    kind: MarkerKind::Task(_),
                    ..
                }) => {
                    let at = indent + 3;
                    let flipped = if body.as_bytes()[at] == b' ' {
                        "x"
                    } else {
                        " "
                    };
                    (
                        format!("{}{flipped}{}", &body[..at], &body[at + 1..]),
                        at,
                        1,
                        1,
                    )
                }
                Some(Marker {
                    kind: MarkerKind::Bullet(_) | MarkerKind::Ordered(..),
                    prefix_len,
                    ..
                }) => {
                    let old = prefix_len - indent;
                    let out = format!("{}- [ ] {}", &body[..indent], &body[prefix_len..]);
                    (out, indent, old, 6)
                }
                _ => (
                    format!("{}- [ ] {}", &body[..indent], &body[indent..]),
                    indent,
                    0,
                    6,
                ),
            }
        });
    }

    /// Public renumber hook for edit paths that remove an ordered item
    /// (delete-line etc.): straightens the list around the caret afterwards.
    pub fn renumber_ordered_at_caret(&mut self) {
        let idx = self.caret_line();
        self.renumber_ordered_from(idx);
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
        self.disk_hash = Some(rope_hash(&self.text));
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
    fn hash_matches_across_rope_chunking() {
        let long = "line of text\n".repeat(5000);
        let b = buf(&long);
        assert_eq!(rope_hash(&b.text), hash_text(&long));
        let mut b = buf("x\n");
        b.apply_edit(0..0, "y");
        b.mark_saved(UNIX_EPOCH);
        assert_eq!(b.disk_hash, Some(hash_text("yx\n")));
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

    // -- line editing ------------------------------------------------

    #[test]
    fn move_line_up_swaps_and_rides_the_caret() {
        let mut b = buf("one\ntwo\nthree\n");
        b.move_caret(5, false); // inside "two"
        b.move_line_up();
        assert_eq!(b.text_string(), "two\none\nthree\n");
        assert_eq!(b.caret(), 1, "caret keeps its offset in the moved line");
        assert!(b.dirty);
        // Undo restores both text and caret.
        assert!(b.undo());
        assert_eq!(b.text_string(), "one\ntwo\nthree\n");
        assert_eq!(b.caret(), 5);
    }

    #[test]
    fn move_line_down_swaps_and_rides_the_caret() {
        let mut b = buf("one\ntwo\nthree\n");
        b.move_caret(1, false); // inside "one"
        b.move_line_down();
        assert_eq!(b.text_string(), "two\none\nthree\n");
        assert_eq!(b.caret(), 5);
    }

    #[test]
    fn move_line_up_at_first_line_is_a_noop() {
        let mut b = buf("one\ntwo\n");
        b.move_line_up();
        assert_eq!(b.text_string(), "one\ntwo\n");
        assert!(!b.dirty, "no edit, no dirty");
    }

    #[test]
    fn move_line_down_at_last_line_is_a_noop() {
        let mut b = buf("one\ntwo\n");
        b.move_caret(5, false);
        b.move_line_down();
        assert_eq!(b.text_string(), "one\ntwo\n");
        assert!(!b.dirty);
    }

    #[test]
    fn move_line_up_lifts_last_line_without_newline() {
        let mut b = buf("a\nb");
        b.move_caret(3, false);
        b.move_line_up();
        assert_eq!(b.text_string(), "b\na");
        assert_eq!(b.caret(), 1, "caret stays after 'b'");
    }

    #[test]
    fn move_line_down_over_last_line_keeps_ending() {
        let mut b = buf("a\nb\nc");
        b.move_caret(3, false); // inside "b\n"
        b.move_line_down();
        assert_eq!(b.text_string(), "a\nc\nb", "no trailing newline invented");
        assert_eq!(b.caret(), 5);
    }

    #[test]
    fn move_line_block_selection_moves_whole_block() {
        let mut b = buf("one\ntwo\nthree\nfour\n");
        // Select "two\nthree\n" (bytes 4..14).
        b.move_caret(4, false);
        b.move_caret(14, true);
        b.move_line_up();
        assert_eq!(b.text_string(), "two\nthree\none\nfour\n");
        assert_eq!(b.anchor(), 0, "selection rides the block, start corner");
        assert_eq!(b.caret(), 10);
    }

    #[test]
    fn duplicate_line_copies_below_and_moves_caret() {
        let mut b = buf("one\ntwo\nthree\n");
        b.move_caret(6, false); // inside "two"
        b.duplicate_line();
        assert_eq!(b.text_string(), "one\ntwo\ntwo\nthree\n");
        assert_eq!(b.caret(), 10, "caret lands on the copy");
    }

    #[test]
    fn duplicate_last_line_adds_the_newline() {
        let mut b = buf("a\nb");
        b.move_caret(3, false);
        b.duplicate_line();
        assert_eq!(b.text_string(), "a\nb\nb");
        assert_eq!(b.caret(), 5, "end of the original becomes end of the copy");
    }

    #[test]
    fn delete_line_removes_content_and_newline() {
        let mut b = buf("one\ntwo\nthree\n");
        b.move_caret(6, false);
        b.delete_line();
        assert_eq!(b.text_string(), "one\nthree\n");
        assert_eq!(b.caret(), 4);
    }

    #[test]
    fn delete_last_line_leaves_prior_newline() {
        let mut b = buf("a\nb");
        b.move_caret(3, false);
        b.delete_line();
        assert_eq!(b.text_string(), "a\n");
    }

    #[test]
    fn delete_selected_lines_removes_the_block() {
        let mut b = buf("one\ntwo\nthree\nfour\n");
        b.move_caret(4, false);
        b.move_caret(14, true);
        b.delete_line();
        assert_eq!(b.text_string(), "one\nfour\n");
    }

    #[test]
    fn insert_line_below_after_caret_line() {
        let mut b = buf("one\ntwo\n");
        b.move_caret(1, false); // inside "one"
        b.insert_line_below();
        assert_eq!(b.text_string(), "one\n\ntwo\n");
        assert_eq!(b.caret(), 4, "caret lands on the new empty line");
    }

    #[test]
    fn insert_line_below_at_end_of_text() {
        let mut b = buf("a\nb");
        b.move_caret(3, false);
        b.insert_line_below();
        assert_eq!(b.text_string(), "a\nb\n");
        assert_eq!(b.caret(), 3);
    }

    #[test]
    fn insert_line_above_before_caret_line() {
        let mut b = buf("one\ntwo\n");
        b.move_caret(1, false);
        b.insert_line_above();
        assert_eq!(b.text_string(), "\none\ntwo\n");
        assert_eq!(b.caret(), 0);
    }

    #[test]
    fn join_lines_merges_with_one_space() {
        let mut b = buf("one\ntwo\nthree\n");
        b.join_lines();
        assert_eq!(b.text_string(), "one two\nthree\n");
        assert_eq!(b.caret(), 4, "caret sits at the seam");
    }

    #[test]
    fn join_lines_drops_next_indent_and_keeps_trailing_space() {
        let mut b = buf("one \n  two\n");
        b.join_lines();
        assert_eq!(b.text_string(), "one two\n", "no double space at the seam");
        // And the tight variant gains exactly one space.
        let mut b = buf("one\n  two\n");
        b.join_lines();
        assert_eq!(b.text_string(), "one two\n");
    }

    #[test]
    fn join_lines_with_empty_neighbour_is_bare() {
        let mut b = buf("a\n\nb\n");
        b.join_lines();
        assert_eq!(b.text_string(), "a\nb\n", "empty next line just disappears");
    }

    #[test]
    fn sort_selected_lines_orders_the_block() {
        let mut b = buf("pear\napple\nfig\n");
        b.move_caret(0, false);
        b.move_caret(14, true);
        b.sort_selected_lines();
        assert_eq!(b.text_string(), "apple\nfig\npear\n");
    }

    #[test]
    fn sort_without_selection_is_a_noop() {
        let mut b = buf("pear\napple\n");
        b.sort_selected_lines();
        assert_eq!(b.text_string(), "pear\napple\n");
        assert!(!b.dirty);
    }

    #[test]
    fn transform_case_upper_lower_toggle() {
        let mut b = buf("hello World\n");
        b.move_caret(0, false);
        b.move_caret(11, true);
        b.transform_case(CaseKind::Upper);
        assert_eq!(b.text_string(), "HELLO WORLD\n");
        assert_eq!(b.selection(), 0..11, "selection survives the transform");
        b.transform_case(CaseKind::Toggle);
        assert_eq!(b.text_string(), "hello world\n");
        b.move_caret(0, false);
        b.move_caret(11, true);
        b.transform_case(CaseKind::Title);
        assert_eq!(b.text_string(), "Hello World\n");
        b.transform_case(CaseKind::Lower);
        assert_eq!(b.text_string(), "hello world\n");
    }

    #[test]
    fn transform_case_collapsed_targets_the_word() {
        let mut b = buf("one Two three");
        b.move_caret(6, false); // inside "Two"
        b.transform_case(CaseKind::Upper);
        assert_eq!(b.text_string(), "one TWO three");
        assert_eq!(b.selection(), 4..7);
    }

    #[test]
    fn toggle_line_comment_adds_then_removes() {
        let mut b = buf("let a = 1;\nlet b = 2;\n");
        b.select_all();
        b.toggle_line_comment("//");
        assert_eq!(b.text_string(), "// let a = 1;\n// let b = 2;\n");
        b.toggle_line_comment("//");
        assert_eq!(b.text_string(), "let a = 1;\nlet b = 2;\n");
    }

    #[test]
    fn toggle_line_comment_only_touches_selected_lines() {
        let mut b = buf("a\nb\nc\n");
        b.move_caret(2, false);
        b.move_caret(4, true); // selects "b\n"
        b.toggle_line_comment("//");
        assert_eq!(b.text_string(), "a\n// b\nc\n");
    }

    #[test]
    fn toggle_line_comment_respects_indent_and_blank_lines() {
        let mut b = buf("  indented\n\nplain\n");
        b.select_all();
        b.toggle_line_comment("#");
        assert_eq!(
            b.text_string(),
            "  # indented\n\n# plain\n",
            "blank lines stay blank, indent kept"
        );
        b.toggle_line_comment("#");
        assert_eq!(b.text_string(), "  indented\n\nplain\n");
    }

    #[test]
    fn toggled_comment_survives_undo_as_one_step() {
        let mut b = buf("x\n");
        b.toggle_line_comment("//");
        assert_eq!(b.text_string(), "// x\n");
        assert!(b.undo());
        assert_eq!(b.text_string(), "x\n");
    }

    // -- enter continuation (F4) -------------------------------------

    #[test]
    fn enter_continues_a_bullet() {
        let mut b = buf("- wash dishes\n");
        b.move_caret(6, false); // after "wash", before the space
        b.smart_enter();
        assert_eq!(b.text_string(), "- wash\n- dishes\n");
        assert_eq!(b.caret(), 9, "caret sits after the new marker");
        // Suffix content stays on the old line.
        let mut b = buf("- done\n");
        b.move_caret(6, false);
        b.smart_enter();
        assert_eq!(b.text_string(), "- done\n- \n");
        assert_eq!(b.caret(), 9);
    }

    #[test]
    fn enter_on_an_empty_bullet_exits_the_list() {
        let mut b = buf("above\n- \nbelow\n");
        b.move_caret(7, false); // after "- "
        b.smart_enter();
        assert_eq!(b.text_string(), "above\n\nbelow\n");
        assert_eq!(b.caret(), 6, "back to the line start, marker gone");
    }

    #[test]
    fn enter_continues_numbered_lists_and_renumbers() {
        let mut b = buf("1. one\n2. two\n3. three\n4. four\n");
        // Split item 2 in half: "2. tw|o" (byte 12 = after "tw").
        b.move_caret(12, false);
        b.smart_enter();
        assert_eq!(
            b.text_string(),
            "1. one\n2. tw\n3. o\n4. three\n5. four\n",
            "inserted item numbers up; the rest straightens"
        );
        assert_eq!(b.caret(), 16, "caret after the `3. ` marker");
        // Removing an item closes the gap too (via delete_line).
        let mut b = buf("1. a\n2. b\n3. c\n");
        b.move_caret(7, false);
        b.delete_line();
        b.renumber_ordered_at_caret();
        assert_eq!(b.text_string(), "1. a\n2. c\n");
    }

    #[test]
    fn ordered_lists_keep_the_first_number_and_separator() {
        let mut b = buf("3) alpha\n4) beta\n");
        b.move_caret(7, false); // splits "alph|a"
        b.smart_enter();
        assert_eq!(
            b.text_string(),
            "3) alph\n4) a\n5) beta\n",
            "base 3 is preserved; `)` sep kept"
        );
        assert_eq!(b.caret(), 11);
    }

    #[test]
    fn enter_continues_tasks_unchecked() {
        let mut b = buf("- [x] water the plants\n");
        b.move_caret(9, false); // inside "water" (after "wat")
        b.smart_enter();
        assert_eq!(
            b.text_string(),
            "- [x] wat\n- [ ] er the plants\n",
            "new task is always unchecked"
        );
        assert_eq!(b.caret(), 16);
        // From the end: a ready-to-type unchecked item.
        let mut b = buf("- [x] done\n");
        b.move_caret(10, false);
        b.smart_enter();
        assert_eq!(b.text_string(), "- [x] done\n- [ ] \n");
        assert_eq!(b.caret(), 17);
        // Exiting after the fresh empty item.
        b.smart_enter();
        assert_eq!(b.text_string(), "- [x] done\n\n");
    }

    #[test]
    fn enter_after_heading_and_prose_is_plain() {
        let mut b = buf("# Title\nnext");
        b.move_caret(7, false);
        b.smart_enter();
        assert_eq!(b.text_string(), "# Title\n\nnext", "no marker invented");
        let mut b = buf("plain text\n");
        b.move_caret(6, false); // after "plain " — prose keeps the space
        b.smart_enter();
        assert_eq!(b.text_string(), "plain \ntext\n");
    }

    #[test]
    fn enter_inside_indent_continues_the_indent() {
        let mut b = buf("  - nested item\n");
        b.move_caret(10, false); // after "nested", before its space
        b.smart_enter();
        assert_eq!(b.text_string(), "  - nested\n  - item\n");
        assert_eq!(b.caret(), 15);
    }

    #[test]
    fn enter_in_nested_quote_list_continues_both() {
        let mut b = buf("> - quoted bullet\nzhi\n");
        b.move_caret(10, false); // after "quoted"
        b.smart_enter();
        assert_eq!(b.text_string(), "> - quoted\n> - bullet\nzhi\n");
        assert_eq!(b.caret(), 15);
    }

    #[test]
    fn enter_replaces_a_selection_like_typing() {
        let mut b = buf("- one two\n");
        b.move_caret(2, false);
        b.move_caret(5, true); // "one"
        b.smart_enter();
        assert_eq!(b.text_string(), "- \n two\n", "insert uses the plain path");
    }

    #[test]
    fn empty_bullet_exit_survives_undo_as_one_step() {
        let mut b = buf("- \n");
        b.move_caret(2, false);
        b.smart_enter();
        assert_eq!(b.text_string(), "\n");
        assert!(b.undo());
        assert_eq!(b.text_string(), "- \n");
        assert_eq!(b.caret(), 2, "caret restored");
    }

    #[test]
    fn soft_break_keeps_item_columns_and_quotes() {
        let mut b = buf("- item\n");
        b.move_caret(4, false); // "it|em"
        b.soft_break();
        assert_eq!(
            b.text_string(),
            "- it\n  em\n",
            "indented to the content col"
        );
        let mut b = buf("> quote\n");
        b.move_caret(7, false); // end of "quote"
        b.soft_break();
        assert_eq!(b.text_string(), "> quote\n> \n", "quote prefix repeats");
        let mut b = buf("1. one\n");
        b.move_caret(6, false);
        b.soft_break();
        assert_eq!(b.text_string(), "1. one\n   \n", "no number invented");
    }

    #[test]
    fn enter_continues_quote_prefixes_not_callout_tags() {
        let mut b = buf("> a wise note\n");
        b.move_caret(8, false); // after "wise", before the space (stripped below)
        b.smart_enter();
        assert_eq!(b.text_string(), "> a wise\n> note\n", "quote stays alive");
        assert_eq!(b.caret(), 11);
        // "> " alone exits the quote.
        let mut b = buf("> \nbody\n");
        b.move_caret(2, false);
        b.smart_enter();
        assert_eq!(b.text_string(), "\nbody\n");
        // Callout header: the tag is never duplicated.
        let mut b = buf("> [!NOTE] heads up\nmore\n");
        b.move_caret(18, false);
        b.smart_enter();
        assert_eq!(
            b.text_string(),
            "> [!NOTE] heads up\n> \nmore\n",
            "only the `>` prefix continues"
        );
        assert_eq!(b.caret(), 21);
    }

    // -- F5: indentation & lists -------------------------------------------

    #[test]
    fn tab_nests_an_item_with_its_children() {
        let mut b = buf("- a\n- b\n  - c\n- d\n");
        b.move_caret(6, false); // inside "b"
        b.indent(false);
        assert_eq!(b.text_string(), "- a\n  - b\n    - c\n- d\n");
        assert_eq!(b.caret(), 8, "caret rides the shifted text");
        b.undo();
        assert_eq!(b.text_string(), "- a\n- b\n  - c\n- d\n", "one undo step");
    }

    #[test]
    fn tab_on_prose_inserts_a_level_and_selection_indents_lines() {
        let mut b = buf("plain\n");
        b.move_caret(5, false);
        b.indent(false);
        assert_eq!(b.text_string(), "plain  \n");
        let mut b = buf("a\nb\nc\n");
        b.move_caret(0, false);
        b.move_caret(3, true); // "a\nb"
        b.indent(false);
        assert_eq!(b.text_string(), "  a\n  b\nc\n");
        assert_eq!(b.selection(), 2..7, "selection keeps its text");
    }

    #[test]
    fn tab_in_code_uses_four_spaces() {
        let mut b = buf("x\ny\n");
        b.move_caret(0, false);
        b.move_caret(3, true);
        b.indent(true);
        assert_eq!(b.text_string(), "    x\n    y\n");
        b.outdent(true);
        assert_eq!(b.text_string(), "x\ny\n");
    }

    #[test]
    fn shift_tab_outdents_and_renumbers() {
        let mut b = buf("1. a\n   1. b\n   2. c\n2. d\n");
        b.move_caret(11, false); // inside "b"
        b.outdent(false);
        assert_eq!(b.text_string(), "1. a\n2. b\n   2. c\n3. d\n");
        let mut b = buf("  - a\n");
        b.move_caret(4, false);
        b.outdent(false);
        assert_eq!(b.text_string(), "- a\n");
        b.outdent(false);
        assert_eq!(b.text_string(), "- a\n", "already flush: no-op");
    }

    #[test]
    fn indenting_an_ordered_item_restarts_the_nested_run() {
        let mut b = buf("1. a\n2. b\n3. c\n");
        b.move_caret(8, false); // inside "b"
        b.indent(false);
        assert_eq!(b.text_string(), "1. a\n   1. b\n2. c\n");
        let mut b = buf("1. a\n   1. x\n2. b\n");
        b.move_caret(15, false);
        b.indent(false);
        assert_eq!(
            b.text_string(),
            "1. a\n   1. x\n   2. b\n",
            "joins the sibling run"
        );
    }

    #[test]
    fn backspace_removes_one_indent_level() {
        let mut b = buf("    text\n");
        b.move_caret(4, false);
        b.backspace();
        assert_eq!(b.text_string(), "  text\n");
        b.backspace();
        assert_eq!(b.text_string(), "text\n");
        b.backspace();
        assert_eq!(b.text_string(), "text\n", "document start: nothing to join");
        let mut b = buf("   x\n");
        b.move_caret(3, false);
        b.backspace();
        assert_eq!(b.text_string(), "  x\n", "odd indent snaps to the stop");
        let mut b = buf("ab\n");
        b.move_caret(2, false);
        b.backspace();
        assert_eq!(b.text_string(), "a\n", "mid-text backspace is unchanged");
        let mut b = buf("\tx\n");
        b.move_caret(1, false);
        b.backspace();
        assert_eq!(b.text_string(), "x\n");
    }

    #[test]
    fn convert_lines_to_lists_and_back() {
        let mut b = buf("one\ntwo\n\nthree\n");
        b.select_all();
        b.convert_list(ListKind::Bullet);
        assert_eq!(b.text_string(), "- one\n- two\n\n- three\n");
        b.convert_list(ListKind::Bullet);
        assert_eq!(
            b.text_string(),
            "one\ntwo\n\nthree\n",
            "same kind toggles off"
        );
        b.select_all();
        b.convert_list(ListKind::Ordered);
        assert_eq!(b.text_string(), "1. one\n2. two\n\n3. three\n");
        b.convert_list(ListKind::Task);
        assert_eq!(
            b.text_string(),
            "- [ ] one\n- [ ] two\n\n- [ ] three\n",
            "kinds swap"
        );
    }

    #[test]
    fn convert_keeps_indent_and_skips_quotes() {
        let mut b = buf("  item\n> quote\n");
        b.select_all();
        b.convert_list(ListKind::Bullet);
        assert_eq!(b.text_string(), "  - item\n> quote\n");
    }

    #[test]
    fn toggle_task_flips_and_promotes() {
        let mut b = buf("- [ ] a\n- [x] b\n- c\nd\n");
        b.select_all();
        b.toggle_task();
        assert_eq!(b.text_string(), "- [x] a\n- [ ] b\n- [ ] c\n- [ ] d\n");
        let mut b = buf("- [ ] a\n");
        b.move_caret(7, false);
        b.toggle_task();
        assert_eq!(b.text_string(), "- [x] a\n");
        assert_eq!(b.caret(), 7);
    }

    #[test]
    fn renumber_steps_over_nested_children() {
        let mut b = buf("1. a\n   - n\n3. c\n");
        b.move_caret(0, false);
        b.renumber_ordered_at_caret();
        assert_eq!(b.text_string(), "1. a\n   - n\n2. c\n");
    }

    // -- F6: auto-pairing -----------------------------------------------------

    fn typed(start: &str, caret: usize, chars: &str) -> OpenBuffer {
        let mut b = buf(start);
        b.move_caret(caret, false);
        for c in chars.chars() {
            if !b.type_char(c, false) {
                b.insert(&c.to_string());
            }
        }
        b
    }

    #[test]
    fn brackets_pair_and_skip_over() {
        let b = typed("", 0, "(a)");
        assert_eq!(b.text_string(), "(a)");
        assert_eq!(b.caret(), 3, "typing the closer skips the auto one");
        let b = typed("", 0, "[[");
        assert_eq!(b.text_string(), "[[]]", "wikilink brackets come free");
        assert_eq!(b.caret(), 2);
        let b = typed("word", 0, "(");
        assert_eq!(b.text_string(), "(word", "no pair in front of a word");
    }

    #[test]
    fn quotes_respect_apostrophes_and_words() {
        let b = typed("", 0, "\"hi\"");
        assert_eq!(b.text_string(), "\"hi\"");
        assert_eq!(b.caret(), 4);
        let b = typed("", 0, "don't");
        assert_eq!(
            b.text_string(),
            "don't",
            "apostrophe after a letter stays single"
        );
    }

    #[test]
    fn markdown_markers_pair_in_prose() {
        let b = typed("a ", 2, "*");
        assert_eq!(b.text_string(), "a **");
        assert_eq!(b.caret(), 3);
        let b = typed("a ", 2, "**bold**");
        assert_eq!(b.text_string(), "a **bold**");
        assert_eq!(b.caret(), 10, "second star expands, closers skip");
        let b = typed("", 0, "*");
        assert_eq!(
            b.text_string(),
            "*",
            "line-start star is a bullet, not a pair"
        );
        let b = typed("2", 1, "*");
        assert_eq!(b.text_string(), "2*", "no pair after an alphanumeric");
        let b = typed("a ", 2, "==hi==");
        assert_eq!(b.text_string(), "a ==hi==");
        assert_eq!(b.caret(), 8);
        let b = typed("a ", 2, "~~x~~");
        assert_eq!(b.text_string(), "a ~~x~~");
    }

    #[test]
    fn backticks_make_fences_not_noise() {
        let b = typed("", 0, "`code`");
        assert_eq!(b.text_string(), "`code`");
        assert_eq!(b.caret(), 6);
        let b = typed("", 0, "```");
        assert_eq!(b.text_string(), "```", "three ticks is a fence opener");
    }

    #[test]
    fn selection_wraps_instead_of_replacing() {
        let mut b = buf("make bold\n");
        b.move_caret(5, false);
        b.move_caret(9, true);
        assert!(b.type_char('*', false));
        assert_eq!(b.text_string(), "make *bold*\n");
        assert_eq!(
            b.selected_text(),
            "bold",
            "selection stays on the inner text"
        );
        assert!(b.type_char('(', false));
        assert_eq!(b.text_string(), "make *(bold)*\n");
        b.undo();
        assert_eq!(
            b.text_string(),
            "make *bold*\n",
            "each wrap is one undo step"
        );
        let mut b = buf("x\n");
        b.select_all();
        assert!(!b.type_char('a', false), "letters replace as usual");
    }

    #[test]
    fn code_fences_skip_markdown_markers() {
        let mut b = buf("a ");
        b.move_caret(2, false);
        assert!(!b.type_char('*', true));
        assert!(b.type_char('(', true));
    }

    #[test]
    fn backspace_deletes_an_empty_pair() {
        let mut b = typed("", 0, "(");
        b.backspace();
        assert_eq!(b.text_string(), "");
        let mut b = typed("a ", 2, "**");
        assert_eq!(b.text_string(), "a ****");
        b.backspace();
        assert_eq!(
            b.text_string(),
            "a **",
            "one pair of stars goes, one remains"
        );
        let mut b = typed("", 0, "(x");
        b.backspace();
        assert_eq!(b.text_string(), "()", "non-empty pair: normal delete");
    }

    #[test]
    fn auto_close_can_be_turned_off() {
        let mut b = buf("");
        b.auto_close = false;
        assert!(!b.type_char('(', false));
        b.insert("(");
        b.insert(")");
        assert_eq!(b.text_string(), "()");
        b.move_caret(1, false);
        b.backspace();
        assert_eq!(b.text_string(), ")", "plain backspace when disabled");
    }

    // -- F9: links ---------------------------------------------------------------

    #[test]
    fn ctrl_k_builds_links() {
        let mut b = buf("see docs now\n");
        b.select_range(4..8);
        b.insert_link();
        assert_eq!(b.text_string(), "see [docs]() now\n");
        assert_eq!(b.caret(), 11, "caret waits in the url slot");
        let mut b = buf("https://a.dev\n");
        b.select_range(0..13);
        b.insert_link();
        assert_eq!(b.text_string(), "[](https://a.dev)\n");
        assert_eq!(b.caret(), 1, "url selection: label first");
        let mut b = buf("x\n");
        b.move_caret(1, false);
        b.insert_link();
        assert_eq!(b.text_string(), "x[]()\n");
        assert_eq!(b.caret(), 2);
        b.undo();
        assert_eq!(b.text_string(), "x\n");
    }

    #[test]
    fn pasting_a_url_over_a_selection_links_it() {
        let mut b = buf("read the docs\n");
        b.select_range(9..13);
        b.paste("  https://a.dev/x \n");
        assert_eq!(b.text_string(), "read the [docs](https://a.dev/x)\n");
        let mut b = buf("plain\n");
        b.move_caret(5, false);
        b.paste("https://a.dev");
        assert_eq!(
            b.text_string(),
            "plainhttps://a.dev\n",
            "lone URL stays bare"
        );
        let mut b = buf("two words\n");
        b.select_range(0..9);
        b.paste("not a url");
        assert_eq!(b.text_string(), "not a url\n");
        let mut b = buf("https://old.dev\n");
        b.select_range(0..15);
        b.paste("https://new.dev");
        assert_eq!(
            b.text_string(),
            "https://new.dev\n",
            "URL over URL replaces"
        );
    }

    #[test]
    fn tab_stops_at_three_levels() {
        let mut b = buf("- a\n  - b\n    - c\n");
        b.move_caret(15, false); // inside "c" (level 3 already)
        b.indent(false);
        assert_eq!(b.text_string(), "- a\n  - b\n    - c\n", "no fourth level");
        // A parent with children is refused when a child would overflow.
        let mut b = buf("- a\n- b\n  - c\n    - d\n");
        b.move_caret(6, false); // inside "b": b->L2, c->L3, d->L4
        b.indent(false);
        assert_eq!(b.text_string(), "- a\n- b\n  - c\n    - d\n");
        // Without the deepest child it nests fine.
        let mut b = buf("- a\n- b\n  - c\n");
        b.move_caret(6, false);
        b.indent(false);
        assert_eq!(b.text_string(), "- a\n  - b\n    - c\n");
        // Tasks and ordered lists obey the same cap.
        let mut b = buf("- [ ] a\n  - [ ] b\n    - [ ] c\n");
        b.move_caret(28, false);
        b.indent(false);
        assert_eq!(b.text_string(), "- [ ] a\n  - [ ] b\n    - [ ] c\n");
        let mut b = buf("1. a\n   1. b\n      1. c\n");
        b.move_caret(20, false);
        b.indent(false);
        assert_eq!(b.text_string(), "1. a\n   1. b\n      1. c\n");
    }

    #[test]
    fn first_item_cannot_nest_but_a_later_sibling_can() {
        let mut b = buf("- a\n- b\n");
        b.move_caret(1, false);
        b.indent(false);
        assert_eq!(b.text_string(), "- a\n- b\n", "nothing above to nest under");
        let mut b = buf("- a\n- b\n");
        b.move_caret(5, false);
        b.indent(false);
        assert_eq!(b.text_string(), "- a\n  - b\n");
        b.outdent(false);
        assert_eq!(b.text_string(), "- a\n- b\n");
    }

    #[test]
    fn checkbox_click_flips_without_moving_the_caret() {
        let mut b = buf("- [ ] a\n- [x] b\n");
        b.move_caret(7, false);
        b.toggle_task_box(2);
        assert_eq!(b.text_string(), "- [x] a\n- [x] b\n");
        assert_eq!(b.caret(), 7);
        b.toggle_task_box(10);
        assert_eq!(b.text_string(), "- [x] a\n- [ ] b\n");
        b.toggle_task_box(0); // not a box
        assert_eq!(b.text_string(), "- [x] a\n- [ ] b\n");
        b.undo();
        assert_eq!(b.text_string(), "- [x] a\n- [x] b\n", "one undo step");
    }
}
