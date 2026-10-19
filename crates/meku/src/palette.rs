//! Quick switcher, command palette, vault search and go-to-line: one overlay
//! (input + filtered list) driven by a [`PaletteKind`].

use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;

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
}

impl PaletteKind {
    fn placeholder(self) -> &'static str {
        match self {
            PaletteKind::Files => "Open a note by name…",
            PaletteKind::Commands => "Type a command…",
            PaletteKind::Themes => "Choose a theme…",
            PaletteKind::Search => "Search every note…",
            PaletteKind::GotoLine => "Line number…",
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
    Save,
    CloseTab,
    ToggleSidebar,
    CycleMode,
    FindInNote,
    ReplaceInNote,
    SearchVault,
    GoToLine,
    ThemePicker,
    ThemeFollowSystem,
    ThemeNext,
    ToggleReduceMotion,
}

/// `(command, title, shortcut)` in the order shown for an empty query.
const COMMANDS: &[(Command, &str, &str)] = &[
    (Command::FileFinder, "Open note…", "Ctrl+P"),
    (Command::NewNote, "New note", "Ctrl+T"),
    (Command::NewFolder, "New folder", ""),
    (Command::OpenFolder, "Open folder…", "Ctrl+O"),
    (Command::Save, "Save", "Ctrl+S"),
    (Command::CloseTab, "Close tab", "Ctrl+W"),
    (Command::ToggleSidebar, "Toggle sidebar", "Ctrl+B"),
    (Command::CycleMode, "Cycle Edit / Live / Read", "Ctrl+E"),
    (Command::FindInNote, "Find in note", "Ctrl+F"),
    (Command::ReplaceInNote, "Find and replace in note", "Ctrl+H"),
    (Command::SearchVault, "Search in all notes", "Ctrl+Shift+F"),
    (Command::GoToLine, "Go to line…", "Ctrl+G"),
    (Command::ThemePicker, "Theme: choose…", ""),
    (Command::ThemeFollowSystem, "Theme: follow system", ""),
    (Command::ThemeNext, "Theme: next", "Ctrl+Alt+T"),
    (Command::ToggleReduceMotion, "Toggle reduce motion", ""),
];

#[derive(Clone)]
pub(crate) enum Target {
    File(PathBuf),
    /// A search hit: note and 0-based line.
    Hit(PathBuf, usize),
    Command(Command),
    Theme(&'static str),
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

const MAX_ITEMS: usize = 10;

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
                theme::PRESETS
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
        let n = p.items.len().min(MAX_ITEMS);
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
            && let Some(preset) = theme::preset(name)
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
            Command::Save => {
                if let Some(rel) = self.doc_rel() {
                    self.flush_title(cx);
                    self.save_buffer(&rel, cx);
                }
            }
            Command::CloseTab => self.close_tab(self.active_tab, cx),
            Command::ToggleSidebar => {
                self.show_sidebar = !self.show_sidebar;
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
            Command::FindInNote => self.open_find(false, cx),
            Command::ReplaceInNote => self.open_find(true, cx),
            Command::SearchVault => self.open_palette(PaletteKind::Search, cx),
            Command::GoToLine => self.open_palette(PaletteKind::GotoLine, cx),
            Command::ThemePicker => self.open_palette(PaletteKind::Themes, cx),
            Command::ThemeFollowSystem => theme::follow_system(cx),
            Command::ThemeNext => {
                let current = theme::active(cx).name;
                let at = theme::PRESETS
                    .iter()
                    .position(|p| p.name == current)
                    .unwrap_or(0);
                theme::choose(theme::PRESETS[(at + 1) % theme::PRESETS.len()].name, cx);
            }
            Command::ToggleReduceMotion => {
                let mut settings = cx.global::<theme::Settings>().clone();
                settings.reduce_motion = !settings.reduce_motion;
                settings.save();
                cx.set_global(settings);
                cx.notify();
            }
        }
    }

    pub(crate) fn render_palette(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.palette.as_ref()?;
        let shell = cx.entity();
        let close_shell = shell.clone();
        let selected = state.selected;
        let rows: Vec<AnyElement> = state
            .items
            .iter()
            .take(MAX_ITEMS)
            .enumerate()
            .map(|(i, item)| {
                let click_shell = shell.clone();
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
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(item.detail.clone()),
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
        let mut list = v_flex().p_1().max_h(px(360.)).children(rows);
        if empty {
            list = list.child(
                div()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No matches"),
            );
        }
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
                .child(
                    div()
                        .id("palette-scrim")
                        .absolute()
                        .inset_0()
                        .bg(theme::scrim(cx))
                        .on_click(move |_, _, cx| {
                            close_shell.update(cx, |view, cx| view.close_palette(true, cx));
                        }),
                )
                .child(
                    v_flex()
                        .id("palette")
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
                        .child(list),
                )
                .into_any_element(),
        )
    }
}
