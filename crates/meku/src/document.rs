//! The single-surface document editor.
//!
//! The open note is one scrollable list with one row per source line. The
//! caret's row is shown raw (markers dimmed); every other row renders with its
//! markup hidden. Editing happens on the buffer directly, so typing re-renders
//! in place: there is no editor swap and no mode switch.
//!
//! The GPUI text-input handler lives here too, so IME, dead keys and the
//! platform text services reach the same caret as keyboard actions do.

use std::cell::RefCell;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use meku_markdown::InlineStyle;
use meku_markdown::highlight::{Tok, highlight};
use meku_markdown::live::{Display, Row, RowKind, build_rows, project, row_at};

use crate::MekuShell;
use crate::Mode;

/// Per-row syntax spans in line-content byte offsets (`None` = plain).
pub(crate) type RowSpans = Vec<Option<meku_markdown::highlight::SharedSpans>>;

/// Syntax spans of one row, shared with the markdown cache.
pub(crate) type RowColors = Option<meku_markdown::highlight::SharedSpans>;

/// Editing commands reachable from the keymap and the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocKey {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    Home,
    End,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
    Backspace,
    Delete,
    DeleteWordBack,
    DeleteWordFwd,
    DeleteToLineStart,
    MoveLineUp,
    MoveLineDown,
    DuplicateLine,
    DeleteLine,
    InsertLineBelow,
    InsertLineAbove,
    JoinLines,
    ToggleComment,
    SoftBreak,
    Indent,
    Outdent,
    ListBullet,
    ListOrdered,
    ListTask,
    ToggleTask,
    Enter,
    SelectAll,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
}

/// Smart `Home`: byte offset of the first non-whitespace char, unless the
/// caret already sits there (or in the leading whitespace), in which case
/// the row start. Measured in chars over the row slice.
fn smart_home_dest(row: &Row, text: &str, caret: usize) -> usize {
    let line = &text[row.src.clone()];
    let indent = line.chars().take_while(|c| c.is_whitespace()).count();
    let column = text[row.src.start..caret.min(row.src.end)].chars().count();
    let offset = if column <= indent {
        0
    } else {
        line.char_indices()
            .nth(indent)
            .map(|(i, _)| i)
            .unwrap_or(line.len())
    };
    row.src.start + offset
}

/// Where `caret` sits relative to fenced code, walking the rows above it the
/// same way the highlighter does: an open delimiter sets the language, a
/// close clears it, and any non-code row closes it too. Returns
/// `(inside_fence, fence_language)`.
fn fence_at(model: &DocModel, caret: usize) -> (bool, Option<String>) {
    let row_ix = row_at(&model.rows, caret);
    let mut open: Option<String> = None;
    for row in &model.rows[..=row_ix] {
        let line = &model.text[row.src.clone()];
        let trimmed = line.trim_start();
        let is_delim = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if row.kind != RowKind::Code {
            open = None;
        } else if is_delim {
            open = match open {
                None => trimmed[3..]
                    .split_whitespace()
                    .next()
                    .map(str::to_string)
                    .filter(|t| !t.is_empty()),
                Some(_) => None,
            };
        }
    }
    (open.is_some(), open)
}

/// Comment prefix for the few v0.1-highlighted languages.
fn comment_prefix(lang: &str) -> Option<&'static str> {
    match lang {
        "rust" | "javascript" | "js" | "typescript" | "ts" => Some("//"),
        "python" | "py" | "bash" | "sh" | "shell" | "ruby" | "yaml" | "yml" | "toml" => Some("#"),
        _ => None,
    }
}

/// Parsed rows for one buffer version.
#[derive(Clone)]
pub(crate) struct DocModel {
    pub version: u64,
    pub text: Rc<str>,
    pub rows: Rc<Vec<Row>>,
    /// Syntax spans per code row, in line-content byte offsets.
    pub hl: Rc<RowSpans>,
}

/// What a rendered row exposes for hit testing: its text layout and the
/// display projection it was drawn with.
pub(crate) struct RowView {
    pub(crate) layout: TextLayout,
    pub(crate) display: Display,
}

/// Row views shared between the render pass and pointer handlers.
pub(crate) type RowViews = Rc<RefCell<Vec<Option<RowView>>>>;

impl MekuShell {
    pub(crate) fn modal_open(&self) -> bool {
        self.naming.is_some() || self.pending_delete.is_some()
    }

    /// Relative path of the note shown by the active tab, when it has a buffer.
    pub(crate) fn doc_rel(&self) -> Option<PathBuf> {
        self.tabs
            .get(self.active_tab)
            .and_then(|t| t.rel.clone())
            .filter(|rel| self.buffers.contains_key(rel))
    }

    /// Rows for the buffer at `rel`, rebuilt only when its version changed.
    pub(crate) fn doc_model(&mut self, rel: &Path, cx: &App) -> Option<DocModel> {
        let buffer = self.buffers.get(rel)?.clone();
        let version = buffer.read(cx).version;
        if let Some(model) = self.doc_models.get(rel)
            && model.version == version
        {
            return Some(model.clone());
        }
        let text: Rc<str> = Rc::from(buffer.read(cx).text_string());
        let rows = Rc::new(build_rows(&text));
        let hl = Self::colored_rows(&text, &rows);
        let model = DocModel {
            version,
            text,
            rows,
            hl,
        };
        self.doc_models.insert(rel.to_path_buf(), model.clone());
        Some(model)
    }

    pub(crate) fn doc_note_edit(&mut self, rel: &Path) {
        self.last_edit.insert(rel.to_path_buf(), Instant::now());
    }

    /// Syntax spans for every row: code-fence content rows get tree-sitter
    /// spans (cached in meku-markdown), everything else stays plain. The
    /// fence language comes from the delimiter row above; closing delimiters
    /// and the delimiter rows themselves stay uncolored.
    pub(crate) fn colored_rows(text: &str, rows: &[Row]) -> Rc<RowSpans> {
        let mut fence_lang: Option<String> = None;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let line = &text[row.src.clone()];
            let trimmed = line.trim_start();
            let is_delim = trimmed.starts_with("```") || trimmed.starts_with("~~~");
            let colored = if row.kind != RowKind::Code {
                fence_lang = None;
                None
            } else if is_delim {
                let tag = if fence_lang.is_none() {
                    trimmed[3..].split_whitespace().next()
                } else {
                    None
                };
                fence_lang = tag.map(str::to_string);
                None
            } else {
                match fence_lang.as_deref() {
                    Some(tag) if !tag.is_empty() => highlight(tag, line),
                    _ => None,
                }
            };
            out.push(colored);
        }
        Rc::new(out)
    }
    /// Render the open note as one list of rows, with a caret and selection.
    pub(crate) fn render_document(
        &mut self,
        rel: &Path,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(model) = self.doc_model(rel, cx) else {
            return div().flex_1().into_any_element();
        };
        let Some(buffer) = self.buffers.get(rel).cloned() else {
            return div().flex_1().into_any_element();
        };
        let mode = self.tab_mode();
        let note_rel = rel.to_path_buf();
        let images: Rc<Vec<Option<ImageSrc>>> = Rc::new(
            model
                .rows
                .iter()
                .map(|row| {
                    self.explorer
                        .root()
                        .and_then(|root| image_source_of(row, &model.text, root, &note_rel))
                })
                .collect(),
        );
        let (caret, selection) = {
            let b = buffer.read(cx);
            (b.caret(), b.selection())
        };
        let active = row_at(&model.rows, caret);
        let count = model.rows.len();
        if self.doc_list_rel.as_deref() != Some(rel) {
            self.doc_list.reset(count);
            self.doc_list_rel = Some(rel.to_path_buf());
        } else if self.doc_list.item_count() != count {
            self.doc_list.reset(count);
        }
        *self.doc_views.borrow_mut() = (0..count).map(|_| None).collect();

        let shell = cx.entity();
        let rows = model.rows.clone();
        let text = model.text.clone();
        let hls = model.hl.clone();
        let views = self.doc_views.clone();
        let list_body = list(self.doc_list.clone(), move |ix, window, cx| {
            let row = &rows[ix];
            // Edit reveals every row, Live only the caret's, Read none.
            let is_caret_row = mode != Mode::Read && ix == active;
            let reveal = match mode {
                Mode::Edit => true,
                Mode::Live => is_caret_row,
                Mode::Read => false,
            };
            row_element(
                ix,
                row,
                &text,
                reveal,
                is_caret_row.then_some(caret),
                match mode {
                    Mode::Read => None,
                    _ => selection_for(row, &selection),
                },
                hls[ix].clone(),
                images[ix].clone(),
                &shell,
                &views,
                window,
                cx,
            )
        })
        .size_full();

        // Registers the text-input handler for this surface while it is painted.
        let focus = self.focus_handle.clone();
        let entity = cx.entity();
        let input_bridge = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                window.handle_input(&focus, ElementInputHandler::new(bounds, entity.clone()), cx);
            },
        )
        .absolute()
        .size_full();

        div()
            .flex_1()
            .min_h_0()
            .relative()
            .id("md-scroll")
            .test_support()
            .px_4()
            .child(input_bridge)
            .child(list_body)
            .into_any_element()
    }
    /// Source byte offset for a point hit on row `ix`. Rows without a text
    /// layout (blank rules, spacers) resolve to their start.
    fn row_source(&self, ix: usize, pos: Point<Pixels>) -> Option<usize> {
        let views = self.doc_views.borrow();
        if let Some(Some(view)) = views.get(ix) {
            let idx = match view.layout.index_for_position(pos) {
                Ok(i) | Err(i) => i,
            };
            return Some(view.display.to_src(idx));
        }
        let rel = self.doc_rel()?;
        let model = self.doc_models.get(&rel)?;
        model.rows.get(ix).map(|r| r.src.start)
    }

    /// Source offset under `pos` among all rendered rows.
    fn source_at(&self, pos: Point<Pixels>) -> Option<usize> {
        let views = self.doc_views.borrow();
        views.iter().flatten().find_map(|view| {
            view.layout.bounds().contains(&pos).then(|| {
                let idx = match view.layout.index_for_position(pos) {
                    Ok(i) | Err(i) => i,
                };
                view.display.to_src(idx)
            })
        })
    }

    /// Pointer press on row `ix`: place the caret (or extend the selection).
    pub(crate) fn doc_pointer(
        &mut self,
        ix: usize,
        pos: Point<Pixels>,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        let (Some(rel), Some(src)) = (self.doc_rel(), self.row_source(ix, pos)) else {
            return;
        };
        if let Some(buffer) = self.buffers.get(&rel).cloned() {
            buffer.update(cx, |b, _| b.move_caret(src, extend));
        }
        cx.notify();
    }

    /// Pointer drag over row `ix`: extend the selection to the pointer.
    pub(crate) fn doc_drag(&mut self, ix: usize, pos: Point<Pixels>, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let (Some(rel), Some(src)) = (self.doc_rel(), self.row_source(ix, pos)) else {
            return;
        };
        if let Some(buffer) = self.buffers.get(&rel).cloned() {
            buffer.update(cx, |b, _| b.move_caret(src, true));
        }
        cx.notify();
    }

    /// Run one editing command against the active note.
    pub(crate) fn doc_key(
        &mut self,
        key: DocKey,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() || self.tab_mode() == Mode::Read {
            return;
        }
        let Some(rel) = self.doc_rel() else {
            return;
        };
        let Some(buffer) = self.buffers.get(&rel).cloned() else {
            return;
        };
        let Some(model) = self.doc_model(&rel, cx) else {
            return;
        };
        let mutated = match key {
            DocKey::Left => {
                buffer.update(cx, |b, _| b.move_left(extend));
                false
            }
            DocKey::Right => {
                buffer.update(cx, |b, _| b.move_right(extend));
                false
            }
            DocKey::WordLeft => {
                buffer.update(cx, |b, _| {
                    let to = b.word_left(b.caret());
                    b.move_caret(to, extend);
                });
                false
            }
            DocKey::WordRight => {
                buffer.update(cx, |b, _| {
                    let to = b.word_right(b.caret());
                    b.move_caret(to, extend);
                });
                false
            }
            DocKey::DocStart => {
                buffer.update(cx, |b, _| b.move_caret(0, extend));
                false
            }
            DocKey::DocEnd => {
                buffer.update(cx, |b, _| {
                    let end = b.len_bytes();
                    b.move_caret(end, extend);
                });
                false
            }
            DocKey::PageUp | DocKey::PageDown => {
                // One viewport of rows, keeping the column like Up/Down.
                let base_font = window.text_style().font_size.to_pixels(window.rem_size());
                let page = window.viewport_size().height.as_f32() / (base_font * 1.15).as_f32();
                let page_rows = (page.max(1.0) as usize).max(1);
                let caret = buffer.read(cx).caret();
                let row = row_at(&model.rows, caret);
                let last = model.rows.len() - 1;
                let target = if key == DocKey::PageUp {
                    row.saturating_sub(page_rows)
                } else {
                    (row + page_rows).min(last)
                };
                let dest = if target == row {
                    if key == DocKey::PageUp {
                        0
                    } else {
                        model.text.len()
                    }
                } else {
                    let column = model.text[model.rows[row].src.start..caret].chars().count();
                    let target_row = &model.rows[target];
                    let line = &model.text[target_row.src.clone()];
                    let offset = line
                        .char_indices()
                        .nth(column)
                        .map(|(i, _)| i)
                        .unwrap_or(line.len());
                    target_row.src.start + offset
                };
                buffer.update(cx, |b, _| b.move_caret(dest, extend));
                false
            }
            DocKey::Up | DocKey::Down => {
                let caret = buffer.read(cx).caret();
                let row = row_at(&model.rows, caret);
                let last = model.rows.len() - 1;
                let target = if key == DocKey::Up {
                    row.saturating_sub(1)
                } else {
                    (row + 1).min(last)
                };
                let dest = if target == row {
                    if key == DocKey::Up {
                        0
                    } else {
                        model.text.len()
                    }
                } else {
                    let column = model.text[model.rows[row].src.start..caret].chars().count();
                    let target_row = &model.rows[target];
                    let line = &model.text[target_row.src.clone()];
                    let offset = line
                        .char_indices()
                        .nth(column)
                        .map(|(i, _)| i)
                        .unwrap_or(line.len());
                    target_row.src.start + offset
                };
                buffer.update(cx, |b, _| b.move_caret(dest, extend));
                false
            }
            DocKey::Home => {
                // Smart: first stop is first non-whitespace, second is
                // column 0 (like editors do).
                let caret = buffer.read(cx).caret();
                let row = &model.rows[row_at(&model.rows, caret)];
                let dest = smart_home_dest(row, &model.text, caret);
                buffer.update(cx, |b, _| b.move_caret(dest, extend));
                false
            }
            DocKey::End => {
                let caret = buffer.read(cx).caret();
                let row = &model.rows[row_at(&model.rows, caret)];
                let dest = row.src.end;
                buffer.update(cx, |b, _| b.move_caret(dest, extend));
                false
            }
            DocKey::DeleteWordBack => {
                buffer.update(cx, |b, _| b.delete_word_backward());
                true
            }
            DocKey::DeleteWordFwd => {
                buffer.update(cx, |b, _| b.delete_word_forward());
                true
            }
            DocKey::DeleteToLineStart => {
                buffer.update(cx, |b, _| b.delete_to_line_start());
                true
            }
            DocKey::MoveLineUp => {
                buffer.update(cx, |b, _| b.move_line_up());
                true
            }
            DocKey::MoveLineDown => {
                buffer.update(cx, |b, _| b.move_line_down());
                true
            }
            DocKey::DuplicateLine => {
                buffer.update(cx, |b, _| b.duplicate_line());
                true
            }
            DocKey::DeleteLine => {
                buffer.update(cx, |b, _| {
                    b.delete_line();
                    // A removed ordered item closes the numbering gap.
                    b.renumber_ordered_at_caret();
                });
                true
            }
            DocKey::InsertLineBelow => {
                buffer.update(cx, |b, _| b.insert_line_below());
                true
            }
            DocKey::InsertLineAbove => {
                buffer.update(cx, |b, _| b.insert_line_above());
                true
            }
            DocKey::JoinLines => {
                buffer.update(cx, |b, _| b.join_lines());
                true
            }
            DocKey::ToggleComment => {
                let caret = buffer.read(cx).caret();
                let (inside, lang) = fence_at(&model, caret);
                let prefix = lang.as_deref().filter(|_| inside).and_then(comment_prefix);
                match prefix {
                    Some(prefix) => {
                        buffer.update(cx, |b, _| b.toggle_line_comment(prefix));
                        true
                    }
                    None => false,
                }
            }
            DocKey::Backspace => {
                buffer.update(cx, |b, _| b.backspace());
                true
            }
            DocKey::Delete => {
                buffer.update(cx, |b, _| b.delete_forward());
                true
            }
            DocKey::Enter => {
                // Enumeration-aware Enter is skipped inside fenced code:
                // a `1. item` line there must stay plain source.
                let caret = buffer.read(cx).caret();
                if fence_at(&model, caret).0 {
                    buffer.update(cx, |b, _| b.insert("\n"));
                } else {
                    buffer.update(cx, |b, _| b.smart_enter());
                }
                true
            }
            DocKey::Indent | DocKey::Outdent => {
                let code = fence_at(&model, buffer.read(cx).caret()).0;
                buffer.update(cx, |b, _| {
                    if key == DocKey::Indent {
                        b.indent(code);
                    } else {
                        b.outdent(code);
                    }
                });
                true
            }
            DocKey::ListBullet | DocKey::ListOrdered | DocKey::ListTask | DocKey::ToggleTask => {
                // List markers are markup; inside a fence they would be code.
                if fence_at(&model, buffer.read(cx).caret()).0 {
                    false
                } else {
                    buffer.update(cx, |b, _| match key {
                        DocKey::ListBullet => b.convert_list(meku_buffer::ListKind::Bullet),
                        DocKey::ListOrdered => b.convert_list(meku_buffer::ListKind::Ordered),
                        DocKey::ListTask => b.convert_list(meku_buffer::ListKind::Task),
                        _ => b.toggle_task(),
                    });
                    true
                }
            }
            DocKey::SoftBreak => {
                buffer.update(cx, |b, _| b.soft_break());
                true
            }
            DocKey::SelectAll => {
                buffer.update(cx, |b, _| b.select_all());
                false
            }
            DocKey::Undo => {
                buffer.update(cx, |b, _| b.undo());
                true
            }
            DocKey::Redo => {
                buffer.update(cx, |b, _| b.redo());
                true
            }
            DocKey::Copy => {
                let text = buffer.read(cx).selected_text();
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                false
            }
            DocKey::Cut => {
                let text = buffer.read(cx).selected_text();
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    buffer.update(cx, |b, _| b.delete_backward());
                    true
                } else {
                    false
                }
            }
            DocKey::Paste => match cx.read_from_clipboard().and_then(|item| item.text()) {
                Some(text) => {
                    buffer.update(cx, |b, _| b.insert(&text));
                    true
                }
                None => false,
            },
        };
        if mutated {
            self.doc_note_edit(&rel);
        }
        // Keep the caret's row on screen after any movement or edit.
        if let Some(model) = self.doc_model(&rel, cx) {
            let caret = buffer.read(cx).caret();
            self.doc_list
                .scroll_to_reveal_item(row_at(&model.rows, caret));
        }
        cx.notify();
    }

    /// Text and utf16 helpers for the input handler: the active note's text.
    fn doc_text(&self, cx: &App) -> Option<(PathBuf, Rc<str>)> {
        let rel = self.doc_rel()?;
        let text: Rc<str> = Rc::from(self.buffers.get(&rel)?.read(cx).text_string());
        Some((rel, text))
    }
}

/// Source range of `row` that falls inside the selection, when any.
fn selection_for(row: &Row, selection: &Range<usize>) -> Option<Range<usize>> {
    if selection.start == selection.end {
        return None;
    }
    let start = selection.start.max(row.src.start);
    let end = selection.end.min(row.src.end);
    (start < end).then_some(start..end)
}

fn heading_scale(kind: RowKind) -> f32 {
    match kind {
        RowKind::Heading(1) => 1.8,
        RowKind::Heading(2) => 1.5,
        RowKind::Heading(3) => 1.3,
        RowKind::Heading(_) => 1.1,
        _ => 1.0,
    }
}

/// Where an `![alt](target)` row's pixels come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImageSrc {
    Url(String),
    Path(PathBuf),
}

/// A row whose whole source line is one image: targeted when hidden (Live/
/// Read presentation); the caret row and Edit mode show the raw text.
fn image_source_of(row: &Row, text: &str, root: &Path, note_rel: &Path) -> Option<ImageSrc> {
    if row.kind != RowKind::Text {
        return None;
    }
    let (_, target) = meku_markdown::live::standalone_image(&text[row.src.clone()])?;
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("data:")
    {
        return Some(ImageSrc::Url(target.to_string()));
    }
    let note_dir = note_rel.parent().unwrap_or(Path::new(""));
    let joined = root.join(note_dir).join(target);
    if joined.starts_with(root) {
        Some(ImageSrc::Path(joined))
    } else {
        None
    }
}

fn apply_inline(style: &mut TextStyle, bits: InlineStyle, cx: &App) {
    if bits.contains(InlineStyle::STRONG) {
        style.font_weight = FontWeight::BOLD;
    }
    if bits.contains(InlineStyle::EMPHASIS) {
        style.font_style = FontStyle::Italic;
    }
    if bits.contains(InlineStyle::STRIKE) {
        style.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.),
            color: None,
        });
    }
    if bits.contains(InlineStyle::CODE) {
        style.color = cx.theme().accent;
    }
    if bits.contains(InlineStyle::LINK) {
        style.color = cx.theme().link;
        style.underline = Some(UnderlineStyle {
            thickness: px(1.),
            color: None,
            wavy: false,
        });
    }
}

/// Zed-like code palette for tree-sitter token classes.
fn tok_color(token: Tok, cx: &App) -> Hsla {
    match token {
        Tok::Comment => cx.theme().muted_foreground,
        Tok::Str => hex(0xa6_e3_a1),
        Tok::Num => hex(0xfa_b3_87),
        Tok::Const => hex(0xf5_c2_e7),
        Tok::Prop => hex(0x94_e2_d5),
        Tok::Func => hex(0x89_b4_fa),
        Tok::Type => hex(0xf9_e2_af),
        Tok::Op => hex(0x94_e2_d5),
        Tok::Keyword => hex(0xcb_a6_f7),
        Tok::Other => cx.theme().foreground,
    }
}

fn hex(argb: u32) -> Hsla {
    rgb(argb).into()
}

/// Text runs for a display projection: syntax colors, inline styles, dimmed
/// markup and the selection background, split at every boundary so runs cover
/// the text exactly.
fn text_runs(
    display: &Display,
    base: &TextStyle,
    selection: Option<(usize, usize)>,
    colored: &[(Range<usize>, Tok)],
    cx: &App,
) -> Vec<TextRun> {
    let len = display.text.len();
    if len == 0 {
        return Vec::new();
    }
    let mut cuts = vec![0, len];
    for run in &display.runs {
        cuts.push(run.range.start);
        cuts.push(run.range.end);
    }
    for (range, _) in colored {
        cuts.push(range.start.min(len));
        cuts.push(range.end.min(len));
    }
    if let Some((a, b)) = selection {
        cuts.push(a.min(len));
        cuts.push(b.min(len));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut runs = Vec::with_capacity(cuts.len());
    for pair in cuts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a == b {
            continue;
        }
        let mut style = base.clone();
        let colored = colored
            .iter()
            .find(|(range, _)| range.start <= a && a < range.end);
        if let Some((_, token)) = colored {
            style.color = tok_color(*token, cx);
        } else {
            let covering = display
                .runs
                .iter()
                .find(|r| r.range.start <= a && a < r.range.end);
            if let Some(run) = covering {
                apply_inline(&mut style, run.style, cx);
                if run.markup {
                    style.color = cx.theme().muted_foreground;
                }
            }
        }
        if selection.is_some_and(|(s, e)| s <= a && a < e) {
            style.background_color = Some(cx.theme().selection);
        }
        runs.push(style.to_run(b - a));
    }
    runs
}

/// Caret geometry from synchronous shaping: exact x-advance of the text
/// before `index` plus the shaped line height. Render-safe, unlike
/// `TextLayout::position_for_index`, which panics before element measurement
/// ("measurement has not been performed"). Shaped with the same text, runs
/// and size the row paints with, so the caret tracks the glyphs exactly.
/// Returns `(x_advance, y_offset, height)`, offset in the row-local box:
/// glyphs are painted centered in the line box (`(line_h - box_h) / 2`
/// padding), so the caret starts there instead of the box top.
/// Single visual line only: rows that wrap past the viewport width keep the
/// caret on the first visual line until wrap-aware positioning lands.
fn caret_geometry(
    window: &mut Window,
    text: &str,
    runs: &[TextRun],
    font_size: Pixels,
    index: usize,
) -> Option<(Pixels, Pixels, Pixels)> {
    let mut ix = index.min(text.len());
    while ix > 0 && !text.is_char_boundary(ix) {
        ix -= 1;
    }
    let shaped = window
        .text_system()
        .shape_line(text.to_string().into(), font_size, runs, None);
    let (prefix, _) = shaped.split_at(ix);
    let font_box = shaped.ascent + shaped.descent;
    let line_height = window
        .text_style()
        .line_height
        .to_pixels(font_size.into(), window.rem_size());
    let y_offset = (line_height - font_box) / 2.0;
    Some((prefix.width(), y_offset, font_box))
}

/// Caret as a plain element inside the row: `x`/`y` are row-local (the text
/// origin plus the shaped advance), so no window-space math is needed and the
/// caret cannot drift away from its row. Deliberately not a `canvas`
/// paint-quad: paint bounds live in window space and got the caret lost.
fn caret_element(x: Pixels, y: Pixels, height: Pixels, color: Hsla) -> impl IntoElement {
    div()
        .id("doc-caret")
        .test_support()
        .absolute()
        .left(x)
        .top(y)
        .w(px(2.))
        .h(height)
        .bg(color)
}

/// One row of the document: its markup projection, styles, caret and pointer handling.
#[allow(clippy::too_many_arguments)]
fn row_element(
    ix: usize,
    row: &Row,
    text: &str,
    reveal: bool,
    caret: Option<usize>,
    selection: Option<Range<usize>>,
    colored: RowColors,
    image: Option<ImageSrc>,
    shell: &Entity<MekuShell>,
    views: &RowViews,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let base_font = window.text_style().font_size.to_pixels(window.rem_size());
    let down_shell = shell.clone();
    let drag_shell = shell.clone();
    let row_div = div()
        .id(("doc-row", ix))
        .test_support()
        .relative()
        .w_full()
        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
            down_shell.update(cx, |view, cx| {
                let focus = view.focus_handle.clone();
                window.focus(&focus, cx);
                view.doc_pointer(ix, ev.position, ev.modifiers.shift, cx);
            });
        })
        .on_mouse_move(move |ev: &MouseMoveEvent, _window, cx| {
            if ev.pressed_button == Some(MouseButton::Left) {
                drag_shell.update(cx, |view, cx| view.doc_drag(ix, ev.position, cx));
            }
        });

    // Standalone image rows draw their picture when hidden; the caret row and
    // Edit mode show the raw text instead (unchanged flow below).
    if !reveal && let Some(source) = image {
        if let Some(slot) = views.borrow_mut().get_mut(ix) {
            *slot = None;
        }
        let inset = px(20.0 * f32::from(row.list_depth.min(8)));
        let mut el = row_div.pl(inset).py(px(4.));
        el = el.child(image_row_element(source, ix, cx));
        return el.into_any_element();
    }

    if row.kind == RowKind::Blank {
        if let Some(slot) = views.borrow_mut().get_mut(ix) {
            *slot = None;
        }
        let row_h = base_font * 1.1;
        let mut el = row_div.h(row_h);
        // Empty lines still need a caret: without one the cursor visibly
        // vanishes whenever the user creates a new line. Approximate the
        // text rows' glyph centering: the glyph box (~1em) sits centered in
        // the 1.1em row, so offset it by the slack.
        if caret.is_some() {
            el = el.child(caret_element(
                px(0.),
                (row_h - base_font) / 2.0,
                base_font,
                cx.theme().foreground,
            ));
        }
        return el.into_any_element();
    }
    if row.kind == RowKind::Rule && !reveal {
        return row_div
            .py(px(10.))
            .child(div().h(px(1.)).w_full().bg(cx.theme().border))
            .into_any_element();
    }

    let display = project(row, text, reveal);
    let caret_disp = caret.map(|c| display.to_disp(c));
    let sel_disp = selection.map(|s| (display.to_disp(s.start), display.to_disp(s.end)));

    let mut base = window.text_style().clone();
    if matches!(row.kind, RowKind::Heading(_)) {
        base.font_weight = FontWeight::BOLD;
    }
    let runs = text_runs(
        &display,
        &base,
        sel_disp,
        colored.as_deref().map_or(&[], |c| c.as_slice()),
        cx,
    );
    let font_size = base_font * heading_scale(row.kind);
    // Shaped before `runs`/`display` move into the element and row view.
    let caret_geo =
        caret_disp.and_then(|index| caret_geometry(window, &display.text, &runs, font_size, index));
    let styled = StyledText::new(display.text.clone()).with_runs(runs);
    let layout = styled.layout().clone();
    if let Some(slot) = views.borrow_mut().get_mut(ix) {
        *slot = Some(RowView {
            layout: layout.clone(),
            display,
        });
    }

    let mut el = row_div.text_size(font_size);
    // Text origin inside the row: mirrors the padding applied below so the
    // caret (row-local) lands exactly where the glyphs start.
    let mut text_x = px(0.);
    let mut text_y = px(0.);
    if matches!(row.kind, RowKind::Heading(_)) {
        el = el.pt(px(6.));
        text_y = px(6.);
    }
    if row.quote_depth > 0 {
        let depth = f32::from(row.quote_depth.min(4));
        el = el
            .border_l_2()
            .border_color(cx.theme().accent)
            .pl(px(14.0 * depth));
        // Later padding refinements replace earlier ones (same as the style
        // system), so track by assignment, not accumulation.
        text_x = px(14.0 * depth);
    }
    if !reveal && row.list_depth > 0 {
        let indent = px(20.0 * f32::from(row.list_depth.min(8)));
        el = el.pl(indent);
        text_x = indent;
    }
    if row.kind == RowKind::Code {
        el = el.bg(cx.theme().sidebar).rounded_sm();
        if colored.is_none() {
            el = el.px_2();
            text_x = px(8.);
        }
    }
    el = el.child(styled);
    if let Some((advance, y_offset, cheight)) = caret_geo {
        el = el.child(caret_element(
            text_x + advance,
            text_y + y_offset,
            cheight,
            cx.theme().foreground,
        ));
    }
    el.into_any_element()
}

/// Placeholder box with the image (or its loading/fallback states) stretched
/// inside a fixed frame, so the row height cannot jitter between load states.
fn image_row_element(source: ImageSrc, ix: usize, cx: &App) -> AnyElement {
    let frame = |inner: AnyElement| {
        div()
            .id(("image-row", ix))
            .test_support()
            .h(px(240.))
            .w(px(480.))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .overflow_hidden()
            .child(inner)
            .into_any_element()
    };
    let inner = match source {
        ImageSrc::Path(path) => img(path)
            .id("doc-image")
            .size_full()
            .max_h(px(240.))
            .object_fit(ObjectFit::Contain)
            .with_fallback(|| {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(Hsla::default())
                    .child("Image unavailable")
                    .into_any_element()
            }),
        ImageSrc::Url(url) => img(url)
            .id("doc-image")
            .size_full()
            .max_h(px(240.))
            .object_fit(ObjectFit::Contain)
            .with_loading(|| {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .child("Loading…")
                    .into_any_element()
            })
            .with_fallback(|| {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .child("Image unavailable")
                    .into_any_element()
            }),
    };
    frame(inner.into_any_element())
}

// -- text input ----------------------------------------------------------

/// Convert a utf16 offset into `text` to a byte offset (clamped to the end).
fn utf16_to_byte(text: &str, target: usize) -> usize {
    let mut units = 0usize;
    for (i, c) in text.char_indices() {
        if units >= target {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

/// Convert a byte offset into `text` to a utf16 offset (floored to a char boundary).
fn byte_to_utf16(text: &str, byte: usize) -> usize {
    let mut b = byte.min(text.len());
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    text[..b].encode_utf16().count()
}

fn utf16_range_to_bytes(text: &str, range: Range<usize>) -> Range<usize> {
    utf16_to_byte(text, range.start)..utf16_to_byte(text, range.end)
}

fn bytes_to_utf16_range(text: &str, range: Range<usize>) -> Range<usize> {
    byte_to_utf16(text, range.start)..byte_to_utf16(text, range.end)
}

impl EntityInputHandler for MekuShell {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if self.modal_open() {
            return None;
        }
        let (_, text) = self.doc_text(cx)?;
        let bytes = utf16_range_to_bytes(&text, range);
        *adjusted_range = Some(bytes_to_utf16_range(&text, bytes.clone()));
        text.get(bytes).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.modal_open() {
            return None;
        }
        let (rel, text) = self.doc_text(cx)?;
        let buffer = self.buffers.get(&rel)?.read(cx);
        let selection = buffer.selection();
        Some(UTF16Selection {
            range: bytes_to_utf16_range(&text, selection),
            reversed: buffer.caret() < buffer.anchor(),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let (rel, text) = self.doc_text(cx)?;
        let marked = self.buffers.get(&rel)?.read(cx).marked()?;
        Some(bytes_to_utf16_range(&text, marked))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(rel) = self.doc_rel()
            && let Some(buffer) = self.buffers.get(&rel).cloned()
        {
            buffer.update(cx, |b, _| b.set_marked(None));
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() || self.tab_mode() == Mode::Read {
            return;
        }
        let Some((rel, doc)) = self.doc_text(cx) else {
            return;
        };
        let Some(buffer) = self.buffers.get(&rel).cloned() else {
            return;
        };
        let plain = range.is_none();
        let target = {
            let b = buffer.read(cx);
            match range {
                Some(r) => utf16_range_to_bytes(&doc, r),
                None => b.marked().unwrap_or_else(|| b.selection()),
            }
        };
        // Plain typing (no explicit range, no composition) gets auto-pairing.
        let mut chars = text.chars();
        if let (true, Some(ch), None) = (plain, chars.next(), chars.next())
            && let Some(model) = self.doc_model(&rel, cx)
        {
            let code = fence_at(&model, buffer.read(cx).caret()).0;
            if buffer.update(cx, |b, _| b.type_char(ch, code)) {
                self.doc_note_edit(&rel);
                cx.notify();
                return;
            }
        }
        let typing = text.chars().count() == 1 && text != "\n";
        buffer.update(cx, |b, _| b.replace(target, text, typing));
        self.doc_note_edit(&rel);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() || self.tab_mode() == Mode::Read {
            return;
        }
        let Some((rel, doc)) = self.doc_text(cx) else {
            return;
        };
        let Some(buffer) = self.buffers.get(&rel).cloned() else {
            return;
        };
        let target = {
            let b = buffer.read(cx);
            match range {
                Some(r) => utf16_range_to_bytes(&doc, r),
                None => b.marked().unwrap_or_else(|| b.selection()),
            }
        };
        let start = target.start;
        buffer.update(cx, |b, _| {
            b.replace(target, new_text, false);
            b.set_marked(Some(start..start + new_text.len()));
            if let Some(sel) = new_selected_range {
                let inner = utf16_range_to_bytes(new_text, sel);
                b.move_caret(start + inner.end, false);
            }
        });
        self.doc_note_edit(&rel);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let rel = self.doc_rel()?;
        let caret = self.buffers.get(&rel)?.read(cx).caret();
        let model = self.doc_models.get(&rel)?;
        let row = row_at(&model.rows, caret);
        let views = self.doc_views.borrow();
        let view = views.get(row)?.as_ref()?;
        let origin = view
            .layout
            .position_for_index(view.display.to_disp(caret))?;
        Some(Bounds::new(origin, size(px(2.), view.layout.line_height())))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let (_, text) = self.doc_text(cx)?;
        let src = self.source_at(point)?;
        Some(byte_to_utf16(&text, src))
    }
}
