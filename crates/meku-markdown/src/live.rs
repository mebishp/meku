//! Live-preview model for the single-surface editor. Pure text logic, no GPUI.
//!
//! The document is one [`Row`] per source line. Each row can be projected to
//! display text in two modes:
//!
//! - **reveal** (the caret's row): every source byte is shown, markup dimmed.
//! - **hidden** (every other row): markup disappears, headings and quotes keep
//!   only their content, list bullets become `•`, task boxes become `☐`/`☑`.
//!
//! [`Display`] keeps an exact source <-> display mapping in both modes, so the
//! caret and pointer land on the same character the user sees.

use std::ops::Range;
use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::{InlineStyle, parser_options};

/// Structural class of a source line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Blank,
    Text,
    Heading(u8),
    Quote,
    Item,
    /// Fence delimiters and fenced code lines: always shown raw.
    Code,
    /// Pipe-table lines: always shown raw.
    Table,
    /// Thematic break (`---`, `***`, `___`).
    Rule,
}

/// A source line that is exactly one image: `![alt](target)` after optional
/// leading whitespace. Targets are used verbatim (`http(s)`, `data:`, paths).
pub fn standalone_image(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start_matches(' ').trim_end();
    let bang = trimmed.strip_prefix("![")?;
    let alt_end = bang.find("]")?;
    let target = bang[alt_end + 1..].trim_start();
    let target = target.strip_prefix("(")?;
    let target = target.strip_suffix(")")?;
    if target.is_empty() || target.contains("\n") {
        return None;
    }
    Some((&bang[..alt_end], target))
}

/// What a byte range is, beyond its inline style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Plain,
    /// Leading indentation of a line (list nesting shows through it).
    Lead,
    /// Unordered list marker including its trailing space (`- `).
    Bullet,
    /// Ordered list marker including its trailing space (`1. `). Stays visible.
    Ordered,
    /// Task checkbox including its trailing space (`[ ] `, `[x] `).
    Task(bool),
}

/// A contiguous run of source bytes on one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub src: Range<usize>,
    pub style: InlineStyle,
    /// Markup (syntax characters) as opposed to content.
    pub markup: bool,
    pub role: Role,
}

/// One source line and its classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Byte range of the line content, excluding the `\n` terminator.
    pub src: Range<usize>,
    pub kind: RowKind,
    /// Number of `>` markers (0 when not quoted).
    pub quote_depth: u8,
    /// Nesting level of a list item, from its leading indentation (two spaces per level).
    pub list_depth: u8,
    /// Runs covering `src` exactly, in order.
    pub runs: Vec<Run>,
}

/// One visible span of display text and the source it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispRun {
    pub range: Range<usize>,
    pub style: InlineStyle,
    pub markup: bool,
}

/// Maps a source range onto a display range. Hidden markup has an empty
/// `disp` range; replacements (bullets, checkboxes) have a glyph `disp` range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub src: Range<usize>,
    pub disp: Range<usize>,
}

/// A row projected for display in one mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    pub text: String,
    pub runs: Vec<DispRun>,
    pub pieces: Vec<Piece>,
    src_end: usize,
}

/// Inline content spans (text, code, html) with their combined style. Markup
/// is everything else. Sorted by start, non-overlapping.
pub fn content_spans(text: &str) -> Vec<(Range<usize>, InlineStyle)> {
    let mut out = Vec::new();
    let mut stack: Vec<u8> = Vec::new();
    for (event, range) in Parser::new_ext(text, parser_options()).into_offset_iter() {
        match event {
            Event::Start(tag) => stack.push(match tag {
                Tag::Strong => InlineStyle::STRONG,
                Tag::Emphasis => InlineStyle::EMPHASIS,
                Tag::Strikethrough => InlineStyle::STRIKE,
                Tag::Link { .. } => InlineStyle::LINK,
                _ => 0,
            }),
            // pulldown-cmark emits balanced Start/End events.
            Event::End(
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link,
            )
            | Event::End(_) => {
                stack.pop();
            }
            Event::Text(_) | Event::Html(_) | Event::InlineHtml(_) | Event::Code(_) => {
                if range.is_empty() {
                    continue;
                }
                let bits = stack.iter().fold(0u8, |acc, b| acc | b);
                let mut style = style_from_bits(bits);
                if matches!(event, Event::Code(_)) {
                    style = style.with(InlineStyle::CODE);
                }
                out.push((range, style));
            }
            _ => {}
        }
    }
    out
}

fn style_from_bits(bits: u8) -> InlineStyle {
    [
        InlineStyle::STRONG,
        InlineStyle::EMPHASIS,
        InlineStyle::STRIKE,
        InlineStyle::CODE,
        InlineStyle::LINK,
    ]
    .into_iter()
    .filter(|flag| bits & flag != 0)
    .fold(InlineStyle::empty(), |style, flag| style.with(flag))
}

/// An inline link found on one source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRef {
    /// Byte range of the whole `[text](dest)` in the line.
    pub range: Range<usize>,
    pub target: String,
}

/// The inline link of `line` covering byte `col`, if any. Reference-style
/// links with no inline destination are not resolvable here and yield `None`.
pub fn link_at(line: &str, col: usize) -> Option<LinkRef> {
    Parser::new_ext(line, parser_options())
        .into_offset_iter()
        .find_map(|(event, range)| match event {
            Event::Start(Tag::Link { dest_url, .. })
                if range.start <= col && col < range.end && !dest_url.is_empty() =>
            {
                Some(LinkRef {
                    range,
                    target: dest_url.to_string(),
                })
            }
            _ => None,
        })
}

/// What a link destination points at, resolved against the note holding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// `http(s)://`, `mailto:` — handed to the system.
    External(String),
    /// `#heading` inside the current note.
    Anchor(String),
    /// A path relative to the mekuto root (never escapes it), with an
    /// optional `#heading`.
    Path {
        rel: PathBuf,
        anchor: Option<String>,
    },
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Classify `dest` as written in the note at `note_rel` (root-relative).
/// `None` for empty destinations and paths that climb out of the root.
pub fn classify_link(dest: &str, note_rel: &Path) -> Option<LinkTarget> {
    let dest = dest.trim();
    if dest.is_empty() {
        return None;
    }
    let lower = dest.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        return Some(LinkTarget::External(dest.to_string()));
    }
    if let Some(anchor) = dest.strip_prefix('#') {
        return Some(LinkTarget::Anchor(percent_decode(anchor)));
    }
    if lower.contains("://") {
        return None; // unknown scheme: never guess
    }
    let (path, anchor) = match dest.split_once('#') {
        Some((p, a)) => (p, Some(percent_decode(a))),
        None => (dest, None),
    };
    let path = percent_decode(path);
    let mut parts: Vec<std::ffi::OsString> = if path.starts_with('/') {
        Vec::new()
    } else {
        note_rel
            .parent()
            .map(|d| d.iter().map(|c| c.to_os_string()).collect())
            .unwrap_or_default()
    };
    for comp in path.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            c => parts.push(c.into()),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(LinkTarget::Path {
        rel: parts.iter().collect(),
        anchor,
    })
}

/// GitHub-style heading slug: lowercase, spaces to `-`, punctuation dropped.
pub fn heading_slug(title: &str) -> String {
    title
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// Title of the first level-1 heading outside code fences, trimmed and
/// without closing `#`s. `None` when the note has no H1 or it is empty.
pub fn first_h1(text: &str) -> Option<String> {
    build_rows(text).iter().find_map(|row| {
        if row.kind != RowKind::Heading(1) {
            return None;
        }
        let line = text[row.src.clone()].trim_start();
        let title = line.trim_start_matches('#').trim();
        let title = title.trim_end_matches('#').trim_end();
        (!title.is_empty()).then(|| title.to_string())
    })
}

/// A file stem for `title`: path separators and characters Windows/Linux
/// tools dislike are dropped, edges trimmed, length capped. `None` when
/// nothing usable is left.
pub fn title_to_stem(title: &str) -> Option<String> {
    let cleaned: String = title
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned: String = cleaned.chars().take(100).collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

/// Build one row per `\n`-separated line. An empty text yields one blank row.
pub fn build_rows(text: &str) -> Vec<Row> {
    let spans = content_spans(text);
    let mut rows = Vec::new();
    let mut in_fence = false;
    let mut offset = 0usize;
    for line in text.split('\n') {
        let start = offset;
        let end = start + line.len();
        offset = end + 1;
        rows.push(classify(text, start..end, &mut in_fence, &spans));
    }
    rows
}

fn classify(
    text: &str,
    src: Range<usize>,
    in_fence: &mut bool,
    spans: &[(Range<usize>, InlineStyle)],
) -> Row {
    let line = &text[src.clone()];
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return Row {
            src,
            kind: RowKind::Blank,
            quote_depth: 0,
            list_depth: 0,
            runs: Vec::new(),
        };
    }
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        *in_fence = !*in_fence;
        return raw_row(src, RowKind::Code);
    }
    if *in_fence {
        return raw_row(src, RowKind::Code);
    }
    if trimmed.starts_with('|') {
        return raw_row(src, RowKind::Table);
    }
    if is_rule(trimmed) {
        return raw_row(src, RowKind::Rule);
    }

    let lead_len = line.len() - trimmed.len();
    let lead = src.start..src.start + lead_len;
    let mut specials: Vec<(Range<usize>, Role)> = Vec::new();
    if lead_len > 0 {
        specials.push((lead.clone(), Role::Lead));
    }

    // Quote markers first, so `> - item` is a quoted list item.
    let mut p = src.start + lead_len;
    let mut quote_depth = 0u8;
    while text[p..src.end].starts_with('>') {
        quote_depth = quote_depth.saturating_add(1);
        p += 1;
        if text[p..src.end].starts_with(' ') {
            p += 1;
        }
    }

    let rest = &text[p..src.end];
    let hashes = rest.bytes().take_while(|&b| b == b'#').count();
    let heading = ((1..=6).contains(&hashes)
        && (rest.len() == hashes || rest.as_bytes()[hashes] == b' '))
        .then_some(hashes as u8);

    let mut kind = if let Some(level) = heading {
        RowKind::Heading(level)
    } else if quote_depth > 0 {
        RowKind::Quote
    } else {
        RowKind::Text
    };
    let mut list_depth = 0u8;

    let bullet = if heading.is_none() {
        bullet_len(rest)
    } else {
        None
    };
    if let Some((marker_len, ordered)) = bullet {
        kind = RowKind::Item;
        list_depth = (lead_len / 2).min(255) as u8;
        let marker = p..p + marker_len;
        if let Some((task_len, checked)) = task_len(&rest[marker_len..]) {
            // A task item shows its checkbox instead of a bullet; the bullet
            // bytes stay plain markup, so they hide off the caret row.
            let task = marker.end..marker.end + task_len;
            specials.push((task, Role::Task(checked)));
        } else {
            specials.push((marker, if ordered { Role::Ordered } else { Role::Bullet }));
        }
    }

    let runs = make_runs(src.clone(), spans, &specials);
    Row {
        src,
        kind,
        quote_depth,
        list_depth,
        runs,
    }
}

fn raw_row(src: Range<usize>, kind: RowKind) -> Row {
    let style = if kind == RowKind::Code {
        InlineStyle::empty().with(InlineStyle::CODE)
    } else {
        InlineStyle::empty()
    };
    let runs = if src.is_empty() {
        Vec::new()
    } else {
        vec![Run {
            src: src.clone(),
            style,
            markup: false,
            role: Role::Plain,
        }]
    };
    Row {
        src,
        kind,
        quote_depth: 0,
        list_depth: 0,
        runs,
    }
}

fn is_rule(trimmed: &str) -> bool {
    let mut chars = trimmed.chars().filter(|c| !c.is_whitespace());
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    let mut count = 1;
    for c in chars {
        if c != first {
            return false;
        }
        count += 1;
    }
    count >= 3
}

/// Length of a list marker (including its following space) and whether it is ordered.
fn bullet_len(rest: &str) -> Option<(usize, bool)> {
    let bytes = rest.as_bytes();
    match bytes.first()? {
        b'-' | b'*' | b'+' => {
            if rest.len() == 1 {
                Some((1, false))
            } else if bytes[1] == b' ' {
                Some((2, false))
            } else {
                None
            }
        }
        b'0'..=b'9' => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 || digits >= bytes.len() {
                return None;
            }
            if !matches!(bytes[digits], b'.' | b')') {
                return None;
            }
            let after = digits + 1;
            if after == bytes.len() {
                Some((after, true))
            } else if bytes[after] == b' ' {
                Some((after + 1, true))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Length of a task checkbox (including its following space) and its checked state.
fn task_len(after_marker: &str) -> Option<(usize, bool)> {
    let bytes = after_marker.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'[' || bytes[2] != b']' {
        return None;
    }
    let checked = match bytes[1] {
        b' ' => false,
        b'x' | b'X' => true,
        _ => return None,
    };
    match bytes.get(3) {
        None => Some((3, checked)),
        Some(b' ') => Some((4, checked)),
        _ => None,
    }
}

fn span_style_at(spans: &[(Range<usize>, InlineStyle)], at: usize) -> Option<InlineStyle> {
    let idx = spans.partition_point(|(r, _)| r.start <= at);
    let (range, style) = spans.get(idx.checked_sub(1)?)?;
    (at < range.end).then_some(*style)
}

/// Split `row` into runs: special ranges keep their role, content spans carry
/// their style, every other byte is markup.
fn make_runs(
    row: Range<usize>,
    spans: &[(Range<usize>, InlineStyle)],
    specials: &[(Range<usize>, Role)],
) -> Vec<Run> {
    if row.is_empty() {
        return Vec::new();
    }
    let mut cuts = vec![row.start, row.end];
    let first = spans.partition_point(|(r, _)| r.end <= row.start);
    for (r, _) in spans[first..].iter().take_while(|(r, _)| r.start < row.end) {
        cuts.push(r.start.max(row.start));
        cuts.push(r.end.min(row.end));
    }
    for (r, _) in specials {
        cuts.push(r.start);
        cuts.push(r.end);
    }
    cuts.retain(|c| *c >= row.start && *c <= row.end);
    cuts.sort_unstable();
    cuts.dedup();

    let mut runs: Vec<Run> = Vec::new();
    for pair in cuts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a == b {
            continue;
        }
        let run = if let Some((_, role)) = specials.iter().find(|(r, _)| r.start <= a && b <= r.end)
        {
            let markup = !matches!(role, Role::Lead | Role::Plain);
            Run {
                src: a..b,
                style: InlineStyle::empty(),
                markup,
                role: *role,
            }
        } else if let Some(style) = span_style_at(spans, a) {
            Run {
                src: a..b,
                style,
                markup: false,
                role: Role::Plain,
            }
        } else {
            Run {
                src: a..b,
                style: InlineStyle::empty(),
                markup: true,
                role: Role::Plain,
            }
        };
        match runs.last_mut() {
            Some(prev)
                if prev.src.end == run.src.start
                    && prev.style == run.style
                    && prev.markup == run.markup
                    && prev.role == run.role =>
            {
                prev.src.end = run.src.end;
            }
            _ => runs.push(run),
        }
    }
    runs
}

/// Project a row for display. `reveal` shows the source verbatim (caret row).
pub fn project(row: &Row, text: &str, reveal: bool) -> Display {
    let mut out = Display {
        text: String::new(),
        runs: Vec::new(),
        pieces: Vec::new(),
        src_end: row.src.end,
    };
    if row.kind == RowKind::Rule && !reveal {
        return out;
    }
    for run in &row.runs {
        let raw = &text[run.src.clone()];
        let shown: &str = if reveal {
            raw
        } else {
            match run.role {
                Role::Lead => "",
                Role::Bullet => "• ",
                Role::Ordered => raw,
                Role::Task(true) => "☑ ",
                Role::Task(false) => "☐ ",
                Role::Plain if run.markup => "",
                Role::Plain => raw,
            }
        };
        let start = out.text.len();
        out.text.push_str(shown);
        let disp = start..out.text.len();
        out.pieces.push(Piece {
            src: run.src.clone(),
            disp: disp.clone(),
        });
        if !disp.is_empty() {
            out.runs.push(DispRun {
                range: disp,
                style: if shown == raw {
                    run.style
                } else {
                    InlineStyle::empty()
                },
                markup: run.markup || (!reveal && shown != raw),
            });
        }
    }
    out
}

impl Display {
    /// Source byte offset for a display byte offset (clamped to the row).
    pub fn to_src(&self, disp: usize) -> usize {
        for piece in &self.pieces {
            if piece.disp.is_empty() {
                continue;
            }
            if disp < piece.disp.end {
                return if piece.disp.len() == piece.src.len() {
                    piece.src.start + disp.saturating_sub(piece.disp.start)
                } else if disp <= piece.disp.start {
                    piece.src.start
                } else {
                    piece.src.end
                };
            }
        }
        self.src_end
    }

    /// Display byte offset for a source byte offset. Positions inside hidden
    /// markup resolve to where that markup would have started.
    pub fn to_disp(&self, src: usize) -> usize {
        for piece in &self.pieces {
            if src < piece.src.start {
                return piece.disp.start;
            }
            if src < piece.src.end {
                return if piece.disp.len() == piece.src.len() {
                    piece.disp.start + (src - piece.src.start)
                } else {
                    piece.disp.start
                };
            }
        }
        self.text.len()
    }
}

/// Index of the row containing source offset `caret` (the last row starting at or before it).
pub fn row_at(rows: &[Row], caret: usize) -> usize {
    rows.partition_point(|r| r.src.start <= caret)
        .saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<RowKind> {
        build_rows(text).iter().map(|r| r.kind).collect()
    }

    fn shown(text: &str, ix: usize, reveal: bool) -> String {
        let rows = build_rows(text);
        project(&rows[ix], text, reveal).text
    }

    #[test]
    fn empty_text_is_one_blank_row() {
        let rows = build_rows("");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, RowKind::Blank);
        assert_eq!(rows[0].src, 0..0);
    }

    #[test]
    fn one_row_per_line_including_trailing_newline() {
        assert_eq!(build_rows("a\nb\n").len(), 3);
        assert_eq!(
            kinds("# T\n\n- a\n> q\n---\n"),
            vec![
                RowKind::Heading(1),
                RowKind::Blank,
                RowKind::Item,
                RowKind::Quote,
                RowKind::Rule,
                RowKind::Blank,
            ]
        );
    }

    #[test]
    fn heading_marker_hidden_off_caret_shown_on_caret() {
        let text = "## Title";
        assert_eq!(shown(text, 0, false), "Title");
        assert_eq!(shown(text, 0, true), "## Title");
    }

    #[test]
    fn emphasis_markers_hidden_and_content_styled() {
        let text = "a **bold** b";
        assert_eq!(shown(text, 0, false), "a bold b");
        let rows = build_rows(text);
        let d = project(&rows[0], text, false);
        let bold = d
            .runs
            .iter()
            .find(|r| r.style.contains(InlineStyle::STRONG))
            .expect("bold run");
        assert_eq!(&d.text[bold.range.clone()], "bold");
    }

    #[test]
    fn link_shows_text_only() {
        assert_eq!(shown("[site](https://x.dev)", 0, false), "site");
        assert_eq!(
            shown("[site](https://x.dev)", 0, true),
            "[site](https://x.dev)"
        );
    }

    #[test]
    fn bullets_become_glyphs_and_ordered_markers_stay() {
        assert_eq!(shown("- a", 0, false), "• a");
        assert_eq!(shown("* a", 0, false), "• a");
        assert_eq!(shown("1. a", 0, false), "1. a");
        assert_eq!(shown("- a", 0, true), "- a");
    }

    #[test]
    fn task_boxes_become_checkbox_glyphs() {
        assert_eq!(shown("- [ ] todo", 0, false), "☐ todo");
        assert_eq!(shown("- [x] done", 0, false), "☑ done");
        assert_eq!(shown("- [ ] todo", 0, true), "- [ ] todo");
    }

    #[test]
    fn nested_items_record_depth_and_hide_indent_off_caret() {
        let rows = build_rows("- a\n  - b\n");
        assert_eq!(rows[1].list_depth, 1);
        assert_eq!(shown("- a\n  - b\n", 1, false), "• b");
    }

    #[test]
    fn quote_markers_hidden_depth_recorded() {
        let rows = build_rows("> > deep");
        assert_eq!(rows[0].kind, RowKind::Quote);
        assert_eq!(rows[0].quote_depth, 2);
        assert_eq!(shown("> > deep", 0, false), "deep");
    }

    #[test]
    fn fenced_code_and_tables_are_always_raw() {
        let text = "```rs\nlet x = 1;\n```\n| a | b |\n";
        let rows = build_rows(text);
        assert_eq!(rows[0].kind, RowKind::Code);
        assert_eq!(rows[1].kind, RowKind::Code);
        assert_eq!(rows[2].kind, RowKind::Code);
        assert_eq!(rows[3].kind, RowKind::Table);
        assert_eq!(shown(text, 1, false), "let x = 1;");
        assert_eq!(shown(text, 3, false), "| a | b |");
    }

    #[test]
    fn rule_hidden_as_empty_text_and_raw_on_caret() {
        assert_eq!(shown("---", 0, false), "");
        assert_eq!(shown("---", 0, true), "---");
    }

    #[test]
    fn hash_without_space_is_not_a_heading() {
        assert_eq!(kinds("#tag"), vec![RowKind::Text]);
    }

    #[test]
    fn mapping_round_trips_content_in_hidden_mode() {
        let text = "x **bold** y";
        let rows = build_rows(text);
        let d = project(&rows[0], text, false);
        // Display "x bold y": every display offset maps to a source offset
        // that displays back to the same offset.
        for disp in 0..=d.text.len() {
            let src = d.to_src(disp);
            assert_eq!(d.to_disp(src), disp, "disp {disp} via src {src}");
        }
        // Display offset 2 is the `b` of `bold`, which sits at source 4.
        assert_eq!(d.to_src(2), 4);
        assert_eq!(&text[d.to_src(2)..d.to_src(2) + 1], "b");
    }

    #[test]
    fn mapping_in_reveal_is_identity() {
        let text = "## hi";
        let rows = build_rows(text);
        let d = project(&rows[0], text, true);
        for i in 0..=text.len() {
            assert_eq!(d.to_src(i), i);
            assert_eq!(d.to_disp(i), i);
        }
    }

    #[test]
    fn bullet_glyph_maps_back_to_marker_start() {
        let text = "- item";
        let rows = build_rows(text);
        let d = project(&rows[0], text, false);
        assert_eq!(d.to_src(0), 0);
        assert_eq!(d.to_src(d.text.len()), text.len());
        // Caret after the glyph lands on the content.
        assert_eq!(&text[d.to_src("• ".len())..], "item");
    }

    #[test]
    fn row_lookup_finds_row_for_any_caret() {
        let rows = build_rows("ab\ncd\n");
        assert_eq!(row_at(&rows, 0), 0);
        assert_eq!(row_at(&rows, 2), 0); // end of "ab"
        assert_eq!(row_at(&rows, 3), 1); // start of "cd"
        assert_eq!(row_at(&rows, 6), 2); // after final newline
    }

    #[test]
    fn multiline_emphasis_split_per_row() {
        let text = "*a\nb*";
        let rows = build_rows(text);
        let first = project(&rows[0], text, false);
        let second = project(&rows[1], text, false);
        assert_eq!(first.text, "a");
        assert_eq!(second.text, "b");
        assert!(first.runs[0].style.contains(InlineStyle::EMPHASIS));
        assert!(second.runs[0].style.contains(InlineStyle::EMPHASIS));
    }

    #[test]
    fn content_spans_cover_text_not_markup() {
        let text = "a *b* c";
        let spans = content_spans(text);
        let covered: String = spans.iter().map(|(r, _)| &text[r.clone()]).collect();
        assert_eq!(covered, "a b c");
    }

    #[test]
    fn standalone_image_lines_parse() {
        assert_eq!(
            standalone_image("![alt](assets/x.png)"),
            Some(("alt", "assets/x.png"))
        );
        assert_eq!(standalone_image("  ![](y.png)"), Some(("", "y.png")));
        // Not a standalone image, not an image at all, empty target.
        assert_eq!(standalone_image("text ![x](y.png)"), None);
        assert_eq!(standalone_image("[link](x.png)"), None);
        assert_eq!(standalone_image("![x]()"), None);
    }

    #[test]
    fn link_at_finds_the_link_under_a_column() {
        let line = "see [docs](a/b.md#top) now";
        let hit = link_at(line, 6).unwrap();
        assert_eq!(hit.target, "a/b.md#top");
        assert_eq!(&line[hit.range.clone()], "[docs](a/b.md#top)");
        assert!(link_at(line, 1).is_none(), "outside the link");
        assert!(link_at(line, 23).is_none(), "after the link");
        assert_eq!(link_at("<https://x.io>", 3).unwrap().target, "https://x.io");
        assert!(link_at("[[x]]", 2).is_none(), "wikilinks stay literal");
    }

    #[test]
    fn classify_resolves_against_the_note() {
        let note = Path::new("notes/day.md");
        assert_eq!(
            classify_link("https://a.dev/x", note),
            Some(LinkTarget::External("https://a.dev/x".into()))
        );
        assert_eq!(
            classify_link("#My%20Part", note),
            Some(LinkTarget::Anchor("My Part".into()))
        );
        assert_eq!(
            classify_link("../top.md#intro", note),
            Some(LinkTarget::Path {
                rel: PathBuf::from("top.md"),
                anchor: Some("intro".into())
            })
        );
        assert_eq!(
            classify_link("sub/a%20b.md", note),
            Some(LinkTarget::Path {
                rel: PathBuf::from("notes/sub/a b.md"),
                anchor: None
            })
        );
        assert_eq!(
            classify_link("/abs.md", note),
            Some(LinkTarget::Path {
                rel: PathBuf::from("abs.md"),
                anchor: None
            })
        );
        assert_eq!(
            classify_link("../../out.md", note),
            None,
            "cannot leave the root"
        );
        assert_eq!(classify_link("ftp://h/f", note), None);
        assert_eq!(classify_link("  ", note), None);
    }

    #[test]
    fn slugs_follow_github_rules() {
        assert_eq!(heading_slug("My Part: 2!"), "my-part-2");
        assert_eq!(heading_slug("  snake_case-ok "), "snake_case-ok");
    }

    #[test]
    fn first_h1_skips_fences_and_lower_levels() {
        assert_eq!(
            first_h1("## Sub\n# Real ##\n# Later\n").as_deref(),
            Some("Real")
        );
        assert_eq!(
            first_h1("```\n# code\n```\n# After\n").as_deref(),
            Some("After")
        );
        assert_eq!(first_h1("text only\n"), None);
        assert_eq!(first_h1("# \n"), None);
    }

    #[test]
    fn titles_become_safe_stems() {
        assert_eq!(title_to_stem("Plan: Q1/Q2?").as_deref(), Some("Plan Q1Q2"));
        assert_eq!(title_to_stem("  a   b  ").as_deref(), Some("a b"));
        assert_eq!(title_to_stem("..hidden.").as_deref(), Some("hidden"));
        assert_eq!(title_to_stem("///"), None);
    }
}
