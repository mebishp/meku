mod explorer;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use explorer::Explorer;
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::sidebar::{
    Sidebar, SidebarFooter, SidebarHeader, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tree::{TreeState, tree};
use gpui_kit::component::*;
use gpui_kit::*;
use meku_buffer::OpenBuffer;
use meku_vault::{OpenTab, Session, VaultEvent};

actions!(
    meku,
    [
        ToggleSidebar,
        NewTab,
        CloseTab,
        NextTab,
        OpenFolder,
        CancelOverlay
    ]
);

#[derive(Debug, Clone)]
struct EditorTab {
    rel: Option<PathBuf>,
    title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NamingMode {
    NewNote,
    NewFolder,
    Rename,
}

#[derive(Debug, Clone)]
struct Naming {
    mode: NamingMode,
    /// Parent directory (rel) for New*, rename target for Rename.
    dir: PathBuf,
    target: Option<PathBuf>,
}

pub struct MekuShell {
    focus_handle: FocusHandle,
    show_sidebar: bool,
    tabs: Vec<EditorTab>,
    active_tab: usize,
    explorer: Explorer,
    tree: Entity<TreeState>,
    buffers: HashMap<PathBuf, Entity<OpenBuffer>>,
    watcher: Option<notify::RecommendedWatcher>,
    watcher_rx: Option<mpsc::Receiver<VaultEvent>>,
    naming: Option<Naming>,
    naming_input: Option<Entity<InputState>>,
    _naming_sub: Option<Subscription>,
    pending_delete: Option<PathBuf>,
    notice: Option<String>,
}

impl MekuShell {
    fn new(cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let this = Self {
            focus_handle: cx.focus_handle(),
            show_sidebar: true,
            tabs: vec![EditorTab {
                rel: None,
                title: "Welcome".to_string(),
            }],
            active_tab: 0,
            explorer: Explorer::new(),
            tree,
            buffers: HashMap::new(),
            watcher: None,
            watcher_rx: None,
            naming: None,
            naming_input: None,
            _naming_sub: None,
            pending_delete: None,
            notice: None,
        };
        this.start_watcher_pump(cx);
        this
    }

    /// Poll the filesystem watcher a few times per second. Runs until the
    /// view is dropped.
    fn start_watcher_pump(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let Some(view) = this.upgrade() else {
                    break;
                };
                cx.update(|cx| {
                    view.update(cx, |shell, cx| shell.poll_watcher(cx));
                });
            }
        })
        .detach();
    }

    // -- tabs ----------------------------------------------------------

    fn tab_title(rel: &Path) -> String {
        rel.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string()
    }

    fn open_file(&mut self, rel: &Path, cx: &mut Context<Self>) {
        if self.explorer.is_dir(rel) {
            self.explorer.toggle(rel);
            self.sync_tree(cx);
            cx.notify();
            return;
        }
        self.open_file_inner(rel, cx);
    }

    fn open_file_inner(&mut self, rel: &Path, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.rel.as_deref() == Some(rel)) {
            self.active_tab = ix;
            self.sync_tree_selection(cx);
            self.save_session();
            cx.notify();
            return;
        }
        let abs = match self.explorer.abs(rel) {
            Some(abs) => abs,
            None => return,
        };
        match OpenBuffer::load(rel.to_path_buf(), &abs) {
            Ok(buffer) => {
                self.buffers.insert(rel.to_path_buf(), cx.new(|_| buffer));
                self.tabs.push(EditorTab {
                    rel: Some(rel.to_path_buf()),
                    title: Self::tab_title(rel),
                });
                self.active_tab = self.tabs.len() - 1;
                self.notice = None;
            }
            Err(e) => {
                self.notice = Some(format!("Cannot open {}: {e}", rel.display()));
            }
        }
        self.sync_tree_selection(cx);
        self.save_session();
        cx.notify();
    }

    fn new_tab(&mut self, _: &NewTab, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len() + 1;
        self.tabs.push(EditorTab {
            rel: None,
            title: format!("Untitled-{n}"),
        });
        self.active_tab = self.tabs.len() - 1;
        self.save_session();
        cx.notify();
    }

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.tabs.push(EditorTab {
                rel: None,
                title: "Welcome".to_string(),
            });
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        self.sync_tree_selection(cx);
        self.save_session();
        cx.notify();
    }

    fn on_close_tab(&mut self, _: &CloseTab, _: &mut Window, cx: &mut Context<Self>) {
        self.close_tab(self.active_tab, cx);
    }

    fn on_next_tab(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
            self.sync_tree_selection(cx);
            self.save_session();
            cx.notify();
        }
    }

    /// Close every tab at or under `rel` (file or deleted directory).
    fn close_tabs_for(&mut self, rel: &Path, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 && self.tabs.iter().all(|t| t.rel.is_none()) {
            return;
        }
        self.tabs.retain(|t| {
            t.rel
                .as_ref()
                .is_none_or(|r| r != rel && !r.starts_with(rel))
        });
        self.buffers.retain(|r, _| r != rel && !r.starts_with(rel));
        if self.tabs.is_empty() {
            self.tabs.push(EditorTab {
                rel: None,
                title: "Welcome".to_string(),
            });
        }
        self.active_tab = self.active_tab.min(self.tabs.len() - 1);
        self.save_session();
        cx.notify();
    }

    // -- mekuto open / session ------------------------------------------

    fn on_open_folder(&mut self, _: &OpenFolder, _: &mut Window, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let dir = cx
                .background_spawn(async move {
                    rfd::FileDialog::new()
                        .set_title("Open mekuto folder")
                        .pick_folder()
                })
                .await;
            if let Some(dir) = dir
                && let Some(view) = this.upgrade()
            {
                view.update(cx, |shell, cx| shell.open_mekuto(&dir, cx));
            }
        })
        .detach();
    }

    fn open_mekuto(&mut self, dir: &Path, cx: &mut Context<Self>) {
        match self.explorer.open(dir) {
            Ok(session) => {
                let root = self.explorer.root().unwrap().to_path_buf();
                let (tx, rx) = mpsc::channel();
                match meku_vault::start_watcher(&root, tx) {
                    Ok(watcher) => {
                        self.watcher = Some(watcher);
                        self.watcher_rx = Some(rx);
                    }
                    Err(e) => {
                        self.notice = Some(format!("Watching disabled: {e}"));
                    }
                }
                self.tabs.clear();
                self.buffers.clear();
                for tab in &session.open_tabs {
                    if root.join(&tab.path).is_file() {
                        self.open_file_inner(&tab.path, cx);
                    }
                }
                if let Some(active) = &session.active_tab
                    && let Some(ix) = self
                        .tabs
                        .iter()
                        .position(|t| t.rel.as_ref() == Some(active))
                {
                    self.active_tab = ix;
                }
                if self.tabs.is_empty() {
                    self.tabs.push(EditorTab {
                        rel: None,
                        title: "Welcome".to_string(),
                    });
                    self.active_tab = 0;
                }
                self.notice = None;
                self.sync_tree(cx);
                self.save_session();
                cx.notify();
            }
            Err(e) => {
                self.notice = Some(format!("Cannot open {}: {e}", dir.display()));
                cx.notify();
            }
        }
    }

    fn save_session(&mut self) {
        let Some(root) = self.explorer.root().map(Path::to_path_buf) else {
            return;
        };
        let session = Session {
            version: 1,
            active_tab: self.tabs.get(self.active_tab).and_then(|t| t.rel.clone()),
            open_tabs: self
                .tabs
                .iter()
                .filter_map(|t| {
                    t.rel.clone().map(|path| OpenTab {
                        path,
                        cursor: 0,
                        scroll_px: 0.0,
                    })
                })
                .collect(),
        };
        // Best effort: a failed save must never break editing.
        let _ = meku_vault::save_session_atomic(&root, &session);
    }

    // -- watcher ----------------------------------------------------------

    fn poll_watcher(&mut self, cx: &mut Context<Self>) {
        let mut events = Vec::new();
        if let Some(rx) = &self.watcher_rx {
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }
        if events.is_empty() {
            return;
        }
        let mut tree_changed = false;
        let mut reload = Vec::new();
        let mut removed = Vec::new();
        for event in &events {
            match event {
                VaultEvent::Modified(file) if self.buffers.contains_key(&file.rel) => {
                    reload.push(file.rel.clone());
                }
                VaultEvent::Removed(rel) => removed.push(rel.clone()),
                _ => {}
            }
            if self.explorer.on_vault_event(event) {
                tree_changed = true;
            }
        }
        for rel in reload {
            self.reload_buffer(&rel, cx);
        }
        for rel in removed {
            self.close_tabs_for(&rel, cx);
        }
        if tree_changed {
            self.explorer.refresh();
            self.sync_tree(cx);
        }
        cx.notify();
    }

    fn reload_buffer(&mut self, rel: &Path, cx: &mut Context<Self>) {
        let Some(abs) = self.explorer.abs(rel) else {
            return;
        };
        if let Ok(buffer) = OpenBuffer::load(rel.to_path_buf(), &abs) {
            self.buffers.insert(rel.to_path_buf(), cx.new(|_| buffer));
            self.notice = Some(format!("Reloaded {} (changed on disk)", rel.display()));
        }
    }

    // -- tree ----------------------------------------------------------

    fn sync_tree(&mut self, cx: &mut Context<Self>) {
        let items = self.explorer.tree_items();
        let selected = self
            .tabs
            .get(self.active_tab)
            .and_then(|t| t.rel.clone())
            .map(|r| SharedString::from(r.to_string_lossy().to_string()));
        self.tree.update(cx, |state, cx| {
            state.set_items(items, cx);
            if let Some(id) = &selected
                && let Some(ix) = state.index_of(id)
            {
                state.set_selected_index(Some(ix), cx);
            }
        });
    }

    fn sync_tree_selection(&mut self, cx: &mut Context<Self>) {
        let selected = self
            .tabs
            .get(self.active_tab)
            .and_then(|t| t.rel.clone())
            .map(|r| SharedString::from(r.to_string_lossy().to_string()));
        self.tree.update(cx, |state, cx| {
            let ix = selected.as_ref().and_then(|id| state.index_of(id));
            state.set_selected_index(ix, cx);
        });
    }

    fn render_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity();
        let menu_shell = shell.clone();
        tree(&self.tree, move |_ix, entry, _selected, _window, cx| {
            let id = entry.item().id.clone();
            let rel = PathBuf::from(id.to_string());
            let is_dir = shell.read(cx).explorer.is_dir(&rel);
            let expanded = shell.read(cx).explorer.is_expanded(&rel);
            let depth = entry.depth();
            let open_shell = shell.clone();
            let open_rel = rel.clone();
            let chevron = if is_dir {
                if expanded { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            ListItem::new(id.clone())
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .pl(px(depth as f32 * 12.0 + 4.0))
                        .text_sm()
                        .child(chevron)
                        .child(entry.item().label.clone()),
                )
                .on_click(move |_, _, cx| {
                    open_shell.update(cx, |view, cx| view.open_file(&open_rel, cx));
                })
        })
        .context_menu(move |_ix, entry, menu, _window, cx| {
            let rel = PathBuf::from(entry.item().id.to_string());
            let is_dir = menu_shell.read(cx).explorer.is_dir(&rel);
            let mut menu = menu;
            if is_dir {
                for (label, mode) in [
                    ("New note", NamingMode::NewNote),
                    ("New folder", NamingMode::NewFolder),
                ] {
                    let s = menu_shell.clone();
                    let dir = rel.clone();
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                        s.update(cx, |view, cx| {
                            view.begin_naming(mode, dir.clone(), None, cx)
                        });
                    }));
                }
                menu = menu.item(PopupMenuItem::separator());
            }
            {
                let s = menu_shell.clone();
                let target = rel.clone();
                menu = menu.item(PopupMenuItem::new("Rename").on_click(move |_, _, cx| {
                    s.update(cx, |view, cx| {
                        view.begin_naming(
                            NamingMode::Rename,
                            target.clone(),
                            Some(target.clone()),
                            cx,
                        )
                    });
                }));
            }
            {
                let s = menu_shell.clone();
                let target = rel.clone();
                menu = menu.item(PopupMenuItem::new("Delete").on_click(move |_, _, cx| {
                    s.update(cx, |view, cx| view.ask_delete(target.clone(), cx));
                }));
            }
            menu
        })
    }

    // -- naming modal (new note / new folder / rename) --------------------

    fn naming_preset(mode: NamingMode, target: Option<&Path>) -> String {
        match mode {
            NamingMode::NewNote => "Untitled.md".to_string(),
            NamingMode::NewFolder => "Untitled folder".to_string(),
            NamingMode::Rename => target
                .and_then(|t| t.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string(),
        }
    }

    fn naming_title(mode: NamingMode) -> &'static str {
        match mode {
            NamingMode::NewNote => "New note",
            NamingMode::NewFolder => "New folder",
            NamingMode::Rename => "Rename",
        }
    }

    fn begin_naming(
        &mut self,
        mode: NamingMode,
        dir: PathBuf,
        target: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.pending_delete = None;
        self.naming = Some(Naming {
            mode,
            dir,
            target: target.clone(),
        });
        self.naming_input = None;
        self._naming_sub = None;
        // The InputState entity is created on the next render, where a
        // Window is available; the preset is derived there too.
        cx.notify();
    }

    fn ensure_naming_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.naming.is_none() || self.naming_input.is_some() {
            return;
        }
        let preset = self
            .naming
            .as_ref()
            .map(|n| Self::naming_preset(n.mode, n.target.as_deref()))
            .unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(preset));
        let sub = cx.subscribe(&input, Self::on_naming_event);
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.naming_input = Some(input);
        self._naming_sub = Some(sub);
    }

    fn on_naming_event(
        &mut self,
        _state: Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.commit_naming(cx);
        }
    }

    fn commit_naming(&mut self, cx: &mut Context<Self>) {
        let (Some(naming), Some(input)) = (self.naming.clone(), self.naming_input.clone()) else {
            return;
        };
        let name = input.read(cx).value().to_string();
        enum Done {
            OpenNote(PathBuf),
            RevealDir(PathBuf),
            Renamed,
        }
        let result: anyhow::Result<Done> = match naming.mode {
            NamingMode::NewNote => self
                .explorer
                .create_note(&naming.dir, &name)
                .map(Done::OpenNote),
            NamingMode::NewFolder => self
                .explorer
                .create_dir(&naming.dir, &name)
                .map(Done::RevealDir),
            NamingMode::Rename => match naming.target.as_ref() {
                Some(target) => match self.explorer.rename(target, &name) {
                    Ok(new_rel) => {
                        self.retarget_tabs(target, &new_rel, cx);
                        Ok(Done::Renamed)
                    }
                    Err(e) => Err(e),
                },
                None => Err(anyhow::anyhow!("nothing to rename")),
            },
        };
        match result {
            Ok(done) => {
                self.naming = None;
                self.naming_input = None;
                self._naming_sub = None;
                self.notice = None;
                self.explorer.refresh();
                match done {
                    Done::RevealDir(dir) => {
                        self.explorer.reveal(&dir);
                    }
                    Done::OpenNote(note) => {
                        self.explorer.reveal(&note);
                        self.sync_tree(cx);
                        self.open_file_inner(&note, cx);
                        return;
                    }
                    Done::Renamed => {}
                }
                self.sync_tree(cx);
                cx.notify();
            }
            Err(e) => {
                self.notice = Some(e.to_string());
                cx.notify();
            }
        }
    }

    fn retarget_tabs(&mut self, old: &Path, new: &Path, cx: &mut Context<Self>) {
        for tab in &mut self.tabs {
            if let Some(rel) = &tab.rel
                && (rel == old || rel.starts_with(old))
            {
                let suffix = rel.strip_prefix(old).unwrap_or(Path::new(""));
                tab.rel = Some(new.join(suffix));
                tab.title = Self::tab_title(&tab.rel.clone().unwrap());
            }
        }
        // Move cached buffers along; directories move whole subtrees.
        let keys: Vec<PathBuf> = self.buffers.keys().cloned().collect();
        for key in keys {
            if (key == *old || key.starts_with(old))
                && let Some(entity) = self.buffers.remove(&key)
            {
                let suffix = key.strip_prefix(old).unwrap_or(Path::new(""));
                let moved = new.join(suffix);
                entity.update(cx, |buffer, _| buffer.rel = moved.clone());
                self.buffers.insert(moved, entity);
            }
        }
        self.save_session();
    }

    fn cancel_naming(&mut self, cx: &mut Context<Self>) {
        if self.naming.is_some() {
            self.naming = None;
            self.naming_input = None;
            self._naming_sub = None;
            cx.notify();
        }
    }

    // -- delete modal ----------------------------------------------------

    fn ask_delete(&mut self, rel: PathBuf, cx: &mut Context<Self>) {
        self.naming = None;
        self.naming_input = None;
        self._naming_sub = None;
        self.pending_delete = Some(rel);
        cx.notify();
    }

    fn commit_delete(&mut self, cx: &mut Context<Self>) {
        let Some(rel) = self.pending_delete.clone() else {
            return;
        };
        match self.explorer.delete(&rel) {
            Ok(()) => {
                self.pending_delete = None;
                self.notice = None;
                self.explorer.refresh();
                self.close_tabs_for(&rel, cx);
                self.sync_tree(cx);
                cx.notify();
            }
            Err(e) => {
                self.notice = Some(e.to_string());
                cx.notify();
            }
        }
    }

    fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        if self.pending_delete.is_some() {
            self.pending_delete = None;
            cx.notify();
        }
    }

    fn on_cancel_overlay(&mut self, _: &CancelOverlay, _: &mut Window, cx: &mut Context<Self>) {
        self.cancel_naming(cx);
        self.cancel_delete(cx);
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.show_sidebar = !self.show_sidebar;
        cx.notify();
    }

    // -- chrome ----------------------------------------------------------

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.explorer.is_open() {
            return Sidebar::new("meku-sidebar")
                .header(SidebarHeader::new().child("No mekuto open"))
                .child(
                    SidebarMenu::new()
                        .child(SidebarMenuItem::new("Explorer"))
                        .child(SidebarMenuItem::new("Search"))
                        .child(SidebarMenuItem::new("Outline")),
                )
                .footer(SidebarFooter::new().child("Meku v0.1"))
                .into_any_element();
        }
        let shell = cx.entity();
        let new_note = shell.clone();
        let new_folder = shell.clone();
        Sidebar::new("meku-sidebar")
            .header(
                SidebarHeader::new().child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(self.explorer.root_name())
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new("new-note")
                                        .label("+ Note")
                                        .ghost()
                                        .xsmall()
                                        .on_click(move |_, _, cx| {
                                            new_note.update(cx, |view, cx| {
                                                view.begin_naming(
                                                    NamingMode::NewNote,
                                                    PathBuf::new(),
                                                    None,
                                                    cx,
                                                )
                                            });
                                        }),
                                )
                                .child(
                                    Button::new("new-folder")
                                        .label("+ Dir")
                                        .ghost()
                                        .xsmall()
                                        .on_click(move |_, _, cx| {
                                            new_folder.update(cx, |view, cx| {
                                                view.begin_naming(
                                                    NamingMode::NewFolder,
                                                    PathBuf::new(),
                                                    None,
                                                    cx,
                                                )
                                            });
                                        }),
                                ),
                        ),
                ),
            )
            .child(
                SidebarMenu::new()
                    .child(SidebarMenuItem::new("Explorer"))
                    .child(SidebarMenuItem::new("Search"))
                    .child(SidebarMenuItem::new("Outline")),
            )
            .footer(SidebarFooter::new().child("Meku v0.1"))
            .into_any_element()
    }

    fn render_center(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let bar_entity = entity.clone();
        let tab_bar = TabBar::new("meku-tabs")
            .selected_index(self.active_tab)
            .on_click(move |index: &usize, _window, cx| {
                let index = *index;
                bar_entity.update(cx, |view, cx| {
                    view.active_tab = index;
                    view.sync_tree_selection(cx);
                    view.save_session();
                    cx.notify();
                });
            })
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                let entity = entity.clone();
                Tab::new().label(tab.title.clone()).suffix(
                    Button::new(format!("close-tab-{index}"))
                        .label("×")
                        .ghost()
                        .xsmall()
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |view, cx| {
                                view.close_tab(index, cx);
                            });
                        }),
                )
            }));

        let active = self
            .tabs
            .get(self.active_tab)
            .cloned()
            .unwrap_or(EditorTab {
                rel: None,
                title: "Welcome".to_string(),
            });
        let empty_state = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(cx.theme().muted_foreground)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(active.title),
            )
            .child(if self.explorer.is_open() {
                "Select a note in the explorer — editing lands in Build 3."
            } else {
                "Open a folder (Ctrl-O) or pass it on the command line."
            });

        v_flex()
            .size_full()
            .child(tab_bar)
            .child(div().flex_1().child(empty_state).into_any_element())
    }

    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let left = if let Some(notice) = &self.notice {
            notice.clone()
        } else if self.explorer.is_open() {
            format!("● {}", self.explorer.root_name())
        } else {
            "○ No mekuto open".to_string()
        };
        h_flex()
            .h(px(28.))
            .px_2()
            .gap_3()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(left)
            .child(h_flex().gap_3().child("Markdown").child("Ln 1, Col 1"))
    }

    fn render_naming_modal(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let naming = self.naming.clone()?;
        let input = self.naming_input.clone()?;
        let shell = cx.entity();
        let commit_shell = shell.clone();
        let cancel_shell = shell.clone();
        let title = Self::naming_title(naming.mode).to_string();
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x0000_0000))
                .opacity(0.5)
                .child(
                    div()
                        .w(px(360.))
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background)
                        .p_4()
                        .opacity(1.0)
                        .gap_3()
                        .flex()
                        .flex_col()
                        .text_color(cx.theme().foreground)
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(Input::new(&input).id("naming-input").small())
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    Button::new("naming-cancel")
                                        .label("Cancel")
                                        .ghost()
                                        .on_click(move |_, _, cx| {
                                            cancel_shell
                                                .update(cx, |view, cx| view.cancel_naming(cx));
                                        }),
                                )
                                .child(
                                    Button::new("naming-commit")
                                        .label("Save")
                                        .primary()
                                        .on_click(move |_, _, cx| {
                                            commit_shell
                                                .update(cx, |view, cx| view.commit_naming(cx));
                                        }),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_delete_modal(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rel = self.pending_delete.clone()?;
        let shell = cx.entity();
        let delete_shell = shell.clone();
        let cancel_shell = shell.clone();
        let is_dir = self.explorer.is_dir(&rel);
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x0000_0000))
                .opacity(0.5)
                .child(
                    div()
                        .w(px(360.))
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background)
                        .p_4()
                        .opacity(1.0)
                        .gap_3()
                        .flex()
                        .flex_col()
                        .text_color(cx.theme().foreground)
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!(
                                    "Delete {}?",
                                    if is_dir { "folder" } else { "note" }
                                )),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(rel.to_string_lossy().to_string()),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    Button::new("delete-cancel")
                                        .label("Cancel")
                                        .ghost()
                                        .on_click(move |_, _, cx| {
                                            cancel_shell
                                                .update(cx, |view, cx| view.cancel_delete(cx));
                                        }),
                                )
                                .child(
                                    Button::new("delete-commit")
                                        .label("Delete")
                                        .danger()
                                        .on_click(move |_, _, cx| {
                                            delete_shell
                                                .update(cx, |view, cx| view.commit_delete(cx));
                                        }),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

impl Render for MekuShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_naming_input(window, cx);
        let focus = self.focus_handle.clone();
        let title = match (self.explorer.is_open(), self.tabs.get(self.active_tab)) {
            (true, Some(tab)) => format!("Meku — {} — {}", self.explorer.root_name(), tab.title),
            (true, None) => format!("Meku — {}", self.explorer.root_name()),
            (false, _) => "Meku — no mekuto open".to_string(),
        };
        let mut root = div()
            .size_full()
            .relative()
            .track_focus(&focus)
            .key_context("Meku")
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_open_folder))
            .on_action(cx.listener(Self::on_cancel_overlay))
            .child(TitleBar::new().child(title))
            .child(
                div().flex_1().child(
                    h_resizable("meku-shell")
                        .child(
                            resizable_panel()
                                .visible(self.show_sidebar)
                                .size(px(240.))
                                .size_range(px(200.)..px(400.))
                                .child(
                                    v_flex()
                                        .size_full()
                                        .child(self.render_sidebar(cx))
                                        .child(
                                            div()
                                                .flex_1()
                                                .overflow_hidden()
                                                .child(self.render_tree(cx)),
                                        )
                                        .into_any_element(),
                                ),
                        )
                        .child(self.render_center(cx).into_any_element()),
                ),
            )
            .child(self.render_status_bar(cx));
        if let Some(modal) = self
            .render_naming_modal(cx)
            .or_else(|| self.render_delete_modal(cx))
        {
            root = root.child(modal);
        }
        root
    }
}

fn main() {
    let initial_dir = std::env::args().nth(1);
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(move |cx| {
        // Required: initializes gpui-component theming/global state.
        gpui_kit::init(cx);

        cx.bind_keys([
            KeyBinding::new("ctrl-b", ToggleSidebar, Some("Meku")),
            KeyBinding::new("ctrl-t", NewTab, Some("Meku")),
            KeyBinding::new("ctrl-w", CloseTab, Some("Meku")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Meku")),
            KeyBinding::new("ctrl-o", OpenFolder, Some("Meku")),
            KeyBinding::new("escape", CancelOverlay, None),
        ]);

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        cx.spawn(async move |cx| {
            let mut options = TitleBar::window_options();
            options.window_bounds = Some(WindowBounds::Windowed(bounds));
            options.app_id = Some("meku".to_string());
            cx.open_window(options, |window, cx| {
                let view = cx.new(MekuShell::new);
                if let Some(dir) = initial_dir {
                    view.update(cx, |shell, cx| shell.open_mekuto(Path::new(&dir), cx));
                }
                // First level on the window must be a Root.
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open meku window");
        })
        .detach();
    });
}

#[cfg(test)]
mod ui_tests {
    //! Headless UI integration tests (require `gpui-kit/test-support`).
    //! They drive the real shell through real key/mouse events — in
    //! particular the modal keyboard flows no unit test can cover.

    // NOTE: no `use super::*` here on purpose. The crate-root globs
    // (`use gpui_kit::*`) pull GPUI's `test` proc-macro into scope, which
    // would shadow the builtin `#[test]` and send the harness into macro
    // recursion. Import everything this module needs explicitly.
    use crate::{
        CancelOverlay, CloseTab, MekuShell, NamingMode, NewTab, NextTab, OpenFolder, ToggleSidebar,
    };
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, KeyBinding, TestAppContext, px, size};
    use std::path::PathBuf;

    /// Manual equivalent of `#[gpui_kit::test]` (whose macro overflows
    /// rustc in this crate). Mirrors the harness the macro generates:
    /// build a TestAppContext, run the body, drain and quit.
    fn run_ui_test(name: &'static str, f: fn(&mut TestAppContext)) {
        gpui_kit::run_test(
            1,
            &[],
            0,
            &mut |dispatcher, _seed| {
                let mut cx = gpui_kit::TestAppContext::build(dispatcher.clone(), Some(name));
                let _entity_refcounts = cx.app.borrow().ref_counts_drop_handle();
                f(&mut cx);
                cx.run_until_parked();
                cx.update(|cx| {
                    cx.background_executor().forbid_parking();
                    cx.quit();
                });
                cx.run_until_parked();
                drop(cx);
                dispatcher.drain_tasks();
                drop(dispatcher);
            },
            None,
        );
    }

    fn boot(cx: &mut TestAppContext) -> (Entity<MekuShell>, gpui_kit::AnyWindowHandle) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            cx.bind_keys([
                KeyBinding::new("ctrl-b", ToggleSidebar, Some("Meku")),
                KeyBinding::new("ctrl-t", NewTab, Some("Meku")),
                KeyBinding::new("ctrl-w", CloseTab, Some("Meku")),
                KeyBinding::new("ctrl-tab", NextTab, Some("Meku")),
                KeyBinding::new("ctrl-o", OpenFolder, Some("Meku")),
                KeyBinding::new("escape", CancelOverlay, None),
            ]);
        });
        let mut view = None;
        let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
            let shell = cx.new(MekuShell::new);
            view = Some(shell.clone());
            Root::new(shell, window, cx)
        });
        (view.unwrap(), handle.into())
    }

    fn open_fixture(cx: &mut TestAppContext, view: &Entity<MekuShell>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n").unwrap();
        let root = dir.path().to_path_buf();
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_mekuto(&root, cx));
        });
        dir
    }

    #[test]
    fn escape_cancels_naming_while_input_focused() {
        run_ui_test("escape_cancels", escape_cancels_body);
    }

    fn escape_cancels_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let _dir = open_fixture(cx, &view);

        // Open the rename modal for note.md through the real entry point.
        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                shell.begin_naming(
                    NamingMode::Rename,
                    PathBuf::from("note.md"),
                    Some(PathBuf::from("note.md")),
                    cx,
                );
            });
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("naming-input", cx);
            assert!(window.find("naming-input").focused().unwrap_or(false));
            window.press("escape", cx);
        })
        .unwrap();

        // Modal gone, file untouched.
        let (naming_open, tabs) = cx
            .update(|cx| view.read_with(cx, |shell, _| (shell.naming.is_some(), shell.tabs.len())));
        assert!(!naming_open);
        assert_eq!(tabs, 1);
    }

    #[test]
    fn enter_commits_naming_and_opens_note() {
        run_ui_test("enter_commits", enter_commits_body);
    }

    fn enter_commits_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let dir = open_fixture(cx, &view);

        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                shell.begin_naming(NamingMode::NewNote, PathBuf::new(), None, cx);
            });
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("naming-input", cx);
            // Accept the "Untitled.md" preset as-is.
            window.press("enter", cx);
        })
        .unwrap();

        assert!(dir.path().join("Untitled.md").exists());
        let (naming_open, tabs) = cx
            .update(|cx| view.read_with(cx, |shell, _| (shell.naming.is_some(), shell.tabs.len())));
        assert!(!naming_open);
        assert_eq!(tabs, 2); // Welcome + Untitled.md
    }

    #[test]
    fn invalid_name_keeps_modal_open_with_notice() {
        run_ui_test("invalid_keeps_modal", invalid_keeps_modal_body);
    }

    fn invalid_keeps_modal_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let _dir = open_fixture(cx, &view);

        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                shell.begin_naming(
                    NamingMode::Rename,
                    PathBuf::from("note.md"),
                    Some(PathBuf::from("note.md")),
                    cx,
                );
            });
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("naming-input", cx);
            // A separator makes the preset an invalid file name.
            window.input("/", cx);
            window.press("enter", cx);
        })
        .unwrap();

        let (naming_open, notice) = cx.update(|cx| {
            view.read_with(cx, |shell, _| {
                (shell.naming.is_some(), shell.notice.clone())
            })
        });
        assert!(naming_open);
        assert!(notice.is_some());
    }
}
