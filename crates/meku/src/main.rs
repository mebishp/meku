mod document;
mod explorer;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use explorer::Explorer;
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::*;
use gpui_kit::*;
// InteractiveElement (div().on_click / overflow_y_scroll) is not in the globs.
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};
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
        CancelOverlay,
        Save,
        CycleMode,
        DocLeft,
        DocRight,
        DocUp,
        DocDown,
        DocHome,
        DocEnd,
        DocSelectLeft,
        DocSelectRight,
        DocSelectUp,
        DocSelectDown,
        DocSelectHome,
        DocSelectEnd,
        DocBackspace,
        DocDelete,
        DocEnter,
        DocSelectAll,
        DocUndo,
        DocRedo,
        DocCopy,
        DocCut,
        DocPaste
    ]
);

/// How the open document is presented. Live is the default: one surface,
/// caret row raw, everything else preview-styled. Edit shows raw source
/// everywhere; Read styles every row, hides the caret and blocks edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Edit,
    Live,
    Read,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Edit => "Edit",
            Mode::Live => "Live",
            Mode::Read => "Read",
        }
    }

    fn next(self) -> Mode {
        match self {
            Mode::Edit => Mode::Live,
            Mode::Live => Mode::Read,
            Mode::Read => Mode::Edit,
        }
    }
}

#[derive(Debug, Clone)]
struct EditorTab {
    rel: Option<PathBuf>,
    title: String,
    mode: Mode,
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
    buffers: HashMap<PathBuf, Entity<OpenBuffer>>,
    last_edit: HashMap<PathBuf, Instant>,
    /// Parsed rows per open file, rebuilt when the buffer version changes.
    doc_models: HashMap<PathBuf, document::DocModel>,
    /// Row layouts from the last render, used for pointer and IME hit testing.
    doc_views: document::RowViews,
    /// Scroll and measurement state of the document list.
    doc_list: ListState,
    /// File the list state currently describes.
    doc_list_rel: Option<PathBuf>,
    /// Rel path with unsaved edits that changed on disk underneath us.
    conflict: Option<PathBuf>,
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
        let this = Self {
            focus_handle: cx.focus_handle(),
            show_sidebar: true,
            tabs: vec![EditorTab {
                rel: None,
                title: "Welcome".to_string(),
                mode: Mode::Live,
            }],
            active_tab: 0,
            explorer: Explorer::new(),
            buffers: HashMap::new(),
            last_edit: HashMap::new(),
            doc_models: HashMap::new(),
            doc_views: Rc::new(RefCell::new(Vec::new())),
            doc_list: ListState::new(0, ListAlignment::Top, px(600.)),
            doc_list_rel: None,
            conflict: None,
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
            cx.notify();
            return;
        }
        self.open_file_inner(rel, cx);
    }

    fn open_file_inner(&mut self, rel: &Path, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.rel.as_deref() == Some(rel)) {
            self.active_tab = ix;
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
                    mode: Mode::Live,
                });
                self.active_tab = self.tabs.len() - 1;
                self.notice = None;
            }
            Err(e) => {
                self.notice = Some(format!("Cannot open {}: {e}", rel.display()));
            }
        }
        self.save_session();
        cx.notify();
    }

    fn new_tab(&mut self, _: &NewTab, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len() + 1;
        self.tabs.push(EditorTab {
            rel: None,
            title: format!("Untitled-{n}"),
            mode: Mode::Live,
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
                mode: Mode::Live,
            });
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        self.save_session();
        cx.notify();
    }

    fn on_close_tab(&mut self, _: &CloseTab, _: &mut Window, cx: &mut Context<Self>) {
        self.close_tab(self.active_tab, cx);
    }

    fn on_next_tab(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
            self.save_session();
            cx.notify();
        }
    }

    /// Presentation mode of the tab currently visible, defaulting to Live.
    fn tab_mode(&self) -> Mode {
        self.tabs
            .get(self.active_tab)
            .map(|t| t.mode)
            .unwrap_or(Mode::Live)
    }

    fn on_cycle_mode(&mut self, _: &CycleMode, _: &mut Window, cx: &mut Context<Self>) {
        if self.doc_rel().is_some() {
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                tab.mode = tab.mode.next();
            }
            // Rows do not change count, only presentation: keep the viewport.
            cx.notify();
        }
    }

    fn set_tab_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.doc_rel().is_some() {
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                tab.mode = mode;
            }
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
        self.doc_models
            .retain(|r, _| r != rel && !r.starts_with(rel));
        self.last_edit
            .retain(|r, _| r != rel && !r.starts_with(rel));
        if self
            .conflict
            .as_ref()
            .is_some_and(|c| c == rel || c.starts_with(rel))
        {
            self.conflict = None;
        }
        if self.tabs.is_empty() {
            self.tabs.push(EditorTab {
                rel: None,
                title: "Welcome".to_string(),
                mode: Mode::Live,
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
                self.last_edit.clear();
                self.doc_models.clear();
                self.conflict = None;
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
                        mode: Mode::Live,
                    });
                    self.active_tab = 0;
                }
                self.notice = None;
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
        if !events.is_empty() {
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
                self.sync_buffer_with_disk(&rel, cx);
            }
            for rel in removed {
                self.close_tabs_for(&rel, cx);
            }
            if tree_changed {
                self.explorer.refresh();
            }
            cx.notify();
        }
        self.autosave(cx);
    }

    /// Reconcile an open buffer with its file after a watcher Modified event.
    /// Identical content (e.g. our own save) only refreshes the mtime, so the
    /// editor is never rebuilt from under the cursor. A dirty buffer with
    /// genuinely different disk content raises [`Self::conflict`] instead of
    /// reloading; a clean one reloads silently.
    fn sync_buffer_with_disk(&mut self, rel: &Path, cx: &mut Context<Self>) {
        let (Some(abs), Some(buffer)) = (self.explorer.abs(rel), self.buffers.get(rel).cloned())
        else {
            return;
        };
        let disk = std::fs::read_to_string(&abs).unwrap_or_default();
        let (dirty, snapshot) = buffer.read_with(cx, |b, _| (b.dirty, b.snapshot().1));
        if disk == snapshot {
            let mtime = std::fs::metadata(&abs).and_then(|m| m.modified()).ok();
            buffer.update(cx, |b, _| {
                if let Some(mtime) = mtime {
                    b.disk_mtime = Some(mtime);
                }
            });
            return;
        }
        if dirty {
            self.conflict = Some(rel.to_path_buf());
            self.notice = Some(format!(
                "{} changed on disk — resolve above to keep editing",
                rel.display()
            ));
            cx.notify();
            return;
        }
        self.replace_buffer_text(rel, &disk, cx);
        self.notice = Some(format!("Reloaded {} (changed on disk)", rel.display()));
        cx.notify();
    }

    /// Replace a buffer's text with the disk version. Used only for explicit
    /// reload decisions (clean buffers, or "Load disk"), so the caret is kept
    /// within bounds and the parsed rows are rebuilt on the next frame.
    fn replace_buffer_text(&mut self, rel: &Path, text: &str, cx: &mut Context<Self>) {
        if let Some(buffer) = self.buffers.get(rel) {
            let len = buffer.read_with(cx, |b, _| b.len_bytes());
            buffer.update(cx, |b, _| {
                b.apply_edit(0..len, text);
                b.dirty = false;
                b.move_caret(0, false);
            });
        }
        self.doc_models.remove(rel);
        self.last_edit.remove(rel);
    }

    fn is_dirty(&self, rel: &Path, cx: &App) -> bool {
        self.buffers.get(rel).is_some_and(|b| b.read(cx).dirty)
    }

    // -- document editing actions ----------------------------------------

    fn on_doc_left(&mut self, _: &DocLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Left, false, window, cx);
    }
    fn on_doc_right(&mut self, _: &DocRight, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Right, false, window, cx);
    }
    fn on_doc_up(&mut self, _: &DocUp, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Up, false, window, cx);
    }
    fn on_doc_down(&mut self, _: &DocDown, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Down, false, window, cx);
    }
    fn on_doc_home(&mut self, _: &DocHome, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Home, false, window, cx);
    }
    fn on_doc_end(&mut self, _: &DocEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::End, false, window, cx);
    }
    fn on_doc_select_left(
        &mut self,
        _: &DocSelectLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::Left, true, window, cx);
    }
    fn on_doc_select_right(
        &mut self,
        _: &DocSelectRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::Right, true, window, cx);
    }
    fn on_doc_select_up(&mut self, _: &DocSelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Up, true, window, cx);
    }
    fn on_doc_select_down(
        &mut self,
        _: &DocSelectDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::Down, true, window, cx);
    }
    fn on_doc_select_home(
        &mut self,
        _: &DocSelectHome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::Home, true, window, cx);
    }
    fn on_doc_select_end(&mut self, _: &DocSelectEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::End, true, window, cx);
    }
    fn on_doc_backspace(&mut self, _: &DocBackspace, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Backspace, false, window, cx);
    }
    fn on_doc_delete(&mut self, _: &DocDelete, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Delete, false, window, cx);
    }
    fn on_doc_enter(&mut self, _: &DocEnter, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Enter, false, window, cx);
    }
    fn on_doc_select_all(&mut self, _: &DocSelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::SelectAll, false, window, cx);
    }
    fn on_doc_undo(&mut self, _: &DocUndo, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Undo, false, window, cx);
    }
    fn on_doc_redo(&mut self, _: &DocRedo, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Redo, false, window, cx);
    }
    fn on_doc_copy(&mut self, _: &DocCopy, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Copy, false, window, cx);
    }
    fn on_doc_cut(&mut self, _: &DocCut, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Cut, false, window, cx);
    }
    fn on_doc_paste(&mut self, _: &DocPaste, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Paste, false, window, cx);
    }

    // -- saving ----------------------------------------------------------

    fn on_save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(rel) = self.tabs.get(self.active_tab).and_then(|t| t.rel.clone()) {
            self.save_buffer(&rel, cx);
        }
    }

    fn save_buffer(&mut self, rel: &Path, cx: &mut Context<Self>) {
        let (Some(abs), Some(buffer)) = (self.explorer.abs(rel), self.buffers.get(rel).cloned())
        else {
            return;
        };
        let text = buffer.read_with(cx, |b, _| b.snapshot().1);
        match std::fs::write(&abs, &text) {
            Ok(()) => {
                let mtime = std::fs::metadata(&abs).and_then(|m| m.modified()).ok();
                buffer.update(cx, |b, _| {
                    b.dirty = false;
                    if let Some(mtime) = mtime {
                        b.disk_mtime = Some(mtime);
                    }
                });
                self.last_edit.remove(rel);
                // A save from a conflicted buffer resolves the conflict.
                if self.conflict.as_deref() == Some(rel) {
                    self.conflict = None;
                }
                self.notice = None;
            }
            Err(e) => {
                self.notice = Some(format!("Save failed for {}: {e}", rel.display()));
            }
        }
        cx.notify();
    }

    /// Save dirty buffers idle for longer than the debounce window.
    /// Runs inside the watcher pump, a few times per second.
    fn autosave(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let due: Vec<PathBuf> = self
            .buffers
            .iter()
            .filter(|(rel, b)| {
                b.read_with(cx, |b, _| b.dirty)
                    && self
                        .last_edit
                        .get(*rel)
                        .is_none_or(|at| now.duration_since(*at) > Duration::from_millis(800))
            })
            .map(|(rel, _)| rel.clone())
            .collect();
        for rel in due {
            self.save_buffer(&rel, cx);
        }
    }

    // -- conflict banner --------------------------------------------------

    fn resolve_keep_mine(&mut self, cx: &mut Context<Self>) {
        if let Some(rel) = self.conflict.clone()
            && let (Some(abs), Some(buffer)) =
                (self.explorer.abs(&rel), self.buffers.get(&rel).cloned())
        {
            // Adopt the on-disk mtime without touching text or dirty state;
            // no further event will arrive for this change.
            if let Ok(mtime) = std::fs::metadata(&abs).and_then(|m| m.modified()) {
                buffer.update(cx, |b, _| b.disk_mtime = Some(mtime));
            }
        }
        self.conflict = None;
        self.notice = None;
        cx.notify();
    }

    fn resolve_load_disk(&mut self, cx: &mut Context<Self>) {
        if let Some(rel) = self.conflict.clone()
            && let Some(abs) = self.explorer.abs(&rel)
            && let Ok(disk) = std::fs::read_to_string(&abs)
        {
            self.replace_buffer_text(&rel, &disk, cx);
            self.conflict = None;
            self.notice = Some(format!("Reloaded {} from disk", rel.display()));
            cx.notify();
        }
    }

    // -- file tree (plain rows; selection derives from the active tab) --

    /// Plain file rows from [`Explorer::flat_rows`]: no virtualized tree
    /// state to starve of height, selection derived from the active tab.
    fn render_file_tree(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.entity();
        let active_rel = self.tabs.get(self.active_tab).and_then(|t| t.rel.clone());
        let rows: Vec<AnyElement> = self
            .explorer
            .flat_rows()
            .into_iter()
            .map(|row| {
                let open_shell = shell.clone();
                let menu_shell = shell.clone();
                let open_rel = row.rel.clone();
                let menu_rel = row.rel.clone();
                let row_id = format!("tree-row-{}", row.rel.display());
                let chevron = if row.is_dir {
                    if row.expanded { "▾ " } else { "▸ " }
                } else {
                    "  "
                };
                let selected = active_rel.as_ref() == Some(&row.rel);
                ListItem::new(row_id)
                    .selected(selected)
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .pl(px(row.depth as f32 * 12.0 + 4.0))
                            .text_sm()
                            .child(chevron)
                            .child(row.label),
                    )
                    .on_click(move |_, _, cx| {
                        open_shell.update(cx, |view, cx| view.open_file(&open_rel, cx));
                    })
                    .context_menu(move |menu, _window, cx| {
                        let is_dir = menu_shell.read(cx).explorer.is_dir(&menu_rel);
                        let mut menu = menu;
                        if is_dir {
                            for (label, mode) in [
                                ("New note", NamingMode::NewNote),
                                ("New folder", NamingMode::NewFolder),
                            ] {
                                let s = menu_shell.clone();
                                let dir = menu_rel.clone();
                                menu = menu.item(PopupMenuItem::new(label).on_click(
                                    move |_, _, cx| {
                                        s.update(cx, |view, cx| {
                                            view.begin_naming(mode, dir.clone(), None, cx)
                                        });
                                    },
                                ));
                            }
                            menu = menu.item(PopupMenuItem::separator());
                        }
                        {
                            let s = menu_shell.clone();
                            let target = menu_rel.clone();
                            menu = menu.item(PopupMenuItem::new("Rename").on_click(
                                move |_, _, cx| {
                                    s.update(cx, |view, cx| {
                                        view.begin_naming(
                                            NamingMode::Rename,
                                            target.clone(),
                                            Some(target.clone()),
                                            cx,
                                        )
                                    });
                                },
                            ));
                        }
                        {
                            let s = menu_shell.clone();
                            let target = menu_rel.clone();
                            menu = menu.item(PopupMenuItem::new("Delete").on_click(
                                move |_, _, cx| {
                                    s.update(cx, |view, cx| view.ask_delete(target.clone(), cx));
                                },
                            ));
                        }
                        menu
                    })
                    .into_any_element()
            })
            .collect();
        v_flex().children(rows).into_any_element()
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
                        self.open_file_inner(&note, cx);
                        return;
                    }
                    Done::Renamed => {}
                }
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
        // Move cached buffers along; directories move subtrees. Editor text
        // is path-independent, so the entities move as they are.
        let keys: Vec<PathBuf> = self.buffers.keys().cloned().collect();
        for key in keys {
            if (key == *old || key.starts_with(old))
                && let Some(entity) = self.buffers.remove(&key)
            {
                let suffix = key.strip_prefix(old).unwrap_or(Path::new(""));
                let moved = new.join(suffix);
                entity.update(cx, |buffer, _| buffer.rel = moved.clone());
                self.buffers.insert(moved.clone(), entity);
                if let Some(at) = self.last_edit.remove(&key) {
                    self.last_edit.insert(moved.clone(), at);
                }
                self.doc_models.remove(&key);
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

    /// Plain themed sidebar: header, file tree, footer. Deliberately plain
    /// `div`s — the virtualized `Sidebar`/`Tree` components starve of
    /// measured height in this layout and render nothing.
    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let base = v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_color(cx.theme().sidebar_border);
        if !self.explorer.is_open() {
            return base
                .child(
                    div()
                        .p_3()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("No mekuto open"),
                )
                .child(
                    div()
                        .px_3()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Open a folder (Ctrl-O) to begin."),
                )
                .into_any_element();
        }
        let shell = cx.entity();
        let new_note = shell.clone();
        let new_folder = shell.clone();
        base.child(
            h_flex()
                .justify_between()
                .items_center()
                .px_3()
                .py_2()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
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
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .id("file-tree-scroll")
                .test_support()
                .overflow_y_scroll()
                .child(self.render_file_tree(cx)),
        )
        .child(
            div()
                .px_3()
                .py_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Meku v0.1"),
        )
        .into_any_element()
    }

    fn render_center(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let bar_entity = entity.clone();
        let tab_bar = TabBar::new("meku-tabs")
            .selected_index(self.active_tab)
            .on_click(move |index: &usize, _window, cx| {
                let index = *index;
                bar_entity.update(cx, |view, cx| {
                    view.active_tab = index;
                    view.save_session();
                    cx.notify();
                });
            })
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                let entity = entity.clone();
                let dirty = tab.rel.as_ref().is_some_and(|rel| self.is_dirty(rel, cx));
                let label = if dirty {
                    format!("• {}", tab.title)
                } else {
                    tab.title.clone()
                };
                Tab::new().label(label).suffix(
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
                mode: Mode::Live,
            });
        let body: AnyElement = match active.rel.clone() {
            Some(rel) if self.buffers.contains_key(&rel) => {
                let mut column = v_flex().size_full().min_h_0();
                if self.conflict.as_deref() == Some(rel.as_path()) {
                    column = column.child(self.render_conflict_banner(&rel, cx));
                }
                column
                    .child(self.render_mode_switch(active.mode, cx))
                    .child(self.render_document(&rel, window, cx))
                    .into_any_element()
            }
            _ => self.render_empty_state(&active.title, cx),
        };

        v_flex()
            .size_full()
            .child(div().flex_none().child(tab_bar))
            .child(v_flex().flex_1().min_h_0().child(body))
    }

    /// Edit / Live / Read buttons, right-aligned above the document. Only
    /// shown while a note is open; the active one is filled.
    fn render_mode_switch(&mut self, mode: Mode, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.entity();
        let buttons: Vec<AnyElement> = [Mode::Edit, Mode::Live, Mode::Read]
            .into_iter()
            .map(|m| {
                let click_shell = shell.clone();
                let mut button = Button::new(format!("mode-{:?}", m).to_lowercase())
                    .label(m.label())
                    .xsmall();
                button = if m == mode {
                    button.primary()
                } else {
                    button.ghost()
                };
                button
                    .on_click(move |_, _, cx| {
                        click_shell.update(cx, |view, cx| view.set_tab_mode(m, cx));
                    })
                    .into_any_element()
            })
            .collect();
        h_flex()
            .flex_none()
            .justify_end()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .children(buttons)
            .into_any_element()
    }

    fn render_empty_state(&mut self, title: &str, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
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
                    .child(title.to_string()),
            )
            .child(if self.explorer.is_open() {
                "Select a note in the explorer to start editing."
            } else {
                "Open a folder (Ctrl-O) or pass it on the command line."
            })
            .into_any_element()
    }

    fn render_conflict_banner(&mut self, rel: &Path, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.entity();
        let load_shell = shell.clone();
        let keep_shell = shell.clone();
        h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .text_xs()
            .child(
                div()
                    .flex_1()
                    .text_color(cx.theme().foreground)
                    .child(format!(
                        "{} changed on disk. Load the disk version or keep your edits?",
                        rel.display()
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("conflict-keep")
                            .label("Keep mine")
                            .ghost()
                            .xsmall()
                            .on_click(move |_, _, cx| {
                                keep_shell.update(cx, |view, cx| view.resolve_keep_mine(cx));
                            }),
                    )
                    .child(
                        Button::new("conflict-load")
                            .label("Load disk")
                            .primary()
                            .xsmall()
                            .on_click(move |_, _, cx| {
                                load_shell.update(cx, |view, cx| view.resolve_load_disk(cx));
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_dirty = self
            .tabs
            .get(self.active_tab)
            .and_then(|t| t.rel.as_deref())
            .is_some_and(|rel| self.is_dirty(rel, cx));
        let left = if let Some(notice) = &self.notice {
            notice.clone()
        } else if self.explorer.is_open() {
            let dot = if active_dirty { "●" } else { "○" };
            format!("{dot} {}", self.explorer.root_name())
        } else {
            "○ No mekuto open".to_string()
        };
        // gpui-kit's StatusBar draws the strip itself; the wrapper pins it as
        // the last child of the root column (flex_none keeps it from shrinking)
        // and carries the test id, since StatusBar is not interactive.
        div().id("status-bar").test_support().flex_none().child(
            StatusBar::new()
                .left(left)
                .right("Markdown")
                .right(if self.doc_rel().is_some() {
                    self.tab_mode().label()
                } else {
                    "Ln 1, Col 1"
                }),
        )
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
        let mut root = v_flex()
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
            .on_action(cx.listener(Self::on_cycle_mode))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_doc_left))
            .on_action(cx.listener(Self::on_doc_right))
            .on_action(cx.listener(Self::on_doc_up))
            .on_action(cx.listener(Self::on_doc_down))
            .on_action(cx.listener(Self::on_doc_home))
            .on_action(cx.listener(Self::on_doc_end))
            .on_action(cx.listener(Self::on_doc_select_left))
            .on_action(cx.listener(Self::on_doc_select_right))
            .on_action(cx.listener(Self::on_doc_select_up))
            .on_action(cx.listener(Self::on_doc_select_down))
            .on_action(cx.listener(Self::on_doc_select_home))
            .on_action(cx.listener(Self::on_doc_select_end))
            .on_action(cx.listener(Self::on_doc_backspace))
            .on_action(cx.listener(Self::on_doc_delete))
            .on_action(cx.listener(Self::on_doc_enter))
            .on_action(cx.listener(Self::on_doc_select_all))
            .on_action(cx.listener(Self::on_doc_undo))
            .on_action(cx.listener(Self::on_doc_redo))
            .on_action(cx.listener(Self::on_doc_copy))
            .on_action(cx.listener(Self::on_doc_cut))
            .on_action(cx.listener(Self::on_doc_paste))
            .child(TitleBar::new().child(title))
            .child(
                v_flex().flex_1().min_h_0().overflow_hidden().child(
                    h_resizable("meku-shell")
                        .child(
                            resizable_panel()
                                .visible(self.show_sidebar)
                                .size(px(240.))
                                .size_range(px(200.)..px(400.))
                                .child(self.render_sidebar(cx)),
                        )
                        .child(self.render_center(window, cx).into_any_element()),
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
            KeyBinding::new("ctrl-s", Save, Some("Meku")),
            KeyBinding::new("ctrl-e", CycleMode, Some("Meku")),
            KeyBinding::new("escape", CancelOverlay, None),
            KeyBinding::new("left", DocLeft, Some("Meku")),
            KeyBinding::new("right", DocRight, Some("Meku")),
            KeyBinding::new("up", DocUp, Some("Meku")),
            KeyBinding::new("down", DocDown, Some("Meku")),
            KeyBinding::new("home", DocHome, Some("Meku")),
            KeyBinding::new("end", DocEnd, Some("Meku")),
            KeyBinding::new("shift-left", DocSelectLeft, Some("Meku")),
            KeyBinding::new("shift-right", DocSelectRight, Some("Meku")),
            KeyBinding::new("shift-up", DocSelectUp, Some("Meku")),
            KeyBinding::new("shift-down", DocSelectDown, Some("Meku")),
            KeyBinding::new("shift-home", DocSelectHome, Some("Meku")),
            KeyBinding::new("shift-end", DocSelectEnd, Some("Meku")),
            KeyBinding::new("backspace", DocBackspace, Some("Meku")),
            KeyBinding::new("delete", DocDelete, Some("Meku")),
            KeyBinding::new("enter", DocEnter, Some("Meku")),
            KeyBinding::new("ctrl-a", DocSelectAll, Some("Meku")),
            KeyBinding::new("ctrl-z", DocUndo, Some("Meku")),
            KeyBinding::new("ctrl-y", DocRedo, Some("Meku")),
            KeyBinding::new("ctrl-shift-z", DocRedo, Some("Meku")),
            KeyBinding::new("ctrl-c", DocCopy, Some("Meku")),
            KeyBinding::new("ctrl-x", DocCut, Some("Meku")),
            KeyBinding::new("ctrl-v", DocPaste, Some("Meku")),
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
        CancelOverlay, CloseTab, CycleMode, DocBackspace, DocCopy, DocCut, DocDelete, DocDown,
        DocEnd, DocEnter, DocHome, DocLeft, DocPaste, DocRedo, DocRight, DocSelectAll,
        DocSelectDown, DocSelectEnd, DocSelectHome, DocSelectLeft, DocSelectRight, DocSelectUp,
        DocUndo, DocUp, MekuShell, Mode, NamingMode, NewTab, NextTab, OpenFolder, Save,
        ToggleSidebar,
    };
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Bounds, Entity, KeyBinding, Pixels, TestAppContext, px, size};
    use std::path::{Path, PathBuf};

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
        boot_sized(cx, 1200.0, 800.0)
    }

    fn boot_sized(
        cx: &mut TestAppContext,
        width: f32,
        height: f32,
    ) -> (Entity<MekuShell>, gpui_kit::AnyWindowHandle) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            cx.bind_keys([
                KeyBinding::new("ctrl-b", ToggleSidebar, Some("Meku")),
                KeyBinding::new("ctrl-t", NewTab, Some("Meku")),
                KeyBinding::new("ctrl-w", CloseTab, Some("Meku")),
                KeyBinding::new("ctrl-tab", NextTab, Some("Meku")),
                KeyBinding::new("ctrl-o", OpenFolder, Some("Meku")),
                KeyBinding::new("ctrl-s", Save, Some("Meku")),
                KeyBinding::new("ctrl-e", CycleMode, Some("Meku")),
                KeyBinding::new("escape", CancelOverlay, None),
                KeyBinding::new("left", DocLeft, Some("Meku")),
                KeyBinding::new("right", DocRight, Some("Meku")),
                KeyBinding::new("up", DocUp, Some("Meku")),
                KeyBinding::new("down", DocDown, Some("Meku")),
                KeyBinding::new("home", DocHome, Some("Meku")),
                KeyBinding::new("end", DocEnd, Some("Meku")),
                KeyBinding::new("shift-left", DocSelectLeft, Some("Meku")),
                KeyBinding::new("shift-right", DocSelectRight, Some("Meku")),
                KeyBinding::new("shift-up", DocSelectUp, Some("Meku")),
                KeyBinding::new("shift-down", DocSelectDown, Some("Meku")),
                KeyBinding::new("shift-home", DocSelectHome, Some("Meku")),
                KeyBinding::new("shift-end", DocSelectEnd, Some("Meku")),
                KeyBinding::new("backspace", DocBackspace, Some("Meku")),
                KeyBinding::new("delete", DocDelete, Some("Meku")),
                KeyBinding::new("enter", DocEnter, Some("Meku")),
                KeyBinding::new("ctrl-a", DocSelectAll, Some("Meku")),
                KeyBinding::new("ctrl-z", DocUndo, Some("Meku")),
                KeyBinding::new("ctrl-y", DocRedo, Some("Meku")),
                KeyBinding::new("ctrl-shift-z", DocRedo, Some("Meku")),
                KeyBinding::new("ctrl-c", DocCopy, Some("Meku")),
                KeyBinding::new("ctrl-x", DocCut, Some("Meku")),
                KeyBinding::new("ctrl-v", DocPaste, Some("Meku")),
            ]);
        });
        let mut view = None;
        let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
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

    fn focus_doc_for(
        cx: &mut TestAppContext,
        view: &Entity<MekuShell>,
        window: gpui_kit::AnyWindowHandle,
    ) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let handle = view.read_with(cx, |shell, _| shell.focus_handle.clone());
            window.focus(&handle, cx);
        })
        .unwrap();
    }

    fn buffer_text(
        cx: &mut TestAppContext,
        view: &Entity<MekuShell>,
        rel: &Path,
    ) -> (String, bool) {
        cx.update(|cx| {
            view.read_with(cx, |shell, cx| {
                shell
                    .buffers
                    .get(rel)
                    .map(|b| b.read_with(cx, |b, _| (b.snapshot().1, b.dirty)))
                    .unwrap_or_default()
            })
        })
    }

    #[test]
    fn typing_flows_into_buffer_and_marks_dirty() {
        run_ui_test("typing_dirty", typing_dirty_body);
    }

    fn typing_dirty_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let _dir = open_fixture(cx, &view);
        let rel = PathBuf::from("note.md");
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });
        focus_doc_for(cx, &view, window);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("Hello ", cx);
        })
        .unwrap();

        let (text, dirty) = buffer_text(cx, &view, &rel);
        assert!(
            text.starts_with("Hello "),
            "typed text lands in buffer: {text:?}"
        );
        assert!(dirty, "buffer is dirty after typing");
        // Tab label gains a dirty dot.
        let label = cx.update(|cx| {
            view.read_with(cx, |shell, cx| {
                let tab = &shell.tabs[shell.active_tab];
                let dirty = tab.rel.as_ref().is_some_and(|r| shell.is_dirty(r, cx));
                format!("{}{}", if dirty { "• " } else { "" }, tab.title)
            })
        });
        assert!(label.starts_with("• "), "dirty tab shows a dot: {label:?}");
    }

    #[test]
    fn save_writes_file_and_clears_dirty() {
        run_ui_test("save_clears", save_clears_body);
    }

    fn save_clears_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let dir = open_fixture(cx, &view);
        let rel = PathBuf::from("note.md");
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });
        focus_doc_for(cx, &view, window);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("saved!", cx);
        })
        .unwrap();
        assert!(buffer_text(cx, &view, &rel).1);

        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.save_buffer(&rel, cx));
        });
        let (text, dirty) = buffer_text(cx, &view, &rel);
        assert!(!dirty);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
            text
        );
    }

    #[test]
    fn undo_reverts_typed_text() {
        run_ui_test("undo_reverts", undo_reverts_body);
    }

    fn undo_reverts_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let _dir = open_fixture(cx, &view);
        let rel = PathBuf::from("note.md");
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });
        focus_doc_for(cx, &view, window);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("xyz", cx);
        })
        .unwrap();
        assert!(buffer_text(cx, &view, &rel).0.starts_with("xyz"));

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-z", cx);
        })
        .unwrap();
        let (text, _) = buffer_text(cx, &view, &rel);
        assert!(
            !text.starts_with("xyz"),
            "undo removed typed text: {text:?}"
        );
    }

    #[test]
    fn external_edit_on_dirty_buffer_raises_conflict() {
        run_ui_test("conflict_flow", conflict_flow_body);
    }

    fn conflict_flow_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let dir = open_fixture(cx, &view);
        let rel = PathBuf::from("note.md");
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });
        focus_doc_for(cx, &view, window);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("mine ", cx);
        })
        .unwrap();
        assert!(
            buffer_text(cx, &view, &rel).1,
            "buffer dirty before conflict"
        );

        // Someone else rewrites the file behind our back.
        std::fs::write(dir.path().join("note.md"), "# Theirs\n").unwrap();
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.sync_buffer_with_disk(&rel, cx));
        });
        let conflict = cx.update(|cx| view.read_with(cx, |shell, _| shell.conflict.clone()));
        assert_eq!(conflict, Some(rel.clone()));
        // Buffer text untouched by the external write.
        assert!(buffer_text(cx, &view, &rel).0.starts_with("mine "));

        // Loading disk wins: buffer follows the file.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("conflict-load", cx);
        })
        .unwrap();
        let (text, _) = buffer_text(cx, &view, &rel);
        assert_eq!(text, "# Theirs\n");
        let conflict = cx.update(|cx| view.read_with(cx, |shell, _| shell.conflict.clone()));
        assert!(conflict.is_none());
    }

    #[test]
    fn external_edit_on_clean_buffer_reloads_silently() {
        run_ui_test("clean_reload", clean_reload_body);
    }

    #[test]
    fn file_tree_rows_open_and_toggle() {
        run_ui_test("tree_rows", tree_rows_body);
    }

    fn tree_rows_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("notes")).unwrap();
        std::fs::write(dir.path().join("notes/b.md"), "# B\n").unwrap();
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();
        let root = dir.path().to_path_buf();
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_mekuto(&root, cx));
        });

        // Both rows render with stable ids.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("tree-row-a.md").is_some());
            assert!(window.try_find("tree-row-notes").is_some());
            // Clicking a file row opens it in a tab.
            window.click("tree-row-a.md", cx);
        })
        .unwrap();
        let tabs = cx.update(|cx| {
            view.read_with(cx, |shell, _| {
                shell
                    .tabs
                    .iter()
                    .map(|t| t.title.clone())
                    .collect::<Vec<_>>()
            })
        });
        assert!(
            tabs.iter().any(|t| t == "a.md"),
            "click opened a.md: {tabs:?}"
        );

        // Clicking a directory toggles expansion: children vanish…
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("tree-row-notes", cx);
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("tree-row-notes/b.md").is_none());
            // …and come back on the next click.
            window.click("tree-row-notes", cx);
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("tree-row-notes/b.md").is_some());
        })
        .unwrap();
    }

    fn clean_reload_body(cx: &mut TestAppContext) {
        let (view, _window) = boot(cx);
        let dir = open_fixture(cx, &view);
        let rel = PathBuf::from("note.md");
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });
        std::fs::write(dir.path().join("note.md"), "# Fresh\n").unwrap();
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.sync_buffer_with_disk(&rel, cx));
        });
        let (text, _) = buffer_text(cx, &view, &rel);
        assert_eq!(text, "# Fresh\n");
        let conflict = cx.update(|cx| view.read_with(cx, |shell, _| shell.conflict.clone()));
        assert!(conflict.is_none());
    }

    fn doc_fixture(cx: &mut TestAppContext) -> (Entity<MekuShell>, tempfile::TempDir, PathBuf) {
        let (view, _window) = boot(cx);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("doc.md"), "# T\n\nbody **bold** here\n").unwrap();
        let rel = PathBuf::from("doc.md");
        let root = dir.path().to_path_buf();
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_mekuto(&root, cx));
        });
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });
        (view, dir, rel)
    }

    #[test]
    fn rows_hide_markup_except_caret_row() {
        run_ui_test("rows_hide_markup", rows_hide_markup_body);
    }

    fn rows_hide_markup_body(cx: &mut TestAppContext) {
        let (view, _dir, rel) = doc_fixture(cx);
        let (kinds, active_row) = cx.update(|cx| {
            view.update(cx, |shell, cx| {
                let model = shell.doc_model(&rel, cx).unwrap();
                let caret = shell.buffers.get(&rel).unwrap().read(cx).caret();
                let kinds = model.rows.iter().map(|r| r.kind).collect::<Vec<_>>();
                (kinds, meku_markdown::live::row_at(&model.rows, caret))
            })
        });
        // "# T", blank, "body **bold** here", trailing blank.
        assert_eq!(kinds.len(), 4);
        assert_eq!(active_row, 0, "caret starts on the first row");
    }

    #[test]
    fn click_moves_caret_without_touching_text() {
        run_ui_test("click_moves_caret", click_moves_caret_body);
    }

    fn click_moves_caret_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("doc.md"), "# T\n\nbody here\n").unwrap();
        let rel = PathBuf::from("doc.md");
        let root = dir.path().to_path_buf();
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_mekuto(&root, cx));
        });
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });

        let before = buffer_text(cx, &view, &rel).0;
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(("doc-row", 2usize), cx);
        })
        .unwrap();

        let (caret_row, after) = cx.update(|cx| {
            view.update(cx, |shell, cx| {
                let caret = shell.buffers.get(&rel).unwrap().read(cx).caret();
                let model = shell.doc_model(&rel, cx).unwrap();
                (
                    meku_markdown::live::row_at(&model.rows, caret),
                    shell.buffers.get(&rel).unwrap().read(cx).text_string(),
                )
            })
        });
        assert_eq!(caret_row, 2, "click lands the caret on the paragraph row");
        assert_eq!(after, before, "merely moving the caret changes no text");
        let _ = dir;
    }

    #[test]
    fn typing_inserts_at_caret_in_place() {
        run_ui_test("typing_inserts", typing_inserts_body);
    }

    fn typing_inserts_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("doc.md"), "# T\n\nbody here\n").unwrap();
        let rel = PathBuf::from("doc.md");
        let root = dir.path().to_path_buf();
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_mekuto(&root, cx));
        });
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });
        // Place the caret at the start of the paragraph row directly: a
        // centre click lands at the end of short text, which is correct
        // behaviour but not what this insertion asserts.
        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                let dest = "# T\n\n".len();
                if let Some(buffer) = shell.buffers.get(&rel).cloned() {
                    buffer.update(cx, |b, _| {
                        b.move_caret(dest, false);
                    });
                }
            });
        });

        focus_doc_for(cx, &view, window);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("! ", cx);
        })
        .unwrap();

        let (text, dirty) = buffer_text(cx, &view, &rel);
        assert_eq!(text, "# T\n\n! body here\n");
        assert!(dirty);
        let _ = dir;
    }

    #[test]
    fn document_has_left_margin() {
        run_ui_test("doc_margin", doc_margin_body);
    }

    fn doc_margin_body(cx: &mut TestAppContext) {
        let (_view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "# T\n\nbody\n");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let scroll = window.find("md-scroll").bounds();
            let row = window.find(("doc-row", 0usize)).bounds();
            let margin = row.origin.x.as_f32() - scroll.origin.x.as_f32();
            assert!(
                margin >= 8.0,
                "document needs a left margin so the caret never clips, got {margin}"
            );
        })
        .unwrap();
    }

    fn caret_bounds(
        cx: &mut TestAppContext,
        window: gpui_kit::AnyWindowHandle,
    ) -> (Bounds<Pixels>, Bounds<Pixels>) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            (
                window.find("doc-caret").bounds(),
                window.find(("doc-row", 2usize)).bounds(),
            )
        })
        .unwrap()
    }

    fn move_doc_caret(cx: &mut TestAppContext, view: &Entity<MekuShell>, dest: usize) {
        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                if let Some(rel) = shell.doc_rel()
                    && let Some(buffer) = shell.buffers.get(&rel).cloned()
                {
                    buffer.update(cx, |b, _| b.move_caret(dest, false));
                }
            });
        });
    }

    #[test]
    fn caret_follows_the_active_row() {
        run_ui_test("caret_follows_row", caret_follows_row_body);
    }

    fn caret_follows_row_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "# T\n\nbody here\n");
        // Caret onto the paragraph row: the caret element must sit inside it.
        move_doc_caret(cx, &view, "# T\n\n".len());
        let (caret, row) = caret_bounds(cx, window);
        assert!(
            caret.origin.y.as_f32() >= row.origin.y.as_f32() - 1.0
                && (caret.origin.y.as_f32() + caret.size.height.as_f32())
                    <= (row.origin.y.as_f32() + row.size.height.as_f32() + 1.0),
            "caret {caret:?} must be inside its row {row:?}"
        );
        // Back to the first row: the caret must move with it, not stick.
        move_doc_caret(cx, &view, 0);
        let top_row = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find(("doc-row", 0usize)).bounds()
            })
            .unwrap();
        let caret = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("doc-caret").bounds()
            })
            .unwrap();
        assert!(
            caret.origin.y.as_f32() >= top_row.origin.y.as_f32() - 1.0
                && (caret.origin.y.as_f32() + caret.size.height.as_f32())
                    <= (top_row.origin.y.as_f32() + top_row.size.height.as_f32() + 1.0),
            "caret {caret:?} must follow the caret to the first row {top_row:?}"
        );
    }

    #[test]
    fn caret_advances_with_the_text() {
        run_ui_test("caret_advances", caret_advances_body);
    }

    fn caret_advances_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "# T\n\nbody here\n");
        move_doc_caret(cx, &view, "# T\n\n".len());
        let (left, _) = caret_bounds(cx, window);
        move_doc_caret(cx, &view, "# T\n\nbody".len());
        let (right, _) = caret_bounds(cx, window);
        assert!(
            right.origin.x.as_f32() > left.origin.x.as_f32() + 5.0,
            "caret must advance with the text: {left:?} -> {right:?}"
        );
    }

    #[test]
    fn caret_matches_painted_glyphs() {
        run_ui_test("caret_glyphs", caret_glyphs_body);
    }

    fn caret_glyphs_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "# T\n\nbody here\n");
        let dest = "# T\n\nbody here".len();
        move_doc_caret(cx, &view, dest);
        let (caret_b, row_b, glyph) = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let caret_b = window.find("doc-caret").bounds();
                let row_b = window.find(("doc-row", 2usize)).bounds();
                let glyph = view.read_with(cx, |shell, _| {
                    shell
                        .doc_views
                        .borrow()
                        .get(2)
                        .and_then(|slot| slot.as_ref())
                        .and_then(|rv| rv.layout.position_for_index(rv.display.to_disp(dest)))
                        .unwrap()
                });
                (caret_b, row_b, glyph)
            })
            .unwrap();
        // Caret div is row-local; the element reports window-space, so shift
        // the glyph point into the row before comparing. `position_for_index`
        // is layout-space (no line-box padding) while paint centers glyphs in
        // the line box: add that padding to the expected y.
        let got_x = caret_b.origin.x.as_f32() - row_b.origin.x.as_f32();
        let got_y = caret_b.origin.y.as_f32() - row_b.origin.y.as_f32();
        let want_x = glyph.x.as_f32() - row_b.origin.x.as_f32();
        let want_y = view.read_with(cx, |shell, _| {
            let views = shell.doc_views.borrow();
            let rv = views[2].as_ref().unwrap();
            let baseline = rv
                .layout
                .position_for_index(rv.display.to_disp(dest))
                .unwrap();
            let wrapped = rv
                .layout
                .line_layout_for_index(rv.display.to_disp(dest))
                .unwrap();
            let font_box = wrapped.unwrapped_layout.ascent + wrapped.unwrapped_layout.descent;
            let padding = (rv.layout.line_height() - font_box) / 2.0;
            baseline.y.as_f32() - row_b.origin.y.as_f32() + padding.as_f32()
        });
        assert!(
            (got_x - want_x).abs() <= 1.5,
            "caret x {got_x} must match painted glyph x {want_x}"
        );
        assert!(
            (got_y - want_y).abs() <= 1.5,
            "caret y {got_y} must match painted glyph y {want_y}"
        );
    }

    /// Open a note of `body` in a window of `width`×`height` and return the
    /// window handle plus the temp dir (kept alive by the caller).
    fn open_note_in(
        cx: &mut TestAppContext,
        width: f32,
        height: f32,
        body: &str,
    ) -> (
        Entity<MekuShell>,
        gpui_kit::AnyWindowHandle,
        tempfile::TempDir,
    ) {
        let (view, window) = boot_sized(cx, width, height);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("doc.md"), body).unwrap();
        let rel = PathBuf::from("doc.md");
        let root = dir.path().to_path_buf();
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_mekuto(&root, cx));
        });
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });
        (view, window, dir)
    }

    /// Chrome must fill the window: title bar on top, status bar pinned to
    /// the bottom edge, document area filling everything between them.
    fn assert_shell_geometry(cx: &mut TestAppContext, width: f32, height: f32, body: &str) {
        let (view, window, dir) = open_note_in(cx, width, height, body);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let status = window.find("status-bar").bounds();
            let status_top = status.origin.y.as_f32();
            let status_h = status.size.height.as_f32();
            let status_bottom = status_top + status_h;
            assert!(
                (status_bottom - height).abs() <= 1.0,
                "status bar bottom {status_bottom} must sit on the {height}px window edge"
            );
            assert!(
                (20.0..40.0).contains(&status_h),
                "status bar height {status_h} should be a thin strip"
            );

            let doc = window.find("md-scroll").bounds();
            let doc_bottom = doc.origin.y.as_f32() + doc.size.height.as_f32();
            assert!(
                (doc_bottom - status_top).abs() <= 1.0,
                "document area must end where the status bar starts ({doc_bottom} vs {status_top})"
            );
            // Title bar is 34px; the tab bar sits above the document inside
            // the centre column, so the document must start below both and
            // can never be taller than the middle region.
            let middle_h = status_top - 34.0;
            assert!(
                doc.size.height.as_f32() <= middle_h + 1.0,
                "document area {} must stay inside the middle region {middle_h}",
                doc.size.height.as_f32()
            );
            assert!(
                doc.origin.y.as_f32() >= 34.0 + 20.0,
                "document must sit below the title bar and tab bar, top at {}",
                doc.origin.y.as_f32()
            );

            let tree = window.find("file-tree-scroll").bounds();
            let tree_bottom = tree.origin.y.as_f32() + tree.size.height.as_f32();
            assert!(
                tree_bottom <= status_top + 1.0,
                "sidebar tree must stay above the status bar ({tree_bottom} vs {status_top})"
            );
        })
        .unwrap();
        let _ = (view, dir);
    }

    #[test]
    fn shell_fills_window_at_1200x800() {
        run_ui_test("shell_1200", shell_1200_body);
    }

    fn shell_1200_body(cx: &mut TestAppContext) {
        assert_shell_geometry(cx, 1200.0, 800.0, "# T\n\nbody here\n");
    }

    #[test]
    fn shell_fills_window_at_1158x763() {
        run_ui_test("shell_1158", shell_1158_body);
    }

    fn shell_1158_body(cx: &mut TestAppContext) {
        assert_shell_geometry(cx, 1158.0, 763.0, "# T\n\nbody here\n");
    }

    #[test]
    fn long_note_scrolls_inside_middle_region() {
        run_ui_test("long_note_scrolls", long_note_body);
    }

    fn long_note_body(cx: &mut TestAppContext) {
        // 60 paragraphs would grow far past an 800px window if the document
        // area were not bounded; it must stay inside the middle and scroll.
        let body: String = (0..60)
            .map(|i| format!("Paragraph {i} text.\n\n"))
            .collect();
        assert_shell_geometry(cx, 1158.0, 763.0, &body);
    }

    // -- Build 5: modes, code highlighting, images, scroll-sync -------------

    fn doc_mode(cx: &mut TestAppContext, view: &Entity<MekuShell>) -> Mode {
        cx.update(|cx| view.read_with(cx, |shell, _| shell.tabs[shell.active_tab].mode))
    }

    fn cycle_with_key(
        cx: &mut TestAppContext,
        view: &Entity<MekuShell>,
        window: gpui_kit::AnyWindowHandle,
    ) {
        focus_doc_for(cx, &view, window);
        cx.update_window(window, |_, window, cx| window.press("ctrl-e", cx))
            .unwrap();
    }

    #[test]
    fn read_mode_hides_caret_and_blocks_typing() {
        run_ui_test("read_mode_blocks", read_mode_blocks_body);
    }

    fn read_mode_blocks_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "# T\n\nbody here\n");

        // Live -> Read.
        cycle_with_key(cx, &view, window);
        assert_eq!(doc_mode(cx, &view), Mode::Read);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("doc-caret").is_none(), "no caret in Read");
            window.input("typed ", cx);
        })
        .unwrap();
        let (text, dirty) = buffer_text(cx, &view, &PathBuf::from("doc.md"));
        assert_eq!(text, "# T\n\nbody here\n", "Read mode must not edit");
        assert!(!dirty, "Read mode must not mark the buffer dirty");

        // Live -> Read -> Edit -> Live: the cycle wraps back.
        cycle_with_key(cx, &view, window);
        assert_eq!(doc_mode(cx, &view), Mode::Edit);
        cycle_with_key(cx, &view, window);
        assert_eq!(doc_mode(cx, &view), Mode::Live);

        // Buttons flip modes too; the roll starts from Live again.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("mode-read", cx);
        })
        .unwrap();
        assert_eq!(doc_mode(cx, &view), Mode::Read);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("mode-edit", cx);
        })
        .unwrap();
        assert_eq!(doc_mode(cx, &view), Mode::Edit);
    }

    #[test]
    fn mode_cycling_keeps_the_row_in_view() {
        run_ui_test("scroll_sync", scroll_sync_body);
    }

    fn scroll_sync_body(cx: &mut TestAppContext) {
        // 80 rows: far past the viewport, so Live scrolling matters.
        let body: String = (0..80).map(|i| format!("Row {i}\n")).collect();
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, &body);
        let rel = PathBuf::from("doc.md");
        let tail = body.len();
        focus_doc_for(cx, &view, window);
        // Put the caret on the last row, then End: the raw row scrolls in.
        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                if let Some(buffer) = shell.buffers.get(&rel).cloned() {
                    buffer.update(cx, |b, _| b.move_caret(tail, false));
                }
            });
        });
        // The list refines its height estimates while scrolling, so a few
        // frames are needed before the tail settles in view.
        for step in 0..4 {
            cx.update_window(window, |_, window, cx| {
                if step > 0 {
                    window.press("end", cx);
                }
                window.render_frame(cx);
            })
            .unwrap();
        }
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find(("doc-row", 80usize)).is_some(),
                "Live scrolled the tail row in",
            );
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| {
            window.click("mode-read", cx);
            window.render_frame(cx);
            assert!(
                window.try_find(("doc-row", 80usize)).is_some(),
                "Read mode keeps the tail row in view",
            );
            window.click("mode-edit", cx);
            window.render_frame(cx);
            assert!(
                window.try_find(("doc-row", 80usize)).is_some(),
                "Edit mode keeps the tail row in view",
            );
        })
        .unwrap();
    }

    #[test]
    fn images_draw_when_their_row_is_hidden() {
        run_ui_test("image_rows", image_rows_body);
    }

    fn image_rows_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(
            cx,
            1200.0,
            800.0,
            "# Title\n\n![pic](assets/pic.png)\n\nafter\n",
        );
        // Caret starts on the heading: the image row (2) is hidden -> drawn.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find(("image-row", 2usize)).is_some(),
                "hidden image row draws the picture element"
            );
        })
        .unwrap();
        // In Edit mode the row shows raw text instead.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("mode-edit", cx);
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find(("image-row", 2usize)).is_none(),
                "Edit mode shows raw image markup, not the picture"
            );
            assert!(
                window.try_find(("doc-row", 2usize)).is_some(),
                "Edit mode shows the raw markup row"
            );
        })
        .unwrap();
    }

    #[test]
    fn code_fences_get_syntax_spans() {
        run_ui_test("code_spans", code_spans_body);
    }

    fn code_spans_body(cx: &mut TestAppContext) {
        let (view, _window, _dir) =
            open_note_in(cx, 1200.0, 800.0, "```rust\nlet x = 1;\n```\nafter\n");
        let (hl, kinds) = cx.update(|cx| {
            view.update(cx, |shell, cx| {
                let model = shell.doc_model(&PathBuf::from("doc.md"), cx).unwrap();
                (
                    model.hl,
                    model.rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
                )
            })
        });
        assert_eq!(
            kinds,
            vec![
                meku_markdown::live::RowKind::Code,
                meku_markdown::live::RowKind::Code,
                meku_markdown::live::RowKind::Code,
                meku_markdown::live::RowKind::Text,
                meku_markdown::live::RowKind::Blank,
            ]
        );
        assert!(hl[0].is_none() && hl[2].is_none() && hl[3].is_none() && hl[4].is_none());
        let spans = hl[1]
            .clone()
            .unwrap_or_else(|| panic!("rust fence must highlight"));
        let text = "let x = 1;";
        assert!(
            spans
                .iter()
                .any(|(r, t)| t == &meku_markdown::highlight::Tok::Keyword
                    && &text[r.clone()] == "let"),
            "keyword `let` should be a token: {spans:?}"
        );
    }
}
