//! Quick switcher, command palette, vault search and go-to-line: one overlay
//! (input + filtered list) driven by a [`PaletteKind`].

use std::path::PathBuf;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;

use crate::icons::{Ico, icon};
use crate::theme::{self, Palette};
use crate::{MekuShell, NamingMode, OpenFolder};

actions!(
    meku_palette,
    [
        OpenFileFinder,
        OpenCommandPalette,
        OpenVaultSearch,
        OpenGoToLine,
        PaletteUp,
        PaletteDown,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteKind {
    Files,
    Commands,
    Themes,
    Search,
    GotoLine,
    Shortcuts,
    Mekutos,
}

impl PaletteKind {
    fn placeholder(self) -> &'static str {
        match self {
            PaletteKind::Files => "Open a note by name…",
            PaletteKind::Commands => "Type a command…",
            PaletteKind::Themes => "Choose a theme…",
            PaletteKind::Search => "Search every note…",
            PaletteKind::GotoLine => "Line number…",
            PaletteKind::Shortcuts => "Search shortcuts…",
            PaletteKind::Mekutos => "Switch to a mekuto…",
        }
    }
}

/// Everything the command palette can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    FileFinder,
    NewNote,
    NewFolder,
    OpenFolder,
    SwitchMekuto,
    Save,
    CloseTab,
    ToggleSidebar,
    CycleMode,
    FormatTable,
    FindInNote,
    ReplaceInNote,
    SearchVault,
    GoToLine,
    ThemePicker,
    ThemeFollowSystem,
    ThemeNext,
    ThemeUndo,
    ThemeCopy,
    ThemeAdd,
    ToggleReduceMotion,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ShortcutsHelp,
    OpenSettings,
    CheckForUpdates,
}

/// `(command, title, shortcut)` in the order shown for an empty query.
const COMMANDS: &[(Command, &str, &str)] = &[
    (Command::FileFinder, "Open note…", "Ctrl+P"),
    (Command::NewNote, "New note", "Ctrl+T"),
    (Command::NewFolder, "New folder", ""),
    (Command::SwitchMekuto, "Switch mekuto…", ""),
    (Command::OpenFolder, "Open or create a mekuto…", "Ctrl+O"),
    (Command::Save, "Save", "Ctrl+S"),
    (Command::CloseTab, "Close tab", "Ctrl+W"),
    (Command::ToggleSidebar, "Toggle sidebar", "Ctrl+\\"),
    (Command::CycleMode, "Cycle Edit / Live / Read", "Ctrl+E"),
    (Command::FormatTable, "Format table", ""),
    (Command::FindInNote, "Find in note", "Ctrl+F"),
    (Command::ReplaceInNote, "Find and replace in note", "Ctrl+H"),
    (Command::SearchVault, "Search in all notes", "Ctrl+Shift+F"),
    (Command::GoToLine, "Go to line…", "Ctrl+G"),
    (Command::ThemePicker, "Theme: choose…", ""),
    (Command::ThemeFollowSystem, "Theme: follow system", ""),
    (Command::ThemeNext, "Theme: next", "Ctrl+Alt+T"),
    (Command::ThemeUndo, "Theme: undo last change", "Ctrl+Alt+Z"),
    (Command::ThemeCopy, "Theme: copy as JSON", ""),
    (Command::ThemeAdd, "Theme: add from clipboard", ""),
    (Command::ToggleReduceMotion, "Toggle reduce motion", ""),
    (Command::ZoomIn, "Zoom in", "Ctrl +"),
    (Command::ZoomOut, "Zoom out", "Ctrl -"),
    (Command::ZoomReset, "Reset zoom", "Ctrl Alt 0"),
    (Command::OpenSettings, "Open settings", "Ctrl ,"),
    (Command::ShortcutsHelp, "Keyboard shortcuts", ""),
    (Command::CheckForUpdates, "Check for updates", ""),
];

#[derive(Clone)]
pub(crate) enum Target {
    File(PathBuf),
    /// A search hit: note and 0-based line.
    Hit(PathBuf, usize),
    Command(Command),
    Theme(&'static str),
    Mekuto(PathBuf),
    Line(usize),
    None,
}

#[derive(Clone)]
pub(crate) struct Item {
    pub label: String,
    pub detail: String,
    pub target: Target,
}

pub(crate) struct PaletteState {
    pub kind: PaletteKind,
    pub input: Entity<InputState>,
    pub _sub: Subscription,
    pub selected: usize,
    pub items: Vec<Item>,
    /// Look to restore when a theme preview is cancelled.
    pub original: Option<Palette>,
}

/// Every default shortcut, grouped by what it does: `(action, keys)`.
const SHORTCUTS: &[(&str, &str)] = &[
    ("Open note", "Ctrl P"),
    ("Command palette", "Ctrl Shift P"),
    ("Settings", "Ctrl ,"),
    ("Undo theme change", "Ctrl Alt Z"),
    ("Search all notes", "Ctrl Shift F"),
    ("Find / replace in note", "Ctrl F  ·  Ctrl H"),
    ("Go to line", "Ctrl G"),
    ("Back / forward", "Alt ←  ·  Alt →"),
    ("New note", "Ctrl T"),
    ("Close tab / next tab", "Ctrl W  ·  Ctrl Tab"),
    ("Toggle sidebar", "Ctrl \\"),
    ("Source / Live / Read", "Ctrl E"),
    ("Zoom in / out / reset", "Ctrl +  ·  Ctrl -  ·  Ctrl Alt 0"),
    ("Next theme", "Ctrl Alt T"),
    ("Bold", "Ctrl B"),
    ("Italic", "Ctrl I"),
    ("Strikethrough", "Ctrl Shift X"),
    ("Highlight", "Ctrl Shift H"),
    ("Inline code", "Ctrl `"),
    ("Insert / edit link", "Ctrl K"),
    ("Heading 1-6 / paragraph", "Ctrl 1-6  ·  Ctrl 0"),
    ("Quote", "Ctrl Alt Q"),
    (
        "Numbered / bulleted / task list",
        "Ctrl Shift 7  ·  8  ·  9",
    ),
    ("Toggle task done", "Ctrl L"),
    ("Indent / outdent item", "Tab  ·  Shift Tab"),
    ("Move line up / down", "Alt ↑  ·  Alt ↓"),
    ("Duplicate / delete line", "Ctrl Shift D  ·  Ctrl Shift K"),
    ("Join lines", "Ctrl J"),
    ("Comment (in code)", "Ctrl /"),
    ("Expand selection", "Ctrl D"),
    ("Word left / right", "Ctrl ←  ·  Ctrl →"),
    ("Block menu", "/"),
    (
        "Insert line below / above",
        "Ctrl Enter  ·  Ctrl Shift Enter",
    ),
    ("Soft line break", "Shift Enter"),
    ("Undo / redo", "Ctrl Z  ·  Ctrl Y"),
];

const MAX_ITEMS: usize = 10;

/// How many rows a palette shows: every remembered mekuto (the list scrolls),
/// a few more shortcuts, and a short list otherwise.
fn row_limit(kind: PaletteKind) -> usize {
    match kind {
        PaletteKind::Mekutos => usize::MAX,
        PaletteKind::Shortcuts => 14,
        _ => MAX_ITEMS,
    }
}

impl MekuShell {
    pub(crate) fn palette_open(&self) -> bool {
        self.palette.is_some() || self.palette_request.is_some()
    }

    pub(crate) fn open_palette(&mut self, kind: PaletteKind, cx: &mut Context<Self>) {
        self.naming = None;
        self.naming_input = None;
        self._naming_sub = None;
        self.pending_delete = None;
        self.close_palette(true, cx);
        self.palette_request = Some(kind);
        cx.notify();
    }

    /// Create the input of a requested palette; needs a window, so it runs
    /// from `render`.
    pub(crate) fn ensure_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(command) = self.run_after.take() {
            self.run_command(command, window, cx);
        }
        let Some(kind) = self.palette_request.take() else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(kind.placeholder()));
        let sub = cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Change => this.refresh_palette(cx),
            InputEvent::PressEnter { .. } => this.palette_confirm(cx),
            _ => {}
        });
        window.focus(&input.read(cx).focus_handle(cx), cx);
        let original = (kind == PaletteKind::Themes).then(|| theme::active(cx));
        self.palette = Some(PaletteState {
            kind,
            input,
            _sub: sub,
            selected: 0,
            items: Vec::new(),
            original,
        });
        self.refresh_palette(cx);
    }

    fn palette_query(&self, cx: &App) -> String {
        self.palette
            .as_ref()
            .map(|p| p.input.read(cx).value().to_string())
            .unwrap_or_default()
    }

    fn compute_items(&self, kind: PaletteKind, query: &str, cx: &App) -> Vec<Item> {
        let q = query.trim();
        match kind {
            PaletteKind::Files => self
                .explorer
                .fuzzy_files(q, 50)
                .into_iter()
                .map(|rel| Item {
                    label: rel
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    detail: rel
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    target: Target::File(rel),
                })
                .collect(),
            PaletteKind::Commands => {
                let mut scored: Vec<(u64, usize)> = COMMANDS
                    .iter()
                    .enumerate()
                    .filter_map(|(i, (_, title, _))| {
                        meku_index::fuzzy_score(q, title).map(|s| (s, i))
                    })
                    .collect();
                if !q.is_empty() {
                    scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                }
                scored
                    .into_iter()
                    .map(|(_, i)| {
                        let (command, title, shortcut) = COMMANDS[i];
                        Item {
                            label: title.to_string(),
                            detail: shortcut.to_string(),
                            target: Target::Command(command),
                        }
                    })
                    .collect()
            }
            PaletteKind::Themes => {
                let current = theme::active(cx).name;
                theme::palettes(cx)
                    .iter()
                    .filter(|p| meku_index::fuzzy_score(q, p.name).is_some())
                    .map(|p| Item {
                        label: p.name.to_string(),
                        detail: format!(
                            "{}{}",
                            if p.dark { "dark" } else { "light" },
                            if p.name == current { "  ✓" } else { "" }
                        ),
                        target: Target::Theme(p.name),
                    })
                    .collect()
            }
            PaletteKind::Search => {
                if q.chars().count() < 2 {
                    return vec![Item {
                        label: "Type at least two characters".into(),
                        detail: String::new(),
                        target: Target::None,
                    }];
                }
                self.explorer
                    .search_text(q, 60)
                    .into_iter()
                    .map(|(rel, line, text)| Item {
                        label: text.chars().take(120).collect(),
                        detail: format!("{}:{}", rel.display(), line + 1),
                        target: Target::Hit(rel, line),
                    })
                    .collect()
            }
            PaletteKind::Mekutos => {
                let current = self.explorer.root().map(std::path::Path::to_path_buf);
                let mut items: Vec<Item> = crate::config::recent_mekutos()
                    .into_iter()
                    .filter(|dir| {
                        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                        meku_index::fuzzy_score(q, name).is_some()
                    })
                    .map(|dir| Item {
                        label: dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("mekuto")
                            .to_string(),
                        detail: if current.as_deref() == Some(dir.as_path()) {
                            "open now  ✓".to_string()
                        } else if !dir.is_dir() {
                            format!("not found · {}", dir.display())
                        } else {
                            dir.to_string_lossy().into_owned()
                        },
                        target: Target::Mekuto(dir),
                    })
                    .collect();
                items.push(Item {
                    label: "Open or create another mekuto…".into(),
                    detail: "pick a folder of notes".into(),
                    target: Target::Command(Command::OpenFolder),
                });
                items
            }
            PaletteKind::Shortcuts => {
                let mut scored: Vec<(u64, usize)> = SHORTCUTS
                    .iter()
                    .enumerate()
                    .filter_map(|(i, (action, keys))| {
                        meku_index::fuzzy_score(q, &format!("{action} {keys}")).map(|s| (s, i))
                    })
                    .collect();
                if !q.is_empty() {
                    scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                }
                scored
                    .into_iter()
                    .map(|(_, i)| Item {
                        label: SHORTCUTS[i].0.to_string(),
                        detail: SHORTCUTS[i].1.to_string(),
                        target: Target::None,
                    })
                    .collect()
            }
            PaletteKind::GotoLine => match q.parse::<usize>() {
                Ok(n) if n > 0 => vec![Item {
                    label: format!("Go to line {n}"),
                    detail: String::new(),
                    target: Target::Line(n - 1),
                }],
                _ => vec![Item {
                    label: "Type a line number".into(),
                    detail: String::new(),
                    target: Target::None,
                }],
            },
        }
    }

    pub(crate) fn refresh_palette(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.palette.as_ref().map(|p| p.kind) else {
            return;
        };
        let query = self.palette_query(cx);
        let items = self.compute_items(kind, &query, cx);
        if let Some(p) = self.palette.as_mut() {
            p.items = items;
            p.selected = 0;
        }
        self.preview_selected_theme(cx);
        cx.notify();
    }

    pub(crate) fn palette_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(p) = self.palette.as_mut() else {
            return;
        };
        let n = p.items.len().min(row_limit(p.kind));
        if n == 0 {
            return;
        }
        p.selected = (p.selected as isize + delta).rem_euclid(n as isize) as usize;
        self.preview_selected_theme(cx);
        cx.notify();
    }

    /// Arrowing through the theme list shows each theme live.
    fn preview_selected_theme(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.palette.as_ref() else {
            return;
        };
        if p.kind != PaletteKind::Themes {
            return;
        }
        if let Some(Item {
            target: Target::Theme(name),
            ..
        }) = p.items.get(p.selected)
            && let Some(preset) = theme::find(name, cx)
        {
            theme::apply(preset, cx);
        }
    }

    pub(crate) fn close_palette(&mut self, restore: bool, cx: &mut Context<Self>) {
        self.palette_request = None;
        if let Some(p) = self.palette.take()
            && restore
            && let Some(original) = p.original
        {
            theme::apply(original, cx);
        }
        cx.notify();
    }

    pub(crate) fn palette_confirm(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.palette.as_ref() else {
            return;
        };
        let Some(item) = p.items.get(p.selected).cloned() else {
            return;
        };
        match item.target {
            Target::None => {}
            Target::File(rel) => {
                self.close_palette(true, cx);
                self.open_file(&rel, cx);
                self.focus_doc = true;
            }
            Target::Hit(rel, line) => {
                self.close_palette(true, cx);
                self.open_file(&rel, cx);
                self.goto_line(line, cx);
                self.focus_doc = true;
            }
            Target::Line(line) => {
                self.close_palette(true, cx);
                self.goto_line(line, cx);
                self.focus_doc = true;
            }
            Target::Theme(name) => {
                // Keep the previewed theme and remember it.
                if let Some(p) = self.palette.as_mut() {
                    p.original = None;
                }
                self.close_palette(false, cx);
                theme::choose(name, cx);
            }
            Target::Mekuto(dir) => {
                self.close_palette(true, cx);
                self.open_mekuto(&dir, cx);
            }
            Target::Command(command) => {
                self.close_palette(true, cx);
                // Commands may need a window (focus, dialogs): run on render.
                self.run_after = Some(command);
                cx.notify();
            }
        }
    }

    /// Move the caret to the start of 0-based `line` of the active note.
    pub(crate) fn goto_line(&mut self, line: usize, cx: &mut Context<Self>) {
        let Some(rel) = self.doc_rel() else {
            return;
        };
        let Some(buffer) = self.buffers.get(&rel).cloned() else {
            return;
        };
        let at = buffer.read(cx).line_start(line);
        self.doc_goto(at, cx);
    }

    /// Add the theme JSON on the clipboard to the global `themes` list and
    /// switch to it; a bad theme is explained instead.
    pub(crate) fn add_theme_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let text = cx
            .read_from_clipboard()
            .and_then(|c| c.text())
            .unwrap_or_default();
        let parsed = serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|e| format!("The clipboard is not valid JSON ({e})"))
            .and_then(|v| crate::config::check_theme(&v).map(|spec| (v, spec)));
        let (value, spec) = match parsed {
            Ok(ok) => ok,
            Err(e) => {
                self.notice = Some(e);
                cx.notify();
                return;
            }
        };
        let mut list: Vec<serde_json::Value> = theme::state(cx)
            .global
            .values
            .get("themes")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        list.retain(|t| {
            t.get("name")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|n| !n.eq_ignore_ascii_case(&spec.name))
        });
        list.push(value);
        let result =
            theme::update_config(cx, |s| s.write_themes(crate::config::Scope::Global, list));
        match result {
            Ok(()) => {
                theme::choose(&spec.name, cx);
                self.notice = Some(format!("Added the theme “{}”", spec.name));
            }
            Err(e) => self.notice = Some(e),
        }
        cx.notify();
    }

    pub(crate) fn run_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            Command::FileFinder => self.open_palette(PaletteKind::Files, cx),
            Command::NewNote => {
                self.begin_naming(NamingMode::NewNote, PathBuf::new(), None, cx);
            }
            Command::NewFolder => {
                self.begin_naming(NamingMode::NewFolder, PathBuf::new(), None, cx);
            }
            Command::OpenFolder => self.on_open_folder(&OpenFolder, window, cx),
            Command::SwitchMekuto => self.open_palette(PaletteKind::Mekutos, cx),
            Command::Save => {
                if let Some(rel) = self.doc_rel() {
                    self.flush_title(cx);
                    self.save_buffer(&rel, cx);
                }
            }
            Command::CloseTab => self.close_tab(self.active_tab, cx),
            Command::ToggleSidebar => {
                self.show_sidebar = !self.show_sidebar;
                self.sidebar_auto_hidden = false;
                cx.notify();
            }
            Command::CycleMode => {
                if self.doc_rel().is_some()
                    && let Some(tab) = self.tabs.get_mut(self.active_tab)
                {
                    tab.mode = tab.mode.next();
                    cx.notify();
                }
            }
            Command::FormatTable => {
                if let Some(rel) = self.doc_rel()
                    && let Some(buffer) = self.buffers.get(&rel).cloned()
                {
                    let done = buffer.update(cx, |b, _| b.table_format());
                    if !done {
                        self.notice = Some("Put the caret inside a table first".into());
                    }
                    self.doc_note_edit(&rel);
                    cx.notify();
                }
            }
            Command::FindInNote => self.open_find(false, cx),
            Command::ReplaceInNote => self.open_find(true, cx),
            Command::SearchVault => self.open_palette(PaletteKind::Search, cx),
            Command::GoToLine => self.open_palette(PaletteKind::GotoLine, cx),
            Command::ThemePicker => self.open_palette(PaletteKind::Themes, cx),
            Command::ThemeFollowSystem => theme::follow_system(cx),
            Command::ThemeNext => {
                theme::cycle(cx);
            }
            Command::ThemeUndo => {
                self.notice = (!theme::undo(cx)).then(|| "No theme change to undo".to_string());
                cx.notify();
            }
            Command::ThemeCopy => {
                let json = theme::palette_json(&theme::active(cx));
                let text = serde_json::to_string_pretty(&json).unwrap_or_default();
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.notice = Some("Theme copied. Paste it into a \"themes\" list, or use \"Theme: add from clipboard\".".into());
                cx.notify();
            }
            Command::ThemeAdd => self.add_theme_from_clipboard(cx),
            Command::ZoomIn => theme::set_zoom(Some(1), cx),
            Command::ZoomOut => theme::set_zoom(Some(-1), cx),
            Command::ZoomReset => theme::set_zoom(None, cx),
            Command::OpenSettings => self.open_settings(None, window, cx),
            Command::ShortcutsHelp => self.open_palette(PaletteKind::Shortcuts, cx),
            Command::CheckForUpdates => {
                self.close_palette(false, cx);
                self.check_for_updates(true, cx);
            }
            Command::ToggleReduceMotion => {
                let on = !theme::cfg(cx).bool("reduce_motion");
                let _ = theme::set_setting("reduce_motion", on.into(), cx);
                cx.notify();
            }
        }
    }

    /// Remove one mekuto from the record (`Some`), or all but the open one.
    pub(crate) fn forget_mekutos(&mut self, one: Option<&std::path::Path>, cx: &mut Context<Self>) {
        match one {
            Some(dir) => crate::config::forget_mekuto(dir),
            None => crate::config::forget_all_mekutos_except(self.explorer.root()),
        }
        let selected = self.palette.as_ref().map_or(0, |p| p.selected);
        self.refresh_palette(cx);
        if let Some(p) = self.palette.as_mut() {
            p.selected = selected.min(p.items.len().saturating_sub(1));
        }
        cx.notify();
    }

    pub(crate) fn render_palette(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.palette.as_ref()?;
        let shell = cx.entity();
        let close_shell = shell.clone();
        let selected = state.selected;
        let current_mekuto = self.explorer.root().map(std::path::Path::to_path_buf);
        let rows: Vec<AnyElement> = state
            .items
            .iter()
            .take(row_limit(state.kind))
            .enumerate()
            .map(|(i, item)| {
                let click_shell = shell.clone();
                // Every remembered mekuto but the open one can be forgotten.
                let forget = match &item.target {
                    Target::Mekuto(dir) if current_mekuto.as_deref() != Some(dir.as_path()) => {
                        Some(forget_button(i, dir.clone(), shell.clone(), cx))
                    }
                    _ => None,
                };
                ListItem::new(("palette-item", i))
                    .selected(i == selected)
                    .child(
                        h_flex()
                            .w_full()
                            .gap_3()
                            .justify_between()
                            .items_center()
                            .child(div().text_sm().truncate().child(item.label.clone()))
                            .child(
                                h_flex()
                                    .flex_none()
                                    .min_w_0()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(item.detail.clone()),
                                    )
                                    .children(forget),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        click_shell.update(cx, |view, cx| {
                            if let Some(p) = view.palette.as_mut() {
                                p.selected = i;
                            }
                            view.palette_confirm(cx);
                        });
                    })
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        let mut inner = v_flex()
            .id("palette-list")
            .p_1()
            .max_h(px(360.))
            .children(rows);
        if empty {
            inner = inner.child(
                div()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No matches"),
            );
        }
        // The mekuto switcher keeps one size however many mekutos there are;
        // a longer record scrolls inside it.
        let fixed = state.kind == PaletteKind::Mekutos;
        let list = crate::smooth::frame(
            "palette-scrollbar",
            if fixed {
                div().h(px(300.))
            } else {
                div().max_h(px(360.))
            },
            if fixed { inner.max_h(px(300.)) } else { inner },
            &self.smooth.palette_h,
            &self.smooth.palette,
        );
        let footer = (state.kind == PaletteKind::Mekutos).then(|| {
            let forgettable = state
                .items
                .iter()
                .filter(|item| {
                    matches!(&item.target, Target::Mekuto(d) if current_mekuto.as_deref() != Some(d.as_path()))
                })
                .count();
            let clear_shell = shell.clone();
            let muted = cx.theme().muted_foreground;
            let (fg, hover) = (cx.theme().foreground, cx.theme().list_hover);
            h_flex()
                .px_3()
                .py_1p5()
                .justify_between()
                .items_center()
                .border_t_1()
                .border_color(cx.theme().border)
                .text_xs()
                .text_color(muted)
                .child(format!(
                    "{} {}",
                    forgettable + usize::from(current_mekuto.is_some()),
                    "remembered"
                ))
                .children((forgettable > 0).then(|| {
                        div()
                            .id("mekutos-clear-all")
                            .test_support()
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover).text_color(fg))
                            .tooltip(|window, cx| {
                                Tooltip::new("Forget every mekuto except the open one")
                                    .build(window, cx)
                            })
                            .child("Clear all")
                            .on_click(move |_, _, cx| {
                                clear_shell.update(cx, |view, cx| view.forget_mekutos(None, cx));
                            })
                }))
        });
        let panel = v_flex()
            .id("palette")
            .occlude()
            .w(px(560.))
            .max_w_full()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Input::new(&state.input)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false),
                    ),
            )
            .child(list)
            .children(footer);
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(72.))
                .key_context("Palette")
                .on_action(cx.listener(|this, _: &PaletteUp, _, cx| this.palette_move(-1, cx)))
                .on_action(cx.listener(|this, _: &PaletteDown, _, cx| this.palette_move(1, cx)))
                .child(theme::backdrop("palette-scrim", cx, move |_, cx| {
                    close_shell.update(cx, |view, cx| view.close_palette(true, cx));
                }))
                .child(theme::fade_in(panel, "palette-in", 140, 6.0, cx))
                .into_any_element(),
        )
    }
}

/// The small "forget" icon at the end of a remembered mekuto's row.
fn forget_button(i: usize, dir: PathBuf, shell: Entity<MekuShell>, cx: &App) -> impl IntoElement {
    let (fg, muted, hover) = (
        cx.theme().foreground,
        cx.theme().muted_foreground,
        cx.theme().list_active,
    );
    div()
        .id(("mekuto-forget", i))
        .test_support()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(22.))
        .rounded_md()
        .cursor_pointer()
        .text_color(muted)
        .hover(move |s| s.bg(hover).text_color(fg))
        .child(icon(Ico::Trash, px(14.), muted))
        .tooltip(|window, cx| {
            Tooltip::new("Forget this mekuto (the folder stays)").build(window, cx)
        })
        // The row's own click would open the mekuto.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            shell.update(cx, |view, cx| view.forget_mekutos(Some(&dir), cx));
        })
}
