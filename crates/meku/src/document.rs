//! The single-surface document editor.
//!
//! The open note is one scrollable list with one row per source line. The
//! caret's row is shown raw (markers dimmed); every other row renders with its
//! markup hidden. Editing happens on the buffer directly, so typing re-renders
//! in place: there is no editor swap and no mode switch.
//!
//! The GPUI text-input handler lives here too, so IME, dead keys and the
//! platform text services reach the same caret as keyboard actions do.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use crate::icons::{Ico, icon};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::{ScrollableElement as _, ScrollbarHandle};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;
use meku_buffer::OpenBuffer;
use meku_markdown::InlineStyle;
use meku_markdown::highlight::{Tok, highlight};
use meku_markdown::live::{
    CalloutKind, CalloutRole, Display, Fence, LinkTarget, Row, RowKind, TableRole, build_rows,
    classify_link, heading_slug, link_at, project, rebuild_rows, row_at,
};
use meku_table::Align;

use crate::MekuShell;
use crate::Mode;
use crate::context_menu::Target;

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
    Wrap(&'static str),
    Heading(usize),
    Quote,
    Expand,
    Link,
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
    /// Standalone-image source per row (computed once per version).
    pub images: Rc<Vec<Option<Vec<ImageInfo>>>>,
    /// Word count of the note (status bar).
    pub words: usize,
    /// Markdown rendering options the model was built with (see [`model_opts`]).
    pub opts: u8,
}

/// Bit set of the settings that change how a note's rows are built, so a
/// model is rebuilt when one flips: 1 tables, 2 callouts, 4 images.
pub(crate) fn model_opts(cx: &App) -> u8 {
    let c = crate::theme::cfg(cx);
    u8::from(c.bool("render_tables"))
        | u8::from(c.bool("render_callouts")) << 1
        | u8::from(c.bool("render_images")) << 2
}

/// What a rendered row exposes for hit testing: its text layout and the
/// display projection it was drawn with.
pub(crate) struct RowView {
    pub(crate) layout: TextLayout,
    pub(crate) display: Display,
    /// A checkbox control sits left of the text (hidden task rows): presses
    /// there belong to the control, not the caret.
    pub(crate) has_checkbox: bool,
    /// Set once the row has been laid out and painted. `TextLayout` panics
    /// when queried before that, and rows are registered while being built.
    painted: Rc<Cell<bool>>,
}

impl RowView {
    /// Whether the layout may be queried (it has been measured and painted).
    pub(crate) fn is_painted(&self) -> bool {
        self.painted.get()
    }
}

/// Row views shared between the render pass and pointer handlers.
pub(crate) type RowViews = Rc<RefCell<Vec<Option<RowView>>>>;

impl MekuShell {
    pub(crate) fn modal_open(&self) -> bool {
        self.naming.is_some()
            || self.pending_delete.is_some()
            || self.palette_open()
            || self.lightbox.is_some()
            || self.settings.is_some()
            || self.update_open
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
        let opts = model_opts(cx);
        if let Some(model) = self.doc_models.get(rel)
            && model.version == version
            && model.opts == opts
        {
            return Some(model.clone());
        }
        let text: Rc<str> = Rc::from(buffer.read(cx).text_string());
        // Typing re-parses only the block around the edit when it can.
        let patched = self
            .doc_models
            .get(rel)
            .filter(|m| m.opts == opts)
            .and_then(|m| rebuild_rows(&m.text, &m.rows, &text));
        let (mut built, fresh) = match patched {
            Some(r) => (r.rows, r.window),
            None => {
                let rows = build_rows(&text);
                let all = 0..rows.len();
                (rows, all)
            }
        };
        let c = crate::theme::cfg(cx);
        for row in &mut built[fresh] {
            if !c.bool("render_tables") && row.table.take().is_some() && row.kind == RowKind::Table
            {
                row.kind = RowKind::Text;
            }
            if !c.bool("render_callouts") {
                row.callout = None;
            }
        }
        let rows = Rc::new(built);
        let hl = Self::colored_rows(&text, &rows);
        let root = self.explorer.root().map(Path::to_path_buf);
        let mut dim_cache = std::mem::take(&mut self.image_dims);
        let images = {
            let explorer = &self.explorer;
            let resolve = |name: &str| explorer.resolve_attachment(rel, name);
            Rc::new(
                rows.iter()
                    .map(|row| {
                        root.as_deref().filter(|_| opts & 4 != 0).and_then(|root| {
                            image_source_of(row, &text, root, rel, &mut dim_cache, &resolve)
                        })
                    })
                    .collect(),
            )
        };
        self.image_dims = dim_cache;
        let words = text.split_whitespace().count();
        let model = DocModel {
            version,
            text,
            rows,
            hl,
            images,
            words,
            opts,
        };
        self.doc_models.insert(rel.to_path_buf(), model.clone());
        Some(model)
    }

    /// True while the inline title field has keyboard focus: document key
    /// bindings (Enter, arrows, ...) must not also act on the body then.
    pub(crate) fn title_focused(&self, window: &Window, cx: &App) -> bool {
        self.title_inputs
            .values()
            .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    /// Input holds the caret solid (and visible) before blinking resumes.
    pub(crate) fn touch_caret(&mut self) {
        self.caret_on = true;
        self.caret_touch = Instant::now();
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(model) = self.doc_model(rel, cx) else {
            return div().flex_1().into_any_element();
        };
        let Some(buffer) = self.buffers.get(rel).cloned() else {
            return div().flex_1().into_any_element();
        };
        let mode = self.tab_mode();
        let images = model.images.clone();
        let (caret, selection) = {
            let b = buffer.read(cx);
            (b.caret(), b.selection())
        };
        let active = row_at(&model.rows, caret);
        let count = model.rows.len();
        // Unmeasured rows count as zero height to the scroll bar, so give
        // every row an estimate; real heights replace it as rows are drawn.
        let line_h = f32::from(window.line_height()) * crate::theme::zoom(cx);
        let hint = px(estimated_row_height(&model.rows, line_h));
        let old_count = self.doc_list.item_count();
        if self.doc_list_rel.as_deref() != Some(rel) {
            self.doc_list.reset_with_uniform_height(count, hint);
            self.doc_hint_width.set(0.0);
            self.doc_list_rel = Some(rel.to_path_buf());
        } else if old_count != count {
            let delta = count.abs_diff(old_count);
            if delta <= 50 {
                // An edit added or removed a few rows at the caret: keep the
                // measured rest, and the scroll position, as they are.
                let at = active.min(old_count);
                self.doc_list.splice(
                    at..(at + old_count.saturating_sub(count)).min(old_count),
                    count.saturating_sub(old_count),
                );
            } else {
                self.doc_list.reset_with_uniform_height(count, hint);
                self.doc_hint_width.set(0.0);
            }
        }
        *self.doc_views.borrow_mut() = (0..count).map(|_| None).collect();
        // Startup scroll waits until the list has been measured.
        if let Some((row, frames)) = self.pending_scroll {
            if frames == 0 {
                self.pending_scroll = None;
                self.doc_list
                    .scroll_to_reveal_item(row.min(count.saturating_sub(1)));
            } else {
                self.pending_scroll = Some((row, frames - 1));
                cx.notify();
            }
        }

        let shell = cx.entity();
        let rows = model.rows.clone();
        let text = model.text.clone();
        let hls = model.hl.clone();
        let views = self.doc_views.clone();
        let caret_on = self.caret_on;
        let wrap_cell = self.doc_wrap.clone();
        let tables = self.doc_tables.clone();
        let slash_view = self.slash_view(cx);
        let copied_ix = self
            .copied
            .filter(|(_, at)| at.elapsed() < std::time::Duration::from_millis(1500))
            .map(|(i, _)| i);
        let version = model.version;
        let col_width = wrap_cell.get();
        let hover_link = self.hover_link;
        let find_all: Rc<Vec<(Range<usize>, bool)>> = Rc::new(self.find_highlights(rel));
        let title_input = self.title_inputs.get(rel).cloned();
        let reveal_pref = Reveal::of(cx);
        let column_em = column_em(cx);
        // While the title field has focus the typing goes there, so the body
        // must not draw a caret of its own on the first line.
        let title_has_focus = self.title_focused(window, cx);
        let picture_cache = self.images.clone();
        let list_body = list(self.doc_list.clone(), move |ix, window, cx| {
            let row = &rows[ix];
            let is_caret_row = mode != Mode::Read && ix == active && !title_has_focus;
            let reveal = row_is_revealed_with(reveal_pref, mode, row, ix, active, &selection);
            let base = window.text_style().font_size.to_pixels(window.rem_size());
            let el = row_element(
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
                hover_link == Some(ix),
                {
                    let lo = find_all.partition_point(|(r, _)| r.end <= row.src.start);
                    let hi = lo + find_all[lo..].partition_point(|(r, _)| r.start < row.src.end);
                    &find_all[lo..hi]
                },
                col_width,
                &rows,
                &tables,
                version,
                copied_ix == Some(ix),
                caret_on,
                if is_caret_row {
                    slash_view.as_ref()
                } else {
                    None
                },
                &shell,
                &picture_cache,
                &views,
                window,
                cx,
            );
            // Centered reading column (about 68ch), 56px above the first line
            // and 45% of the window below the last so the end can rise to
            // the middle.
            let mut item = div().w_full().flex().justify_center().px(px(24.));
            let el = match (ix, title_input.as_ref()) {
                // The inline title sits above the first row, scrolls with the
                // document and is not part of the markdown.
                (0, Some(input)) => v_flex()
                    .w_full()
                    .child(title_element(input, base))
                    .child(el)
                    .into_any_element(),
                _ => el,
            };
            if ix == 0 {
                item = item.pt(px(56.));
            }
            if ix + 1 == rows.len() {
                item = item.pb(window.viewport_size().height * 0.45);
            }
            // A press in the margins beside a row (or in the room under the
            // last one) still places the caret: on the nearest text, or at the
            // end of the note when it is below all of it.
            let margin_shell = shell.clone();
            let last = ix + 1 == rows.len();
            let margin_menu = shell.clone();
            item = item.on_mouse_down(
                MouseButton::Right,
                move |ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    margin_menu.update(cx, |view, cx| {
                        view.open_doc_menu(Target::Text(Some(ix)), ev.position, window, cx)
                    });
                },
            );
            item = item.on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                margin_shell.update(cx, |view, cx| {
                    let focus = view.focus_handle.clone();
                    window.focus(&focus, cx);
                    let below = last
                        && view
                            .doc_views
                            .borrow()
                            .get(ix)
                            .and_then(|v| v.as_ref())
                            .filter(|v| v.is_painted())
                            .is_none_or(|v| ev.position.y > v.layout.bounds().bottom());
                    if below {
                        view.doc_click_below(cx);
                    } else {
                        view.doc_pointer(ix, ev.position, ev.modifiers.shift, ev.click_count, cx);
                    }
                });
            });
            item.child(div().w_full().max_w(base * column_em).child(el))
                .into_any_element()
        })
        .size_full();

        // Registers the text-input handler for this surface while it is painted.
        let focus = self.focus_handle.clone();
        let entity = cx.entity();
        let input_bridge = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                window.handle_input(&focus, ElementInputHandler::new(bounds, entity.clone()), cx);
                // Width rows wrap at: the scroll area minus the margins around
                // the reading column, capped at the column width. The caret
                // needs it to shape wrapped lines the same way they paint.
                let base = window.text_style().font_size.to_pixels(window.rem_size());
                let inner = bounds.size.width - px(2.0 * (16.0 + 24.0));
                let width = inner.min(base * column_em).max(px(0.));
                if (wrap_cell.get() - width).abs() > px(0.5) {
                    wrap_cell.set(width);
                    window.refresh();
                }
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
            // Zoom scales the document only (chrome keeps its size): every row
            // measure derives from this container's text size.
            .text_size(cx.theme().font_size * crate::theme::zoom(cx))
            .on_mouse_down(MouseButton::Left, {
                let shell = cx.entity();
                move |_, window, cx| {
                    shell.update(cx, |view, cx| {
                        let focus = view.focus_handle.clone();
                        window.focus(&focus, cx);
                        view.doc_click_below(cx);
                    });
                }
            })
            .on_mouse_down(MouseButton::Right, {
                let shell = cx.entity();
                move |ev: &MouseDownEvent, window, cx| {
                    shell.update(cx, |view, cx| {
                        view.open_doc_menu(Target::Text(None), ev.position, window, cx)
                    });
                }
            })
            .child(input_bridge)
            .child(list_body)
            .child(self.row_estimates(hint))
            .child(crate::smooth::wheel(
                &self.smooth.doc,
                crate::smooth::for_list(&self.doc_list),
            ))
            .vertical_scrollbar(&DocBar(self.doc_list.clone()))
            .children(self.render_find_bar(cx))
            .pipe_fade(buffer.entity_id(), cx)
    }
    /// The list forgets its row-height estimates whenever its width changes
    /// (and on its first layout), which leaves the scroll bar with only the
    /// rows drawn so far. This zero-size layer runs after the list has laid
    /// out and puts the estimates back, once per width, keeping the scroll
    /// position.
    fn row_estimates(&self, hint: Pixels) -> impl IntoElement {
        let list = self.doc_list.clone();
        let applied = self.doc_hint_width.clone();
        canvas(
            move |_, window, _| {
                let width = f32::from(list.viewport_bounds().size.width);
                if width > 0.0 && (applied.get() - width).abs() > 0.5 {
                    applied.set(width);
                    let top = list.logical_scroll_top();
                    list.reset_with_uniform_height(list.item_count(), hint);
                    list.scroll_to(top);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }

    /// Source byte offset for a point hit on row `ix`. Rows without a text
    /// layout (blank rules, spacers) resolve to their start.
    pub(crate) fn row_source(&self, ix: usize, pos: Point<Pixels>) -> Option<usize> {
        let views = self.doc_views.borrow();
        if let Some(Some(view)) = views.get(ix)
            && view.is_painted()
        {
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
        views.iter().flatten().filter(|v| v.is_painted()).find_map(|view| {
            view.layout.bounds().contains(&pos).then(|| {
                let idx = match view.layout.index_for_position(pos) {
                    Ok(i) | Err(i) => i,
                };
                view.display.to_src(idx)
            })
        })
    }

    /// A press below all the text: focus the note and put the caret at the end
    /// of its last line of text (not on a trailing blank line).
    pub(crate) fn doc_click_below(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let Some(rel) = self.doc_rel() else { return };
        let Some(end) = self.doc_models.get(&rel).map(|m| {
            m.rows
                .iter()
                .rev()
                .find(|r| r.kind != RowKind::Blank)
                .map_or(m.text.len(), |r| r.src.end)
        }) else {
            return;
        };
        self.touch_caret();
        self.flush_title(cx);
        if let Some(buffer) = self.buffers.get(&rel).cloned() {
            buffer.update(cx, |b, _| b.move_caret(end, false));
        }
        self.update_slash(false, cx);
        cx.notify();
    }

    /// Pointer press on row `ix`: place the caret (or extend the selection).
    pub(crate) fn doc_pointer(
        &mut self,
        ix: usize,
        pos: Point<Pixels>,
        extend: bool,
        clicks: usize,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        self.touch_caret();
        self.flush_title(cx);
        if let Some(Some(view)) = self.doc_views.borrow().get(ix)
            && view.is_painted()
            && view.has_checkbox
            && pos.x < view.layout.bounds().left()
        {
            return; // the checkbox control handles this press
        }
        let (Some(rel), Some(src)) = (self.doc_rel(), self.row_source(ix, pos)) else {
            return;
        };
        if let Some(buffer) = self.buffers.get(&rel).cloned() {
            buffer.update(cx, |b, _| match clicks {
                0 | 1 => b.move_caret(src, extend),
                2 => b.select_word_at(src),
                _ => b.select_line_at(src),
            });
        }
        self.update_slash(false, cx);
        cx.notify();
    }

    /// Follow the link under `pos` in row `ix`. Returns `false` when the
    /// pointer is not over a followable link (the click then acts normally).
    pub(crate) fn doc_follow_link(
        &mut self,
        ix: usize,
        pos: Point<Pixels>,
        ctrl: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.link_under(ix, pos, ctrl, cx) {
            Some((rel, target)) => match classify_link(&target, &rel) {
                Some(target) => self.follow_link(target, cx),
                None => false,
            },
            None => false,
        }
    }

    /// The link destination under `pos` in row `ix` when a click there
    /// should follow it: always with Ctrl, otherwise whenever the row shows
    /// rendered text (Read mode, or any row but the caret's in Live). On the
    /// caret row and in Edit the raw source is being edited, so a plain
    /// click just places the caret.
    fn link_under(
        &mut self,
        ix: usize,
        pos: Point<Pixels>,
        ctrl: bool,
        cx: &App,
    ) -> Option<(PathBuf, String)> {
        if self.modal_open() {
            return None;
        }
        let rel = self.doc_rel()?;
        let src = self.row_source(ix, pos)?;
        let caret = self.buffers.get(&rel)?.read(cx).caret();
        let model = self.doc_models.get(&rel)?;
        let row = model.rows.get(ix)?;
        if row.kind == RowKind::Code {
            return None;
        }
        let selection = self.buffers.get(&rel)?.read(cx).selection();
        let rendered = !row_is_revealed_with(
            Reveal::of(cx),
            self.tab_mode(),
            row,
            ix,
            row_at(&model.rows, caret),
            &selection,
        );
        let plain_click = crate::theme::cfg(cx).str("link_click") == "click";
        if !(ctrl || (rendered && plain_click)) {
            return None;
        }
        // The pointer must really be over the text, not the padding beside it.
        let views = self.doc_views.borrow();
        if let Some(Some(view)) = views.get(ix)
            && view.is_painted()
            && !view.layout.bounds().contains(&pos)
        {
            return None;
        }
        let line = &model.text[row.src.clone()];
        let link = link_at(line, src.saturating_sub(row.src.start))?;
        Some((rel, link.target))
    }

    /// Pointer moved over row `ix`: remember whether it is on a followable
    /// link so the row can show the pointing hand.
    pub(crate) fn doc_hover(
        &mut self,
        ix: usize,
        pos: Point<Pixels>,
        ctrl: bool,
        cx: &mut Context<Self>,
    ) {
        let over = self.link_under(ix, pos, ctrl, cx).is_some();
        let next = over.then_some(ix);
        if self.hover_link != next {
            self.hover_link = next;
            cx.notify();
        }
    }

    /// Checkbox click: flip the task box at byte `at` of the active note.
    pub(crate) fn toggle_task_at(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(rel) = self.doc_rel() else {
            return;
        };
        if let Some(buffer) = self.buffers.get(&rel).cloned() {
            buffer.update(cx, |b, _| b.toggle_task_box(at));
            self.doc_note_edit(&rel);
            cx.notify();
        }
    }

    /// Open a classified link: external → system handler, `#x` → heading in
    /// this note, path → note tab (plus heading). Missing targets do nothing.
    pub(crate) fn follow_link(&mut self, target: LinkTarget, cx: &mut Context<Self>) -> bool {
        match target {
            LinkTarget::External(url) => {
                cx.open_url(&url);
                true
            }
            LinkTarget::Anchor(name) => self.goto_heading(&name, cx),
            LinkTarget::Path { rel, anchor } => {
                let Some(root) = self.explorer.root() else {
                    return false;
                };
                if !meku_vault::is_note(&rel) || !root.join(&rel).is_file() {
                    return false;
                }
                self.open_file(&rel, cx);
                if let Some(name) = anchor {
                    self.goto_heading(&name, cx);
                }
                true
            }
        }
    }

    /// Jump to the heading whose slug (or exact title) matches `name`.
    fn goto_heading(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let slug = heading_slug(name);
        let hit = self
            .doc_outline(cx)
            .into_iter()
            .find(|(_, title, _)| heading_slug(title) == slug || title == name);
        match hit {
            Some((_, _, at)) => {
                self.doc_goto(at, cx);
                true
            }
            None => false,
        }
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
        if self.modal_open()
            || self.tab_mode() == Mode::Read
            || self.title_focused(window, cx)
            || self.find_focused(window, cx)
        {
            return;
        }
        self.touch_caret();
        // An open slash menu takes the navigation keys.
        if self.slash.is_some() {
            match key {
                DocKey::Up => {
                    self.slash_move(-1, cx);
                    return;
                }
                DocKey::Down => {
                    self.slash_move(1, cx);
                    return;
                }
                DocKey::Enter | DocKey::Indent => {
                    self.slash_confirm(None, cx);
                    return;
                }
                _ => {}
            }
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
        // What is drawn decides how the caret moves: rendered rows hide markup.
        let mode = self.tab_mode();
        let (sel, caret_now) = {
            let b = buffer.read(cx);
            (b.selection(), b.caret())
        };
        let cur_ix = row_at(&model.rows, caret_now);
        let pref = Reveal::of(cx);
        let revealed_now =
            row_is_revealed_with(pref, mode, &model.rows[cur_ix], cur_ix, cur_ix, &sel);
        let revealed_when_caret =
            |t: usize| row_is_revealed_with(pref, mode, &model.rows[t], t, t, &(0..0));
        let mutated = match key {
            DocKey::Left | DocKey::Right => {
                let forward = key == DocKey::Right;
                let row_src = model.rows[cur_ix].src.clone();
                let on_image = !revealed_now && model.images[cur_ix].is_some();
                let target = if !(sel.start == sel.end || extend) {
                    None
                } else if on_image {
                    // Images are one atom: the caret is before or after them.
                    if forward {
                        (caret_now < row_src.end).then_some(row_src.end)
                    } else {
                        (caret_now > row_src.start).then_some(row_src.start)
                    }
                } else {
                    Visible::of(&model, cur_ix, revealed_now)
                        .and_then(|v| v.step(&model.text, &row_src, caret_now, forward))
                };
                buffer.update(cx, |b, _| match target {
                    Some(p) => b.move_caret(p, extend),
                    None if forward => b.move_right(extend),
                    None => b.move_left(extend),
                });
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
                    let column = match Visible::of(&model, row, revealed_now) {
                        Some(v) => v.column(caret),
                        None => model.text[model.rows[row].src.start..caret].chars().count(),
                    };
                    let target_row = &model.rows[target];
                    match Visible::of(&model, target, revealed_when_caret(target))
                        .filter(|_| model.images[target].is_none())
                    {
                        Some(v) => v.offset(column),
                        None if model.images[target].is_some() => target_row.src.end,
                        None => {
                            let line = &model.text[target_row.src.clone()];
                            let offset = line
                                .char_indices()
                                .nth(column)
                                .map(|(i, _)| i)
                                .unwrap_or(line.len());
                            target_row.src.start + offset
                        }
                    }
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
                    let column = match Visible::of(&model, row, revealed_now) {
                        Some(v) => v.column(caret),
                        None => model.text[model.rows[row].src.start..caret].chars().count(),
                    };
                    let target_row = &model.rows[target];
                    match Visible::of(&model, target, revealed_when_caret(target))
                        .filter(|_| model.images[target].is_none())
                    {
                        Some(v) => v.offset(column),
                        None if model.images[target].is_some() => target_row.src.end,
                        None => {
                            let line = &model.text[target_row.src.clone()];
                            let offset = line
                                .char_indices()
                                .nth(column)
                                .map(|(i, _)| i)
                                .unwrap_or(line.len());
                            target_row.src.start + offset
                        }
                    }
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
            DocKey::Backspace | DocKey::Delete => {
                let backward = key == DocKey::Backspace;
                let collapsed = sel.start == sel.end;
                // Backspace on an image row removes the last image (with the
                // spaces before it); select the line to edit the source.
                if backward
                    && collapsed
                    && !revealed_now
                    && let Some(last) = model.images[cur_ix].as_ref().and_then(|v| v.last())
                {
                    let line_start = model.rows[cur_ix].src.start;
                    let mut start = last.range.start;
                    while start > line_start && model.text.as_bytes()[start - 1] == b' ' {
                        start -= 1;
                    }
                    let range = start..last.range.end;
                    buffer.update(cx, |b, _| b.replace(range, "", false));
                    self.doc_note_edit(&rel);
                    cx.notify();
                    return;
                }
                let vis = (collapsed && !revealed_now)
                    .then(|| Visible::of(&model, cur_ix, revealed_now))
                    .flatten();
                buffer.update(cx, |b, _| {
                    // At the start of a list item Backspace peels the marker.
                    if backward && vis.is_some() && b.list_backspace() {
                        return;
                    }
                    let range = vis.as_ref().and_then(|v| v.char_range(caret_now, backward));
                    // Hidden markup in the way (the visible char is not the
                    // adjacent source char): delete the visible one. Otherwise
                    // keep the buffer's smart backspace (pairs, indent stops).
                    let plain = |r: &Range<usize>| {
                        if backward {
                            r.end == caret_now
                        } else {
                            r.start == caret_now
                        }
                    };
                    match range {
                        Some(r) if !plain(&r) => b.replace(r, "", false),
                        _ if backward => b.backspace(),
                        _ => b.delete_forward(),
                    }
                });
                true
            }
            DocKey::Enter => {
                // Enumeration-aware Enter is skipped inside fenced code:
                // a `1. item` line there must stay plain source.
                let caret = buffer.read(cx).caret();
                if fence_at(&model, caret).0 {
                    buffer.update(cx, |b, _| b.insert("\n"));
                } else if !crate::theme::cfg(cx).bool("smart_lists") {
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
            DocKey::Link => {
                // Caret inside an existing link: select its destination to
                // retype. Otherwise start a new one.
                let (line_start, caret) = {
                    let b = buffer.read(cx);
                    let row = model.rows.get(row_at(&model.rows, b.caret()));
                    (row.map(|r| r.src.clone()), b.caret())
                };
                let existing = line_start.and_then(|src| {
                    let line = &model.text[src.clone()];
                    link_at(line, caret - src.start).and_then(|l| {
                        let whole = &line[l.range.clone()];
                        let open = whole.rfind("](")?;
                        let dest =
                            src.start + l.range.start + open + 2..src.start + l.range.end - 1;
                        (dest.start <= dest.end).then_some(dest)
                    })
                });
                buffer.update(cx, |b, _| match existing {
                    Some(dest) => b.select_range(dest),
                    None => b.insert_link(),
                });
                true
            }
            DocKey::Wrap(marker) => {
                if fence_at(&model, buffer.read(cx).caret()).0 {
                    false
                } else {
                    buffer.update(cx, |b, _| b.toggle_wrap(marker));
                    true
                }
            }
            DocKey::Heading(level) => {
                if fence_at(&model, buffer.read(cx).caret()).0 {
                    false
                } else {
                    buffer.update(cx, |b, _| b.set_heading(level));
                    true
                }
            }
            DocKey::Quote => {
                if fence_at(&model, buffer.read(cx).caret()).0 {
                    false
                } else {
                    buffer.update(cx, |b, _| b.toggle_quote());
                    true
                }
            }
            DocKey::Expand => {
                buffer.update(cx, |b, _| b.expand_selection());
                false
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
                } else if let Some(path) = self.image_under_caret(&rel, cx) {
                    // Nothing selected on an image line: copy the picture.
                    self.copy_image_file(&path, cx);
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
            DocKey::Paste => {
                let clip = cx.read_from_clipboard();
                // Picture files copied in a file manager: copy them into the
                // attachments folder and link them in.
                if let Some(files) = clip.as_ref().and_then(crate::clipboard::image_files) {
                    self.on_drop_paths(&files, cx);
                    self.doc_note_edit(&rel);
                    return;
                }
                match clip.as_ref().and_then(|item| item.text()) {
                    Some(text) => {
                        buffer.update(cx, |b, _| b.paste(&text));
                        true
                    }
                    None => match clip.as_ref().and_then(pasted_image) {
                        // Screenshots and copied pictures: save beside the
                        // note and link them in.
                        Some(image) => self.paste_image(&rel, &buffer, image, cx),
                        None => false,
                    },
                }
            }
        };
        if mutated {
            self.doc_note_edit(&rel);
        }
        // Keep the caret's row on screen after any movement or edit.
        if let Some(model) = self.doc_model(&rel, cx) {
            let caret = buffer.read(cx).caret();
            self.doc_list
                .scroll_to_reveal_item(row_at(&model.rows, caret));
            let in_code = fence_at(&model, caret).0;
            self.update_slash(in_code, cx);
        }
        cx.notify();
    }

    /// Headings of the active note as `(level, title, source offset)`, in
    /// document order. Fenced `#` lines are `Code` rows, so never listed.
    pub(crate) fn doc_outline(&mut self, cx: &App) -> Vec<(u8, String, usize)> {
        let Some(rel) = self.doc_rel() else {
            return Vec::new();
        };
        let Some(model) = self.doc_model(&rel, cx) else {
            return Vec::new();
        };
        model
            .rows
            .iter()
            .filter_map(|row| {
                let RowKind::Heading(level) = row.kind else {
                    return None;
                };
                let line = &model.text[row.src.clone()];
                let title = line.trim_start().trim_start_matches('#').trim();
                let title = title.trim_end_matches('#').trim_end();
                Some((level, title.to_string(), row.src.start))
            })
            .collect()
    }

    /// Move the caret to `offset` (a heading start) and scroll its row in.
    pub(crate) fn doc_goto(&mut self, offset: usize, cx: &mut Context<Self>) {
        let Some(rel) = self.doc_rel() else {
            return;
        };
        let Some(buffer) = self.buffers.get(&rel).cloned() else {
            return;
        };
        buffer.update(cx, |b, _| b.move_caret(offset, false));
        if let Some(model) = self.doc_model(&rel, cx) {
            self.doc_list
                .scroll_to_reveal_item(row_at(&model.rows, offset));
        }
        cx.notify();
    }

    /// Put the caret at `at` (a cell click): no scrolling, caret kept solid.
    pub(crate) fn doc_set_caret(&mut self, at: usize, cx: &mut Context<Self>) {
        self.touch_caret();
        self.flush_title(cx);
        if let Some(rel) = self.doc_rel()
            && let Some(buffer) = self.buffers.get(&rel).cloned()
        {
            buffer.update(cx, |b, _| b.move_caret(at, false));
        }
        cx.notify();
    }

    /// The local picture on the caret's line, when that line is an image.
    fn image_under_caret(&mut self, rel: &Path, cx: &App) -> Option<PathBuf> {
        let caret = self.buffers.get(rel)?.read(cx).caret();
        let model = self.doc_model(rel, cx)?;
        let ix = row_at(&model.rows, caret);
        model
            .images
            .get(ix)?
            .as_ref()?
            .iter()
            .find_map(|info| match &info.src {
                ImageSrc::Path(path) if path.is_file() => Some(path.clone()),
                _ => None,
            })
    }

    /// Copy a picture file to the system clipboard and say how it went.
    pub(crate) fn copy_image_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.notice = Some(match crate::clipboard::copy_image(path, cx) {
            Ok(()) => "Image copied".to_string(),
            Err(e) => e,
        });
        cx.notify();
    }

    /// Save a pasted picture as `attachments/pasted-<time>.<ext>` next to the
    /// note and insert `![](…)` on its own line. Returns whether the note
    /// changed.
    fn paste_image(
        &mut self,
        rel: &Path,
        buffer: &Entity<OpenBuffer>,
        image: Image,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(root) = self.explorer.root().map(Path::to_path_buf) else {
            return false;
        };
        let folder = crate::theme::cfg(cx)
            .str("attachments_folder")
            .trim_matches('/')
            .to_string();
        let dir = rel.parent().unwrap_or(Path::new("")).join(&folder);
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let stamp = pasted_stamp(secs);
        let ext = image_extension(image.format);
        let mut name = format!("Pasted image {stamp}.{ext}");
        let mut n = 1;
        while root.join(&dir).join(&name).exists() {
            n += 1;
            name = format!("Pasted image {stamp}-{n}.{ext}");
        }
        let saved = std::fs::create_dir_all(root.join(&dir))
            .and_then(|()| std::fs::write(root.join(&dir).join(&name), &image.bytes));
        if let Err(e) = saved {
            self.notice = Some(format!("Could not save the pasted image: {e}"));
            return false;
        }
        Self::insert_embed(buffer, &name, cx);
        self.notice = Some(format!("Saved {}", dir.join(&name).display()));
        true
    }

    /// Put `![[name]]` on its own line at the caret (images render when they
    /// are alone on their line).
    fn insert_embed(buffer: &Entity<OpenBuffer>, name: &str, cx: &mut Context<Self>) {
        let link = format!("![[{name}]]");
        buffer.update(cx, |b, _| {
            let (line_start, line) = (b.line_start_of_caret(), b.caret_line_text());
            let text = if line.trim().is_empty() && b.caret() == line_start + line.len() {
                format!("{link}\n")
            } else {
                format!("\n{link}\n")
            };
            b.insert(&text);
        });
    }

    /// Files dropped on the window: a folder opens as the mekuto, a note
    /// opens (copied into the mekuto first when it lives elsewhere), an image
    /// is copied into `attachments/` and embedded in the open note.
    pub(crate) fn on_drop_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        for path in paths {
            if path.is_dir() {
                self.open_mekuto(path, cx);
                return;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            let Some(root) = self.explorer.root().map(Path::to_path_buf) else {
                self.notice = Some("Open a folder first, then drop notes and images".into());
                continue;
            };
            if meku_vault::is_note(path) {
                let rel = match path.strip_prefix(&root) {
                    Ok(rel) => rel.to_path_buf(),
                    Err(_) => {
                        let name = path.file_name().map(PathBuf::from).unwrap_or_default();
                        let mut target = name.clone();
                        let mut n = 1;
                        while root.join(&target).exists() {
                            n += 1;
                            target = PathBuf::from(format!(
                                "{} {n}.{ext}",
                                name.file_stem().and_then(|s| s.to_str()).unwrap_or("note")
                            ));
                        }
                        if std::fs::copy(path, root.join(&target)).is_err() {
                            self.notice = Some(format!("Could not copy {}", path.display()));
                            continue;
                        }
                        self.explorer.refresh();
                        self.notice =
                            Some(format!("Copied into the mekuto as {}", target.display()));
                        target
                    }
                };
                self.open_file(&rel, cx);
            } else if matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif"
            ) {
                let (Some(rel), Some(name)) = (
                    self.doc_rel(),
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .map(str::to_string),
                ) else {
                    self.notice = Some("Open a note first, then drop the image into it".into());
                    continue;
                };
                let Some(buffer) = self.buffers.get(&rel).cloned() else {
                    continue;
                };
                let folder = crate::theme::cfg(cx)
                    .str("attachments_folder")
                    .trim_matches('/')
                    .to_string();
                let dir = rel.parent().unwrap_or(Path::new("")).join(folder);
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
                let mut file = name.clone();
                let mut n = 1;
                while root.join(&dir).join(&file).exists() {
                    n += 1;
                    file = format!("{stem} {n}.{ext}");
                }
                let copied = std::fs::create_dir_all(root.join(&dir))
                    .and_then(|()| std::fs::copy(path, root.join(&dir).join(&file)).map(|_| ()));
                match copied {
                    Ok(()) => {
                        Self::insert_embed(&buffer, &file, cx);
                        self.doc_note_edit(&rel);
                        self.notice = Some(format!("Added {file}"));
                    }
                    Err(e) => self.notice = Some(format!("Could not add the image: {e}")),
                }
            }
        }
        cx.notify();
    }

    /// Scroll the row holding `offset` into view without moving the caret.
    pub(crate) fn scroll_to_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        let Some(rel) = self.doc_rel() else {
            return;
        };
        if let Some(model) = self.doc_model(&rel, cx) {
            self.doc_list
                .scroll_to_reveal_item(row_at(&model.rows, offset));
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

/// The note's inline title: large, bold, borderless, flush with the body
/// text. Editing it renames the file (see `MekuShell::commit_title`).
fn title_element(input: &Entity<InputState>, base: Pixels) -> AnyElement {
    let size = base * 2.0;
    // `Input` sizes itself from a small preset (24px tall for custom sizes),
    // which clips a 32px title; refine its style directly instead.
    let field = Input::new(input)
        .appearance(false)
        .bordered(false)
        .focus_bordered(false);
    let field = gpui_kit::Styled::h(field, size * 1.45)
        .text_size(size)
        .line_height(relative(1.3))
        .font_weight(FontWeight::BOLD);
    div()
        .id("doc-title")
        .test_support()
        // The title field owns its presses; they are not the note's margin.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .w_full()
        .mb(base * 0.5)
        // The input pads 8px on each side; pull it back so the title text
        // lines up with the document column.
        .ml(px(-8.))
        .child(field)
        .into_any_element()
}

/// Reading column width in em of the body size: 68 characters at roughly
/// 0.55em per average character.
/// Reading column width in em for the `column_width` setting.
fn column_em(cx: &App) -> f32 {
    match crate::theme::cfg(cx).str("column_width") {
        "narrow" => 30.0,
        "wide" => 48.0,
        "full" => 400.0,
        _ => COLUMN_EM,
    }
}

const COLUMN_EM: f32 = 37.5;

/// The document list as the scroll bar sees it. Row heights are estimates
/// until drawn, so a thumb dragged to the very end maps to a point short of
/// the real end; reaching the end of the bar goes to the last row instead.
#[derive(Clone)]
pub(crate) struct DocBar(pub(crate) ListState);

impl ScrollbarHandle for DocBar {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.viewport_bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        ScrollbarHandle::offset(&self.0)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let room = self.0.max_offset_for_scrollbar().y;
        if room > px(0.) && -offset.y >= room - px(2.) {
            self.0
                .scroll_to_reveal_item(self.0.item_count().saturating_sub(1));
        } else {
            ScrollbarHandle::set_offset(&self.0, offset);
        }
    }

    fn content_size(&self) -> Size<Pixels> {
        ScrollbarHandle::content_size(&self.0)
    }

    fn start_drag(&self) {
        ScrollbarHandle::start_drag(&self.0);
    }

    fn end_drag(&self) {
        ScrollbarHandle::end_drag(&self.0);
    }
}

/// Rough height of an average row, in pixels, for the scroll bar's first
/// guess (measured on typical notes: blank 0.67 lines, text 1, items 1.15,
/// headings 2 to 2.4).
fn estimated_row_height(rows: &[Row], line_h: f32) -> f32 {
    let lines: f32 = rows
        .iter()
        .map(|r| match r.kind {
            RowKind::Blank => 0.67,
            RowKind::Heading(1 | 2) => 2.4,
            RowKind::Heading(3) => 2.0,
            RowKind::Heading(_) => 1.6,
            RowKind::Item => 1.15,
            _ => 1.0,
        })
        .sum();
    (lines / rows.len().max(1) as f32 * line_h).max(1.0)
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ImageSrc {
    Url(String),
    Path(PathBuf),
}

/// An image row: where it comes from, its caption, an explicit width
/// (`![alt|320](..)`) and, for local files, its real pixel size.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImageInfo {
    /// Absolute source range of this image's `![..]` token.
    pub range: Range<usize>,
    pub src: ImageSrc,
    pub caption: String,
    pub width: Option<u32>,
    pub dims: Option<(u32, u32)>,
}

/// Pixel size of a local image from its header (no decoding), cached by
/// modification time.
pub(crate) type DimCache = HashMap<PathBuf, (std::time::SystemTime, (u32, u32))>;

fn probe_dims(cache: &mut DimCache, path: &Path) -> Option<(u32, u32)> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    if let Some((at, dims)) = cache.get(path)
        && *at == mtime
    {
        return Some(*dims);
    }
    let size = imagesize::size(path).ok()?;
    let dims = (size.width as u32, size.height as u32);
    cache.insert(path.to_path_buf(), (mtime, dims));
    Some(dims)
}

/// The images of a row that holds nothing else (`![](a.png)`, Obsidian
/// `![[b.png|300]]`, any number side by side): targeted while the row is
/// not being edited as source. `resolve` finds a vault file by name.
fn image_source_of(
    row: &Row,
    text: &str,
    root: &Path,
    note_rel: &Path,
    dims: &mut DimCache,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<Vec<ImageInfo>> {
    if row.kind != RowKind::Text {
        return None;
    }
    let line = &text[row.src.clone()];
    let refs = meku_markdown::live::image_line(line)?;
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let range = row.src.start + r.range.start..row.src.start + r.range.end;
        let target = r.target.as_str();
        let remote = target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("data:");
        let src = if remote {
            ImageSrc::Url(target.to_string())
        } else if r.wiki {
            // Missing files still get a (broken) slot so the embed is visible.
            let path = resolve(target).unwrap_or_else(|| root.join(".missing").join(target));
            ImageSrc::Path(path)
        } else {
            let decoded = meku_markdown::live::percent_decode(target);
            let note_dir = note_rel.parent().unwrap_or(Path::new(""));
            // Resolve `..` lexically and refuse anything that climbs out of the mekuto.
            let mut parts: Vec<std::ffi::OsString> =
                note_dir.iter().map(|c| c.to_os_string()).collect();
            let mut escapes = false;
            for comp in decoded.split('/') {
                match comp {
                    "" | "." => {}
                    ".." => escapes |= parts.pop().is_none(),
                    c => parts.push(c.into()),
                }
            }
            if escapes {
                continue;
            }
            ImageSrc::Path(root.join(parts.iter().collect::<PathBuf>()))
        };
        let dims = match &src {
            ImageSrc::Path(p) => probe_dims(dims, p),
            ImageSrc::Url(_) => None,
        };
        out.push(ImageInfo {
            range,
            src,
            caption: r.alt,
            width: r.width,
            dims,
        });
    }
    (!out.is_empty()).then_some(out)
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
        style.font_family = cx.theme().mono_font_family.clone();
        style.color = hex(crate::theme::active(cx).code);
    }
    if bits.contains(InlineStyle::HIGHLIGHT) {
        // Highlighter yellow: a bright one on light themes, a warm gold on dark.
        let mut mark = if crate::theme::active(cx).dark {
            hex(0xd9a520)
        } else {
            hex(0xffd84d)
        };
        mark.a = if crate::theme::active(cx).dark {
            0.38
        } else {
            0.55
        };
        style.background_color = Some(mark);
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
    let p = crate::theme::active(cx);
    match token {
        Tok::Comment => hex(p.comment),
        Tok::Str => hex(p.string),
        Tok::Num => hex(p.number),
        Tok::Const => hex(p.constant),
        Tok::Prop | Tok::Op => hex(p.prop),
        Tok::Func => hex(p.func),
        Tok::Type => hex(p.ty),
        Tok::Keyword => hex(p.keyword),
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
    code_row: bool,
    find: &[(usize, usize, bool)],
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
    for (a, b, _) in find {
        cuts.push((*a).min(len));
        cuts.push((*b).min(len));
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
                let keep = style.color;
                apply_inline(&mut style, run.style, cx);
                if code_row {
                    // Fenced code is plain text with its own syntax colors, not
                    // inline-code coloring.
                    style.color = keep;
                }
                if run.markup {
                    style.color = cx.theme().muted_foreground;
                }
            }
        }
        if let Some((_, _, current)) = find.iter().find(|(s, e, _)| *s <= a && a < *e) {
            style.background_color = Some(crate::theme::accent_alpha(
                cx,
                if *current { 0.55 } else { 0.28 },
            ));
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
    wrap_width: Option<Pixels>,
) -> Option<(Pixels, Pixels, Pixels)> {
    let mut ix = index.min(text.len());
    while ix > 0 && !text.is_char_boundary(ix) {
        ix -= 1;
    }
    let line_height = window
        .text_style()
        .line_height
        .to_pixels(font_size.into(), window.rem_size());
    // Shape with the width the row actually wraps at, so the caret lands on
    // the right visual line of a long, soft-wrapped paragraph.
    let lines = window
        .text_system()
        .shape_text(text.to_string().into(), font_size, runs, wrap_width, None)
        .ok()?;
    let line = lines.first()?;
    let at = line.position_for_index(ix, line_height)?;
    let font_box = line.ascent() + line.descent();
    let y_offset = at.y + (line_height - font_box) / 2.0;
    Some((at.x, y_offset, font_box))
}

/// Caret as a plain element inside the row: `x`/`y` are row-local (the text
/// origin plus the shaped advance), so no window-space math is needed and the
/// caret cannot drift away from its row. Deliberately not a `canvas`
/// paint-quad: paint bounds live in window space and got the caret lost.
fn caret_element(
    x: Pixels,
    y: Pixels,
    height: Pixels,
    font_size: Pixels,
    color: Hsla,
) -> impl IntoElement {
    // max(1.5px, 0.125em), snapped to whole pixels.
    let width = px((f32::from(font_size) * 0.125).max(1.5).round());
    div()
        .id("doc-caret")
        .test_support()
        .absolute()
        .left(x)
        .top(y)
        .w(width)
        .h(height)
        .bg(color)
}

/// Caret math that follows what is drawn. A rendered (non-raw) row hides
/// markup, so source offsets inside hidden markers are not places the caret
/// can be; these helpers map through the row's display projection.
struct Visible {
    display: Display,
}

impl Visible {
    /// The rendered projection of row `ix`, or `None` when the row is shown
    /// raw (then source and visible positions coincide).
    fn of(model: &DocModel, ix: usize, revealed: bool) -> Option<Self> {
        let row = &model.rows[ix];
        (!revealed && row.kind != RowKind::Code).then(|| Self {
            display: project(row, &model.text, false),
        })
    }

    /// The nearest place the caret can actually be drawn.
    fn canon(&self, src: usize) -> usize {
        self.display.to_src(self.display.to_disp(src))
    }

    /// Next visible caret stop left/right of `from` within the row, or `None`
    /// at the row's edge (the caller then crosses to the neighbouring row).
    fn step(&self, text: &str, row: &Range<usize>, from: usize, forward: bool) -> Option<usize> {
        let start = self.canon(from);
        let mut p = from;
        loop {
            if forward {
                if p >= row.end {
                    return None;
                }
                p += text[p..].chars().next().map_or(1, char::len_utf8);
            } else {
                if p <= row.start {
                    return None;
                }
                p -= text[..p].chars().next_back().map_or(1, char::len_utf8);
            }
            let c = self.canon(p);
            if (forward && c > start) || (!forward && c < start) {
                return Some(c);
            }
        }
    }

    /// Characters of rendered text left of the caret (the column Up/Down keep).
    fn column(&self, caret: usize) -> usize {
        self.display.text[..self.display.to_disp(caret).min(self.display.text.len())]
            .chars()
            .count()
    }

    /// Source offset of rendered-text column `col`.
    fn offset(&self, col: usize) -> usize {
        let at = self
            .display
            .text
            .char_indices()
            .nth(col)
            .map_or(self.display.text.len(), |(i, _)| i);
        self.display.to_src(at)
    }

    /// Source range to delete for the rendered character before / at the
    /// caret. A character that stands for a hidden marker (a bullet glyph)
    /// takes the whole marker with it.
    fn char_range(&self, caret: usize, backward: bool) -> Option<Range<usize>> {
        let d = &self.display;
        let at = d.to_disp(self.canon(caret));
        let ch_start = if backward {
            d.text[..at].char_indices().next_back()?.0
        } else {
            if at >= d.text.len() {
                return None;
            }
            at
        };
        let ch_len = d.text[ch_start..].chars().next()?.len_utf8();
        let piece = d
            .pieces
            .iter()
            .find(|p| !p.disp.is_empty() && p.disp.start <= ch_start && ch_start < p.disp.end)?;
        Some(if piece.disp.len() == piece.src.len() {
            let start = piece.src.start + (ch_start - piece.disp.start);
            start..start + ch_len
        } else {
            piece.src.clone()
        })
    }
}

/// Whether row `ix` shows its raw markdown. Edit shows everything and Read
/// nothing. In Live the editing line stays *rendered* (bullets, checkboxes,
/// bold, links) so typing feels like a word processor; raw source appears only
/// where the user is working on the markup itself: lines touched by a
/// selection, headings (their `#`s), table rows, rules, and fence lines.
/// When Live view shows raw markdown (the `reveal_markdown` setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reveal {
    /// Only on selected text (and structural rows under the caret).
    Selection,
    /// The whole caret line too.
    CaretLine,
    /// Never for text; headings/tables/rules/fences still show under the caret.
    Never,
}

impl Reveal {
    pub(crate) fn of(cx: &App) -> Self {
        match crate::theme::cfg(cx).str("reveal_markdown") {
            "caret-line" => Reveal::CaretLine,
            "never" => Reveal::Never,
            _ => Reveal::Selection,
        }
    }
}

#[cfg(test)]
pub(crate) fn row_is_revealed(
    mode: Mode,
    row: &Row,
    ix: usize,
    caret_row: usize,
    selection: &Range<usize>,
) -> bool {
    row_is_revealed_with(Reveal::Selection, mode, row, ix, caret_row, selection)
}

pub(crate) fn row_is_revealed_with(
    reveal: Reveal,
    mode: Mode,
    row: &Row,
    ix: usize,
    caret_row: usize,
    selection: &Range<usize>,
) -> bool {
    match mode {
        Mode::Edit => true,
        Mode::Read => false,
        Mode::Live => {
            let selected = selection.start < selection.end
                && selection.end > row.src.start
                && selection.start <= row.src.end
                && !(selection.start == row.src.end && row.src.end > row.src.start);
            let structural = matches!(
                row.kind,
                RowKind::Heading(_) | RowKind::Table | RowKind::Rule
            ) || row.fence.is_some();
            let selected = selected && reveal != Reveal::Never;
            selected || (ix == caret_row && (structural || reveal == Reveal::CaretLine))
        }
    }
}

/// Short fade when a note becomes the visible one (tab switch, open).
trait OpenFade: Sized {
    fn pipe_fade(self, note: EntityId, cx: &App) -> AnyElement;
}

impl<E: IntoElement + Styled + 'static> OpenFade for E {
    fn pipe_fade(self, note: EntityId, cx: &App) -> AnyElement {
        // Keyed by the note's buffer, which survives a rename, so renaming
        // the open note does not replay the fade.
        let id = SharedString::from(format!("doc-open-{}", note.as_u64()));
        crate::theme::fade_in(self, id, 160, 0.0, cx)
    }
}

/// `YYYYMMDDHHMMSS` (UTC) for Obsidian-style `Pasted image …` file names.
pub(crate) fn pasted_stamp(unix_secs: u64) -> String {
    let (days, rem) = (unix_secs / 86_400, unix_secs % 86_400);
    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// The first image on the clipboard, if any.
fn pasted_image(item: &ClipboardItem) -> Option<Image> {
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(image.clone()),
        _ => None,
    })
}

fn image_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg",
        ImageFormat::Bmp => "bmp",
        _ => "png",
    }
}

/// Background of fenced code blocks.
fn code_bg(cx: &App) -> Hsla {
    cx.theme().sidebar
}

/// Source text between the fence lines of the block opened at row `ix`.
fn fence_block_text(ix: usize, rows: &[Row], text: &str) -> String {
    let mut end = ix + 1;
    while end < rows.len() && !rows[end].fence.as_ref().is_some_and(|f| !f.open) {
        end += 1;
    }
    if end == ix + 1 {
        return String::new();
    }
    text[rows[ix + 1].src.start..rows[end - 1].src.end].to_string()
}

/// A hidden fence line: the top one becomes a header strip with the language
/// and a copy button, the bottom one a closing strip, so the block reads as
/// one rounded container.
#[allow(clippy::too_many_arguments)]
fn fence_row_element(
    row_div: impl ParentElement + Styled + IntoElement,
    ix: usize,
    fence: &Fence,
    rows: &[Row],
    text: &str,
    copied: bool,
    shell: &Entity<MekuShell>,
    cx: &App,
) -> AnyElement {
    let bg = code_bg(cx);
    if !fence.open {
        return row_div
            .h(px(10.))
            .bg(bg)
            .rounded_bl(px(8.))
            .rounded_br(px(8.))
            .into_any_element();
    }
    let code = fence_block_text(ix, rows, text);
    let theme_colors = cx.theme();
    let (fg, muted) = (theme_colors.foreground, theme_colors.muted_foreground);
    let (hover_bg, success) = (theme_colors.list_hover, theme_colors.success);
    let shell = shell.clone();
    let lang = fence.lang.clone();
    row_div
        .h(px(32.))
        .bg(bg)
        .rounded_tl(px(8.))
        .rounded_tr(px(8.))
        .px(px(14.))
        .child(
            h_flex()
                .size_full()
                .justify_end()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(lang),
                )
                .child(
                    // Presses here belong to the button, not the caret.
                    div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            h_flex()
                                .id(("copy-code", ix))
                                .test_support()
                                .gap_1p5()
                                .h(px(24.))
                                .px_2()
                                .items_center()
                                .rounded_md()
                                .cursor_pointer()
                                .text_xs()
                                .text_color(if copied { success } else { muted })
                                .hover(move |s| s.bg(hover_bg).text_color(fg))
                                .child(icon(
                                    if copied { Ico::Check } else { Ico::Copy },
                                    px(14.),
                                    if copied { success } else { muted },
                                ))
                                .child(if copied { "Copied" } else { "Copy" })
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                                    shell.update(cx, |view, cx| {
                                        view.copied = Some((ix, Instant::now()));
                                        cx.notify();
                                    });
                                }),
                        ),
                ),
        )
        .into_any_element()
}

/// Accent of a callout family, from the active theme.
fn callout_color(kind: CalloutKind, cx: &App) -> Hsla {
    let theme = cx.theme();
    match kind {
        CalloutKind::Note => crate::theme::accent_alpha(cx, 1.0),
        CalloutKind::Info => theme.info,
        CalloutKind::Tip | CalloutKind::Success => theme.success,
        CalloutKind::Warning => theme.warning,
        // Own hues from the theme's syntax colours, so each type reads apart
        // even where the status colours coincide.
        CalloutKind::Question => hex(crate::theme::active(cx).prop),
        CalloutKind::Danger => theme.danger,
        CalloutKind::Bug => hex(crate::theme::active(cx).constant),
        CalloutKind::Example => hex(crate::theme::active(cx).keyword),
        CalloutKind::Quote => theme.muted_foreground,
    }
}

fn callout_icon(kind: CalloutKind) -> Ico {
    match kind {
        CalloutKind::Note => Ico::Notes,
        CalloutKind::Info => Ico::Info,
        CalloutKind::Tip => Ico::Idea,
        CalloutKind::Success => Ico::CheckCircle,
        CalloutKind::Question => Ico::HelpCircle,
        CalloutKind::Warning => Ico::Warning,
        CalloutKind::Danger => Ico::CloseCircle,
        CalloutKind::Bug => Ico::Bug,
        CalloutKind::Example => Ico::Terminal,
        CalloutKind::Quote => Ico::Quote,
    }
}

/// Column widths per table (keyed by its header row), valid for one document
/// version.
pub(crate) type TableCache = Rc<RefCell<(u64, HashMap<usize, Rc<Vec<Pixels>>>)>>;

/// Hidden-mode display of one table cell: its inline markup rendered.
fn cell_display(cell: &str) -> Display {
    let rows = build_rows(cell);
    project(&rows[0], cell, false)
}

/// Widest cell of every column of the table starting at row `start`.
fn table_col_widths(
    start: usize,
    rows: &[Row],
    text: &str,
    window: &mut Window,
    cx: &App,
) -> Vec<Pixels> {
    let size = window.text_style().font_size.to_pixels(window.rem_size());
    let mut widths: Vec<Pixels> = Vec::new();
    for row in rows[start..]
        .iter()
        .take_while(|r| r.table.as_ref().is_some_and(|t| t.start == start))
    {
        let Some(t) = row.table.as_ref() else {
            continue;
        };
        if t.role == TableRole::Delimiter {
            continue;
        }
        let mut style = window.text_style().clone();
        if t.role == TableRole::Header {
            style.font_weight = FontWeight::BOLD;
        }
        for (c, range) in t.cells.iter().enumerate() {
            let display = cell_display(&text[range.clone()]);
            let width = if display.text.is_empty() {
                px(0.)
            } else {
                let runs = text_runs(&display, &style, None, &[], false, &[], cx);
                window
                    .text_system()
                    .shape_line(display.text.clone().into(), size, &runs, None)
                    .width
            };
            if widths.len() <= c {
                widths.resize(c + 1, px(0.));
            }
            widths[c] = widths[c].max(width);
        }
    }
    // Keep one enormous cell from pushing the grid off the page.
    widths
        .iter()
        .map(|w| (*w).clamp(px(24.), px(360.)))
        .collect()
}

/// A hidden (non-caret) table line drawn as aligned cells.
#[allow(clippy::too_many_arguments)]
fn table_row_element(
    row_div: impl ParentElement + Styled + IntoElement,
    row: &Row,
    next_in_table: bool,
    text: &str,
    widths: &[Pixels],
    shell: &Entity<MekuShell>,
    window: &mut Window,
    cx: &App,
) -> AnyElement {
    let Some(t) = row.table.as_ref() else {
        return row_div.into_any_element();
    };
    let pad_x = px(10.);
    let total = widths.iter().fold(px(0.), |acc, w| acc + *w + pad_x * 2.0);
    if t.role == TableRole::Delimiter {
        // The header rule: the delimiter line itself is never shown.
        return row_div
            .h(px(1.))
            .child(div().w(total).h(px(1.)).bg(cx.theme().border))
            .into_any_element();
    }
    let mut style = window.text_style().clone();
    if t.role == TableRole::Header {
        style.font_weight = FontWeight::BOLD;
    }
    let size = window.text_style().font_size.to_pixels(window.rem_size());
    let mut cells = h_flex().w(total);
    if t.role == TableRole::Body {
        let mut hairline = cx.theme().border;
        hairline.a = 0.6;
        if next_in_table {
            cells = cells.border_b_1().border_color(hairline);
        }
    }
    for (c, width) in widths.iter().enumerate() {
        let range = t.cells.get(c);
        let display = range.map(|r| cell_display(&text[r.clone()]));
        let mut cell = div()
            .w(*width + pad_x * 2.0)
            .px(pad_x)
            .py(px(6.))
            .min_h(size * 1.6)
            .flex();
        cell = match t.aligns.get(c) {
            Some(Align::Right) => cell.justify_end(),
            Some(Align::Center) => cell.justify_center(),
            _ => cell,
        };
        if let Some(display) = display {
            let runs = text_runs(&display, &style, None, &[], false, &[], cx);
            if !display.text.is_empty() {
                cell = cell.child(StyledText::new(display.text.clone()).with_runs(runs));
            }
        }
        if let Some(range) = range {
            let at = range.start;
            let shell = shell.clone();
            cell = cell.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                shell.update(cx, |view, cx| {
                    let focus = view.focus_handle.clone();
                    window.focus(&focus, cx);
                    view.doc_set_caret(at, cx);
                });
            });
        }
        cells = cells.child(cell);
    }
    row_div.child(cells).into_any_element()
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
    image: Option<Vec<ImageInfo>>,
    pointer: bool,
    find_hits: &[(Range<usize>, bool)],
    col_width: Pixels,
    rows: &[Row],
    tables: &TableCache,
    version: u64,
    copied: bool,
    caret_on: bool,
    slash: Option<&crate::slash::SlashView>,
    shell: &Entity<MekuShell>,
    pictures: &Entity<RetainAllImageCache>,
    views: &RowViews,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let base_font = window.text_style().font_size.to_pixels(window.rem_size());
    let down_shell = shell.clone();
    let drag_shell = shell.clone();
    let menu_shell = shell.clone();
    let row_div = div()
        .id(("doc-row", ix))
        .test_support()
        .relative()
        .w_full()
        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
            // The row owns the press; the margins and the empty space around
            // the text handle their own.
            cx.stop_propagation();
            down_shell.update(cx, |view, cx| {
                let focus = view.focus_handle.clone();
                window.focus(&focus, cx);
                if view.doc_follow_link(ix, ev.position, ev.modifiers.control, cx) {
                    return;
                }
                view.doc_pointer(ix, ev.position, ev.modifiers.shift, ev.click_count, cx);
            });
        })
        .on_mouse_down(
            MouseButton::Right,
            move |ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                menu_shell.update(cx, |view, cx| {
                    view.open_doc_menu(Target::Text(Some(ix)), ev.position, window, cx)
                });
            },
        )
        .on_mouse_move(move |ev: &MouseMoveEvent, _window, cx| {
            if ev.pressed_button == Some(MouseButton::Left) {
                drag_shell.update(cx, |view, cx| view.doc_drag(ix, ev.position, cx));
            } else {
                drag_shell.update(cx, |view, cx| {
                    view.doc_hover(ix, ev.position, ev.modifiers.control, cx)
                });
            }
        });
    let row_div = if pointer {
        row_div.cursor_pointer()
    } else {
        row_div
    };

    // Standalone image rows draw their picture when hidden; the caret row and
    // Edit mode show the raw text instead (unchanged flow below).
    if !reveal && let Some(images) = image {
        if let Some(slot) = views.borrow_mut().get_mut(ix) {
            *slot = None;
        }
        let inset = px(20.0 * f32::from(row.list_depth.min(8)));
        let avail = if col_width > px(0.) {
            col_width - inset
        } else {
            px(560.)
        };
        return row_div
            .pl(inset)
            .py(px(6.))
            .child(image_row_element(
                images,
                ix,
                caret.is_some(),
                avail,
                shell,
                pictures,
                cx,
            ))
            .into_any_element();
    }

    if !reveal && let Some(fence) = &row.fence {
        if let Some(slot) = views.borrow_mut().get_mut(ix) {
            *slot = None;
        }
        return fence_row_element(row_div, ix, fence, rows, text, copied, shell, cx);
    }

    if !reveal && let Some(table) = &row.table {
        if let Some(slot) = views.borrow_mut().get_mut(ix) {
            *slot = None;
        }
        let widths = {
            let mut cache = tables.borrow_mut();
            if cache.0 != version {
                *cache = (version, HashMap::new());
            }
            cache
                .1
                .entry(table.start)
                .or_insert_with(|| Rc::new(table_col_widths(table.start, rows, text, window, cx)))
                .clone()
        };
        let next_in_table = rows
            .get(ix + 1)
            .and_then(|r| r.table.as_ref())
            .is_some_and(|t| t.start == table.start);
        return table_row_element(
            row_div,
            row,
            next_in_table,
            text,
            &widths,
            shell,
            window,
            cx,
        );
    }

    if row.kind == RowKind::Blank {
        if let Some(slot) = views.borrow_mut().get_mut(ix) {
            *slot = None;
        }
        // The line the caret sits on is as tall as a text line, so typing the
        // first character does not make the row (and everything below) jump.
        let line_h = window.text_style().line_height_in_pixels(window.rem_size());
        let row_h = if caret.is_some() {
            line_h.max(base_font * 1.1)
        } else {
            base_font * 1.1
        };
        let mut el = row_div.h(row_h).text_size(base_font);
        // A brand-new note says how to begin.
        if text.is_empty() && ix == 0 {
            el = el.child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .text_color(cx.theme().muted_foreground)
                    .child("Start writing, or type / for blocks"),
            );
        }
        if caret.is_some() && caret_on {
            // Same caret box a text row gets: shape one space so the height
            // and vertical offset match the glyphs typed here next.
            let style = window.text_style();
            let runs = [style.to_run(1)];
            let (y, h) = caret_geometry(window, " ", &runs, base_font, 0, None)
                .map_or(((row_h - base_font) / 2.0, base_font), |(_, y, h)| (y, h));
            el = el.child(caret_element(px(0.), y, h, base_font, cx.theme().caret));
        }
        return el.into_any_element();
    }
    if row.kind == RowKind::Rule && !reveal {
        return row_div
            .py(base_font * 0.9)
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
    if matches!(row.kind, RowKind::Code | RowKind::Table) {
        base.font_family = cx.theme().mono_font_family.clone();
    }
    // Callouts draw only while their line is not being edited.
    let callout = row.callout.filter(|_| !reveal);
    if let Some(c) = callout
        && c.role == CalloutRole::Title
    {
        base.font_weight = FontWeight::BOLD;
        base.color = callout_color(c.kind, cx);
    }
    // Find matches on this row, in display coordinates.
    let find_disp: Vec<(usize, usize, bool)> = find_hits
        .iter()
        .map(|(r, cur)| {
            (
                display.to_disp(r.start.max(row.src.start)),
                display.to_disp(r.end.min(row.src.end)),
                *cur,
            )
        })
        .collect();
    let runs = text_runs(
        &display,
        &base,
        sel_disp,
        colored.as_deref().map_or(&[], |c| c.as_slice()),
        row.kind == RowKind::Code,
        &find_disp,
        cx,
    );
    let font_size = base_font * heading_scale(row.kind);
    let display_empty = display.text.is_empty();
    // Kept for caret shaping below, once this row's padding is known.
    let caret_shape = caret_disp.map(|index| (display.text.clone(), runs.clone(), index));
    let styled = StyledText::new(display.text.clone()).with_runs(runs);
    let layout = styled.layout().clone();
    let painted = Rc::new(Cell::new(false));
    if let Some(slot) = views.borrow_mut().get_mut(ix) {
        *slot = Some(RowView {
            layout: layout.clone(),
            display,
            has_checkbox: !reveal && row.task().is_some(),
            painted: painted.clone(),
        });
    }

    let mut el = row_div.text_size(font_size);
    // Text origin inside the row: mirrors the padding applied below so the
    // caret (row-local) lands exactly where the glyphs start.
    let mut text_x = px(0.);
    let mut text_y = px(0.);
    if let RowKind::Heading(level) = row.kind {
        // Space above 1.6em (H1-2) / 1.3em (H3+) and below 0.5em / 0.4em of
        // the heading size; the blank row before it already supplies the
        // body-sized gap, so only the remainder is added here.
        let (above, below) = if level <= 2 { (1.6, 0.5) } else { (1.3, 0.4) };
        let pad_top = if ix == 0 {
            px(0.)
        } else {
            (font_size * above - base_font * 1.1).max(px(0.))
        };
        el = el.pt(pad_top).pb(font_size * below);
        text_y = pad_top;
    }
    if row.quote_depth > 0 && callout.is_none() {
        let depth = f32::from(row.quote_depth.min(4));
        let mut rule = cx.theme().muted_foreground;
        rule.a = 0.5;
        el = el
            .border_l_2()
            .border_color(rule)
            .pl(base_font * (1.2 * depth));
        // Later padding refinements replace earlier ones (same as the style
        // system), so track by assignment, not accumulation.
        text_x = base_font * (1.2 * depth);
    }
    if let Some(c) = callout {
        let color = callout_color(c.kind, cx);
        let mut tint = color;
        tint.a = 0.11;
        let pad = px(14.);
        el = el.bg(tint).pl(pad).pr(pad);
        text_x = pad;
        if c.first {
            el = el.pt(px(10.)).rounded_tl(px(8.)).rounded_tr(px(8.));
            text_y = px(10.);
        }
        if c.last {
            el = el.pb(px(10.)).rounded_bl(px(8.)).rounded_br(px(8.));
        }
        if c.role == CalloutRole::Title {
            let icon_size = px(16.);
            let line_h = window.text_style().line_height_in_pixels(window.rem_size());
            el = el.pl(pad + px(26.)).child(
                div()
                    .absolute()
                    .left(pad)
                    .top(text_y + ((line_h - icon_size) / 2.0).max(px(0.)))
                    .child(icon(callout_icon(c.kind), icon_size, color)),
            );
            text_x = pad + px(26.);
            // A title-less callout shows its kind as the title.
            if display_empty {
                el = el.child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_color(color)
                        .child(c.kind.default_title()),
                );
            }
        }
    }
    if !reveal && row.list_depth > 0 {
        let indent = base_font * (1.5 * f32::from(row.list_depth.min(8)));
        el = el.pl(indent);
        text_x = indent;
        // One faint guide per ancestor level makes nested items read as
        // subsections of the item above.
        for level in 1..=row.list_depth {
            el = el.child(
                div()
                    .absolute()
                    .top(px(0.))
                    .bottom(px(0.))
                    .left(base_font * (1.5 * f32::from(level - 1) + 0.3))
                    .w(px(1.))
                    .bg(cx.theme().border),
            );
        }
    }
    // An empty task row ("- [ ] " just typed) has no text to give it height;
    // without a floor the checkbox and caret overflow a zero-height row and
    // nothing shows.
    if display_empty {
        let line_h =
            window.text_style().line_height_in_pixels(window.rem_size()) * heading_scale(row.kind);
        el = el.min_h(line_h);
    }
    // Tasks: a real checkbox control in the gap before the text.
    if !reveal && let Some((checked, task_src)) = row.task() {
        let box_size = px(16.);
        let gap = base_font * 1.6;
        let line_h =
            window.text_style().line_height_in_pixels(window.rem_size()) * heading_scale(row.kind);
        let toggle_shell = shell.clone();
        let at = task_src.start;
        el = el.pl(text_x + gap).child(
            div()
                .absolute()
                .left(text_x)
                .top(text_y + ((line_h - box_size) / 2.0).max(px(0.)))
                .child(Checkbox::new(("task-box", ix)).checked(checked).on_click(
                    move |_, _, cx| {
                        toggle_shell.update(cx, |view, cx| view.toggle_task_at(at, cx));
                    },
                )),
        );
        text_x += gap;
    }
    if row.kind == RowKind::Item {
        el = el.pb(base_font * 0.25);
    }
    if row.kind == RowKind::Code {
        el = el
            .bg(code_bg(cx))
            .px(px(14.))
            .min_h(window.text_style().line_height_in_pixels(window.rem_size()));
        text_x = px(14.);
        // One rounded container whatever is revealed: the block's first and
        // last rows carry the corners (also while the fences show as source).
        let opens = row.fence.as_ref().is_some_and(|f| f.open);
        let closes = row.fence.as_ref().is_some_and(|f| !f.open)
            || rows
                .get(ix + 1)
                .is_none_or(|next| next.kind != RowKind::Code);
        if opens {
            el = el.rounded_tl(px(8.)).rounded_tr(px(8.)).min_h(px(32.));
        }
        if closes {
            el = el.rounded_bl(px(8.)).rounded_br(px(8.));
        }
    }
    // A title-less callout shows its default title instead of empty text.
    let title_less = callout.is_some_and(|c| c.role == CalloutRole::Title) && display_empty;
    if !title_less {
        el = el.child(styled).child(
            canvas(
                {
                    let painted = painted.clone();
                    move |_, _, _| painted.set(true)
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_0(),
        );
    }
    let right_pad = if row.kind == RowKind::Code {
        px(14.)
    } else {
        px(0.)
    };
    let wrap = (col_width > px(0.)).then(|| (col_width - text_x - right_pad).max(px(40.)));
    let wants_caret = caret_shape.is_some();
    let caret_geo = caret_shape
        .filter(|_| !display_empty)
        .and_then(|(text, runs, index)| {
            caret_geometry(window, &text, &runs, font_size, index, wrap)
        })
        .or_else(|| {
            // An empty line (e.g. inside a code block) has nothing to shape.
            (wants_caret && display_empty).then(|| {
                let line_h = window.text_style().line_height_in_pixels(window.rem_size());
                (px(0.), ((line_h - font_size) / 2.0).max(px(0.)), font_size)
            })
        });
    if let (Some(menu), Some((advance, y_offset, cheight))) = (slash, caret_geo) {
        // Drawn above the rows below it, anchored under the caret.
        el = el.child(
            deferred(
                div()
                    .absolute()
                    .left(text_x + advance)
                    .top(text_y + y_offset + cheight + px(6.))
                    .child(crate::slash::menu_element(menu, shell, cx)),
            )
            .with_priority(10),
        );
    }
    if let Some((advance, y_offset, cheight)) = caret_geo.filter(|_| caret_on) {
        el = el.child(caret_element(
            text_x + advance,
            text_y + y_offset,
            cheight,
            font_size,
            cx.theme().caret,
        ));
    }
    // The row that becomes the editing row eases from slightly dimmed, so
    // the markers appearing feels like a reveal rather than a jump.
    if reveal && caret.is_some() {
        return crate::theme::fade_from(el, ("row-reveal", ix), 120, 0.6, 0.0, cx);
    }
    el.into_any_element()
}

/// Frame size for an image: its natural (or `|width`-hinted) size, never wider
/// than the column, never taller than `max_h`, aspect preserved. With
/// no known size (remote images) a 16:9 box.
pub(crate) fn image_frame_size(info: &ImageInfo, avail: Pixels, max_h: f32) -> (Pixels, Pixels) {
    let avail = f32::from(avail).max(120.0);
    match info.dims {
        Some((w, h)) if w > 0 && h > 0 => {
            let natural = info.width.unwrap_or(w) as f32;
            let mut fw = natural.min(avail);
            let mut fh = fw * h as f32 / w as f32;
            if fh > max_h {
                fw *= max_h / fh;
                fh = max_h;
            }
            (px(fw), px(fh))
        }
        _ => {
            let fw = info.width.map_or(480.0, |w| w as f32).min(avail);
            (px(fw), px(fw * 9.0 / 16.0))
        }
    }
}

/// The image picture itself, `Contain`-fit into whatever box it is given,
/// with loading / unavailable states. Shared by the row and the lightbox.
fn with_cache(img: Img, cache: Option<&Entity<RetainAllImageCache>>) -> Img {
    match cache {
        Some(cache) => img.image_cache(cache),
        None => img,
    }
}

pub(crate) fn picture(
    src: &ImageSrc,
    pictures: Option<&Entity<RetainAllImageCache>>,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let note = move |text: &'static str| {
        move || {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(muted)
                .child(text)
                .into_any_element()
        }
    };
    match src {
        ImageSrc::Path(path) => with_cache(img(path.clone()), pictures)
            .size_full()
            .object_fit(ObjectFit::Contain)
            .with_fallback(note("Image unavailable"))
            .into_any_element(),
        ImageSrc::Url(url) => with_cache(img(url.clone()), pictures)
            .size_full()
            .object_fit(ObjectFit::Contain)
            .with_loading(note("Loading…"))
            .with_fallback(note("Image unavailable"))
            .into_any_element(),
    }
}

/// An image-only row: each picture framed and rounded, flowing left to right
/// and wrapping like inline images; click one to zoom it (lightbox). The
/// accent ring shows the caret is on this row.
fn image_row_element(
    images: Vec<ImageInfo>,
    ix: usize,
    focused: bool,
    avail: Pixels,
    shell: &Entity<MekuShell>,
    pictures: &Entity<RetainAllImageCache>,
    cx: &App,
) -> AnyElement {
    let theme_colors = cx.theme();
    let border = theme_colors.border;
    let muted = theme_colors.muted_foreground;
    let ring = crate::theme::accent_alpha(cx, 0.9);
    let hover_border = ring;
    let mut flow = h_flex().flex_wrap().gap(px(10.)).items_start();
    let max_h = crate::theme::cfg(cx).f64("image_max_height") as f32;
    let zoomable = crate::theme::cfg(cx).bool("image_click_zoom");
    for (k, info) in images.into_iter().enumerate() {
        let (fw, fh) = image_frame_size(&info, avail, max_h);
        let open = info.clone();
        let menu_info = info.clone();
        let menu_shell = shell.clone();
        let shell = shell.clone();
        let id: ElementId = if k == 0 {
            ("image-row", ix).into()
        } else {
            ("image-row-n", ix * 1000 + k).into()
        };
        let frame = div()
            .id(id)
            .test_support()
            .w(fw)
            .h(fh)
            .rounded_lg()
            .border_1()
            .border_color(if focused { ring } else { border })
            .overflow_hidden()
            .cursor_pointer()
            .hover(move |s| s.border_color(hover_border))
            // A press on the picture zooms it; it must not move the caret.
            .on_mouse_down(
                MouseButton::Right,
                move |ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let info = menu_info.clone();
                    menu_shell.update(cx, |view, cx| {
                        view.open_doc_menu(Target::Image(info), ev.position, window, cx)
                    });
                },
            )
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                if !zoomable {
                    return;
                }
                let open = open.clone();
                shell.update(cx, |view, cx| {
                    // Esc must reach the shell whatever had focus before.
                    window.focus(&view.focus_handle, cx);
                    view.open_lightbox(open, cx);
                });
            })
            .child(picture(&info.src, Some(pictures), cx));
        let mut column = v_flex().gap_1().child(frame);
        if !info.caption.is_empty() {
            column = column.child(
                div()
                    .w(fw)
                    .text_xs()
                    .text_color(muted)
                    .child(info.caption.clone()),
            );
        }
        flow = flow.child(column);
    }
    flow.into_any_element()
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
        self.touch_caret();
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
                self.update_slash(code, cx);
                cx.notify();
                return;
            }
        }
        let typing = text.chars().count() == 1 && text != "\n";
        buffer.update(cx, |b, _| b.replace(target, text, typing));
        self.doc_note_edit(&rel);
        let in_code = self
            .doc_model(&rel, cx)
            .is_some_and(|m| fence_at(&m, buffer.read(cx).caret()).0);
        self.update_slash(in_code, cx);
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
        self.touch_caret();
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
        let view = views.get(row)?.as_ref().filter(|v| v.is_painted())?;
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
