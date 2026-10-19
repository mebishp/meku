mod chrome;
mod document;
mod explorer;
mod find;
mod icons;
mod lightbox;
mod palette;
mod slash;
mod theme;

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
use gpui_kit::component::*;
use gpui_kit::*;
// InteractiveElement (div().on_click / overflow_y_scroll) is not in the globs.
use gpui_kit::InteractiveElement;
use meku_buffer::OpenBuffer;
use meku_markdown::live::title_to_stem;
use meku_vault::{OpenTab, Session, VaultEvent};

actions!(
    meku,
    [
        ToggleSidebar,
        CycleTheme,
        NavBack,
        NavForward,
        NewTab,
        CloseTab,
        NextTab,
        OpenFolder,
        CancelOverlay,
        Save,
        CycleMode,
        DocLeft,
        DocRight,
        DocWordLeft,
        DocWordRight,
        DocUp,
        DocDown,
        DocHome,
        DocEnd,
        DocDocStart,
        DocDocEnd,
        DocPageUp,
        DocPageDown,
        DocSelectLeft,
        DocSelectRight,
        DocSelectWordLeft,
        DocSelectWordRight,
        DocSelectUp,
        DocSelectDown,
        DocSelectHome,
        DocSelectEnd,
        DocSelectDocStart,
        DocSelectDocEnd,
        DocSelectPageUp,
        DocSelectPageDown,
        DocBackspace,
        DocDelete,
        DocDeleteWordBack,
        DocDeleteWordFwd,
        DocDeleteToLineStart,
        DocMoveLineUp,
        DocMoveLineDown,
        DocDuplicateLine,
        DocDeleteLine,
        DocInsertLineBelow,
        DocInsertLineAbove,
        DocJoinLines,
        DocToggleComment,
        DocSoftBreak,
        DocBold,
        DocItalic,
        DocStrike,
        DocHighlight,
        DocInlineCode,
        DocQuote,
        DocExpand,
        DocH0,
        DocH1,
        DocH2,
        DocH3,
        DocH4,
        DocH5,
        DocH6,
        DocLink,
        DocIndent,
        DocOutdent,
        DocListBullet,
        DocListOrdered,
        DocListTask,
        DocToggleTask,
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
    /// Width document rows wrap at (set while painting; used by the caret).
    doc_wrap: Rc<std::cell::Cell<Pixels>>,
    doc_tables: document::TableCache,
    /// Fence row whose code was just copied (button shows "Copied").
    copied: Option<(usize, Instant)>,
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
    /// Caret blink phase and the last keystroke/click (blink holds solid
    /// for 600ms after input).
    caret_on: bool,
    caret_touch: Instant,
    /// Row whose rendered link the pointer is over (pointing-hand cursor).
    hover_link: Option<usize>,
    /// Notes visited, oldest first, and the position in them (back/forward).
    nav_history: Vec<PathBuf>,
    nav_pos: usize,
    navigating: bool,
    /// Image shown in the zoom view, and its zoom (`None` = fit to window).
    lightbox: Option<document::ImageInfo>,
    lightbox_zoom: Option<f32>,
    /// Header-probed pixel sizes of local images.
    image_dims: document::DimCache,
    /// Scroll state of the tab strip, and the tab it was last scrolled to.
    tabs_scroll: ScrollHandle,
    tabs_scrolled_to: Option<usize>,
    /// Outline section expanded in the sidebar.
    outline_open: bool,
    palette: Option<palette::PaletteState>,
    slash: Option<slash::SlashState>,
    /// `/` offset whose menu was closed with Esc.
    slash_dismissed: Option<usize>,
    find: Option<find::FindState>,
    /// Open the find bar on the next render (`true` = with replace).
    find_request: Option<bool>,
    palette_request: Option<palette::PaletteKind>,
    /// Command chosen in the palette, run on the next render (needs a window).
    run_after: Option<palette::Command>,
    /// Row to scroll into view on the next render (startup/screenshot hook).
    pending_scroll: Option<(usize, u8)>,
    /// The inline title above each open note's body (not part of the
    /// markdown): shows the file stem, and editing it renames the file.
    title_inputs: HashMap<PathBuf, Entity<InputState>>,
    title_subs: HashMap<PathBuf, Subscription>,
    /// Note whose title was edited and when; committed after 800ms idle.
    title_pending: Option<(PathBuf, Instant)>,
    /// Titles to put back to the file stem on the next render (rejected or
    /// emptied edits).
    title_resync: Vec<PathBuf>,
    /// Focus the title of this note (selecting it) on the next render.
    focus_title: Option<PathBuf>,
    /// Move keyboard focus to the document on the next render.
    focus_doc: bool,
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
            doc_wrap: Rc::new(std::cell::Cell::new(px(0.))),
            doc_tables: Rc::new(RefCell::new((u64::MAX, HashMap::new()))),
            copied: None,
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
            caret_on: true,
            caret_touch: Instant::now(),
            hover_link: None,
            nav_history: Vec::new(),
            nav_pos: 0,
            navigating: false,
            lightbox: None,
            lightbox_zoom: None,
            image_dims: HashMap::new(),
            tabs_scroll: ScrollHandle::new(),
            tabs_scrolled_to: None,
            outline_open: true,
            palette: None,
            slash: None,
            slash_dismissed: None,
            find: None,
            find_request: None,
            palette_request: None,
            run_after: None,
            pending_scroll: None,
            title_inputs: HashMap::new(),
            title_subs: HashMap::new(),
            title_pending: None,
            title_resync: Vec::new(),
            focus_title: None,
            focus_doc: false,
        };
        this.start_watcher_pump(cx);
        this.start_caret_blink(cx);
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

    /// Startup overrides from the environment (see `main`).
    fn apply_startup_state(&mut self, cx: &mut Context<Self>) {
        let mode = match std::env::var("MEKU_MODE").as_deref() {
            Ok("edit") => Some(Mode::Edit),
            Ok("live") => Some(Mode::Live),
            Ok("read") => Some(Mode::Read),
            _ => None,
        };
        if let (Some(mode), Some(tab)) = (mode, self.tabs.get_mut(self.active_tab)) {
            tab.mode = mode;
        }
        if let Some(at) = std::env::var("MEKU_CARET")
            .ok()
            .and_then(|v| v.parse().ok())
            && let Some(rel) = self.doc_rel()
            && let Some(buffer) = self.buffers.get(&rel).cloned()
        {
            buffer.update(cx, |b, _| b.move_caret(at, false));
        }
        self.update_slash(false, cx);
        self.pending_scroll = std::env::var("MEKU_ROW")
            .ok()
            .and_then(|v| v.parse().ok())
            .map(|row| (row, 3));
        if std::env::var("MEKU_LIGHTBOX").is_ok()
            && let Some(rel) = self.doc_rel()
            && let Some(model) = self.doc_model(&rel, cx)
            && let Some(info) = model.images.iter().flatten().next().cloned()
        {
            self.open_lightbox(info, cx);
            self.lightbox_zoom = std::env::var("MEKU_ZOOM").ok().and_then(|v| v.parse().ok());
        }
        if std::env::var("MEKU_FIND").is_ok() {
            self.open_find(std::env::var("MEKU_REPLACE").is_ok(), cx);
        }
        match std::env::var("MEKU_PALETTE").as_deref() {
            Ok("files") => self.open_palette(palette::PaletteKind::Files, cx),
            Ok("commands") => self.open_palette(palette::PaletteKind::Commands, cx),
            Ok("themes") => self.open_palette(palette::PaletteKind::Themes, cx),
            Ok("search") => self.open_palette(palette::PaletteKind::Search, cx),
            _ => {}
        }
        if std::env::var("MEKU_MODAL").as_deref() == Ok("folder") {
            self.begin_naming(NamingMode::NewFolder, PathBuf::new(), None, cx);
        }
        cx.notify();
    }

    /// Stepped caret blink: 530ms on, 530ms off, held solid for 600ms after
    /// any input. Only re-renders while a note is open outside Read mode.
    fn start_caret_blink(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(530))
                    .await;
                let Some(view) = this.upgrade() else {
                    break;
                };
                cx.update(|cx| {
                    view.update(cx, |shell, cx| {
                        if shell.doc_rel().is_none() || shell.tab_mode() == Mode::Read {
                            return;
                        }
                        let held = shell.caret_touch.elapsed() < Duration::from_millis(600);
                        shell.caret_on = held || !shell.caret_on;
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    // -- inline title <-> file name ----------------------------------------

    fn stem_of(rel: &Path) -> String {
        rel.file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string()
    }

    /// Create the title input of the active note when missing, and apply
    /// queued resyncs/focus. Needs a window, so it runs from `render`.
    fn ensure_title_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rel) = self.doc_rel() else {
            return;
        };
        if !self.title_inputs.contains_key(&rel) {
            let stem = Self::stem_of(&rel);
            let input = cx.new(|cx| InputState::new(window, cx).default_value(stem));
            // Look the note up by input entity: renames move the map key.
            let sub = cx.subscribe(&input, |this, state, event: &InputEvent, cx| {
                let owner = this
                    .title_inputs
                    .iter()
                    .find(|(_, e)| **e == state)
                    .map(|(k, _)| k.clone());
                if let Some(rel) = owner {
                    this.on_title_event(&rel, event, cx);
                }
            });
            self.title_inputs.insert(rel.clone(), input);
            self.title_subs.insert(rel.clone(), sub);
        }
        for r in std::mem::take(&mut self.title_resync) {
            if let Some(input) = self.title_inputs.get(&r).cloned() {
                let stem = Self::stem_of(&r);
                input.update(cx, |state, cx| state.set_value(stem, window, cx));
            }
        }
        if self.focus_title.as_deref() == Some(rel.as_path())
            && let Some(input) = self.title_inputs.get(&rel).cloned()
        {
            self.focus_title = None;
            window.focus(&input.read(cx).focus_handle(cx), cx);
            input.update(cx, |state, cx| state.select_all(window, cx));
        }
    }

    fn on_title_event(&mut self, rel: &Path, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => {
                self.title_pending = Some((rel.to_path_buf(), Instant::now()));
            }
            InputEvent::PressEnter { .. } => {
                self.commit_title(rel, true, cx);
                self.focus_doc = true;
                cx.notify();
            }
            InputEvent::Blur => self.commit_title(rel, true, cx),
            InputEvent::Focus => {}
        }
    }

    /// Rename the note to the title text. An empty or unusable title can't
    /// remove the name: it snaps back (`force`) or waits for more typing.
    /// A name already in use is refused with a notice and snaps back too.
    fn commit_title(&mut self, rel: &Path, force: bool, cx: &mut Context<Self>) {
        if self.title_pending.as_ref().is_some_and(|(r, _)| r == rel) {
            self.title_pending = None;
        }
        let Some(input) = self.title_inputs.get(rel).cloned() else {
            return;
        };
        let typed = input.read(cx).value().to_string();
        let current = Self::stem_of(rel);
        let Some(stem) = title_to_stem(&typed) else {
            if force {
                self.title_resync.push(rel.to_path_buf());
                cx.notify();
            }
            return;
        };
        if stem == current {
            if force && typed != stem {
                self.title_resync.push(rel.to_path_buf());
                cx.notify();
            }
            return;
        }
        let ext = rel.extension().and_then(|e| e.to_str()).unwrap_or("md");
        match self.explorer.rename(rel, &format!("{stem}.{ext}")) {
            Ok(new_rel) => {
                self.retarget_tabs(rel, &new_rel, cx);
                self.explorer.refresh();
                self.explorer.reveal(&new_rel);
                self.notice = None;
                // Normalize the text (stripped characters, trailing space)
                // only on Enter/blur, never under the caret while typing.
                if force && typed != stem {
                    self.title_resync.push(new_rel);
                }
            }
            Err(e) => {
                self.notice = Some(e.to_string());
                if force {
                    self.title_resync.push(rel.to_path_buf());
                }
            }
        }
        cx.notify();
    }

    /// Commit a title edit that is still waiting for its idle timer — called
    /// whenever focus or the active note is about to change.
    pub(crate) fn flush_title(&mut self, cx: &mut Context<Self>) {
        if let Some((rel, _)) = self.title_pending.clone() {
            self.commit_title(&rel, true, cx);
        }
    }

    /// Create `Untitled.md` (`Untitled 2.md`, ...) — an empty note whose
    /// title is focused and selected, ready to be typed over.
    fn new_untitled_note(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let Some(root) = self.explorer.root().map(Path::to_path_buf) else {
            return;
        };
        let name = (1..)
            .map(|n| match n {
                1 => "Untitled".to_string(),
                n => format!("Untitled {n}"),
            })
            .find(|name| !root.join(&dir).join(format!("{name}.md")).exists())
            .unwrap_or_else(|| "Untitled".to_string());
        match self.explorer.create_note(&dir, &name) {
            Ok(rel) => {
                self.explorer.refresh();
                self.explorer.reveal(&rel);
                self.open_file_inner(&rel, cx);
                self.focus_title = Some(rel);
                self.notice = None;
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
        cx.notify();
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
        self.flush_title(cx);
        if let Some(ix) = self.tabs.iter().position(|t| t.rel.as_deref() == Some(rel)) {
            self.active_tab = ix;
            self.explorer.reveal(rel);
            self.record_visit();
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
                // The empty "Welcome" placeholder only exists until a real
                // note is opened.
                self.tabs
                    .retain(|t| !(t.rel.is_none() && t.title == "Welcome"));
                self.buffers.insert(rel.to_path_buf(), cx.new(|_| buffer));
                self.tabs.push(EditorTab {
                    rel: Some(rel.to_path_buf()),
                    title: Self::tab_title(rel),
                    mode: Mode::Live,
                });
                self.active_tab = self.tabs.len() - 1;
                self.explorer.reveal(rel);
                self.record_visit();
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
        self.new_tab_action(cx);
    }

    pub(crate) fn new_tab_action(&mut self, cx: &mut Context<Self>) {
        if self.explorer.is_open() {
            self.new_untitled_note(PathBuf::new(), cx);
            return;
        }
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
        self.flush_title(cx);
        if self.tabs.is_empty() {
            return;
        }
        let closed = self.tabs.remove(index);
        if let Some(rel) = closed.rel {
            self.title_inputs.remove(&rel);
            self.title_subs.remove(&rel);
        }
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

    /// Remember the visible note for back/forward.
    pub(crate) fn record_visit(&mut self) {
        let Some(rel) = self.doc_rel() else {
            return;
        };
        if self.navigating || self.nav_history.get(self.nav_pos) == Some(&rel) {
            return;
        }
        if !self.nav_history.is_empty() {
            self.nav_history.truncate(self.nav_pos + 1);
        }
        self.nav_history.push(rel);
        if self.nav_history.len() > 100 {
            self.nav_history.remove(0);
        }
        self.nav_pos = self.nav_history.len() - 1;
    }

    pub(crate) fn can_nav(&self, delta: isize) -> bool {
        let target = self.nav_pos as isize + delta;
        target >= 0 && (target as usize) < self.nav_history.len()
    }

    /// Back (`-1`) / forward (`1`) through visited notes; reopens closed ones.
    pub(crate) fn nav_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if !self.can_nav(delta) {
            return;
        }
        self.nav_pos = (self.nav_pos as isize + delta) as usize;
        let rel = self.nav_history[self.nav_pos].clone();
        self.navigating = true;
        self.open_file_inner(&rel, cx);
        self.navigating = false;
    }

    pub(crate) fn close_other_tabs(&mut self, keep: usize, cx: &mut Context<Self>) {
        self.flush_title(cx);
        let Some(kept) = self.tabs.get(keep).cloned() else {
            return;
        };
        self.tabs = vec![kept];
        self.active_tab = 0;
        self.save_session();
        cx.notify();
    }

    pub(crate) fn close_all_tabs(&mut self, cx: &mut Context<Self>) {
        while !self.tabs.is_empty() && self.tabs.iter().any(|t| t.rel.is_some()) {
            self.close_tab(0, cx);
        }
    }

    fn on_next_tab(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
            self.record_visit();
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
        let (dirty, snapshot, known) =
            buffer.read_with(cx, |b, _| (b.dirty, b.snapshot().1, b.disk_hash));
        // Our own save (or the content we loaded): the watcher event can
        // arrive after more typing, when the buffer no longer equals the
        // disk. That is not an outside edit.
        if disk == snapshot || known == Some(meku_buffer::hash_text(&disk)) {
            let mtime = std::fs::metadata(&abs).and_then(|m| m.modified()).ok();
            buffer.update(cx, |b, _| {
                if let Some(mtime) = mtime {
                    b.disk_mtime = Some(mtime);
                }
                b.disk_hash = Some(meku_buffer::hash_text(&disk));
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
                b.disk_hash = Some(meku_buffer::hash_text(text));
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
    fn on_doc_word_left(&mut self, _: &DocWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::WordLeft, false, window, cx);
    }
    fn on_doc_word_right(&mut self, _: &DocWordRight, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::WordRight, false, window, cx);
    }
    fn on_doc_doc_start(&mut self, _: &DocDocStart, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::DocStart, false, window, cx);
    }
    fn on_doc_doc_end(&mut self, _: &DocDocEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::DocEnd, false, window, cx);
    }
    fn on_doc_page_up(&mut self, _: &DocPageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::PageUp, false, window, cx);
    }
    fn on_doc_page_down(&mut self, _: &DocPageDown, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::PageDown, false, window, cx);
    }
    fn on_doc_select_word_left(
        &mut self,
        _: &DocSelectWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::WordLeft, true, window, cx);
    }
    fn on_doc_select_word_right(
        &mut self,
        _: &DocSelectWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::WordRight, true, window, cx);
    }
    fn on_doc_select_doc_start(
        &mut self,
        _: &DocSelectDocStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::DocStart, true, window, cx);
    }
    fn on_doc_select_doc_end(
        &mut self,
        _: &DocSelectDocEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::DocEnd, true, window, cx);
    }
    fn on_doc_select_page_up(
        &mut self,
        _: &DocSelectPageUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::PageUp, true, window, cx);
    }
    fn on_doc_select_page_down(
        &mut self,
        _: &DocSelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::PageDown, true, window, cx);
    }
    fn on_doc_delete_word_back(
        &mut self,
        _: &DocDeleteWordBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::DeleteWordBack, false, window, cx);
    }
    fn on_doc_delete_word_fwd(
        &mut self,
        _: &DocDeleteWordFwd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::DeleteWordFwd, false, window, cx);
    }
    fn on_doc_delete_to_line_start(
        &mut self,
        _: &DocDeleteToLineStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::DeleteToLineStart, false, window, cx);
    }
    fn on_doc_move_line_up(
        &mut self,
        _: &DocMoveLineUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::MoveLineUp, false, window, cx);
    }
    fn on_doc_move_line_down(
        &mut self,
        _: &DocMoveLineDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::MoveLineDown, false, window, cx);
    }
    fn on_doc_duplicate_line(
        &mut self,
        _: &DocDuplicateLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::DuplicateLine, false, window, cx);
    }
    fn on_doc_delete_line(
        &mut self,
        _: &DocDeleteLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::DeleteLine, false, window, cx);
    }
    fn on_doc_insert_line_below(
        &mut self,
        _: &DocInsertLineBelow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::InsertLineBelow, false, window, cx);
    }
    fn on_doc_insert_line_above(
        &mut self,
        _: &DocInsertLineAbove,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::InsertLineAbove, false, window, cx);
    }
    fn on_doc_join_lines(&mut self, _: &DocJoinLines, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::JoinLines, false, window, cx);
    }
    fn on_doc_toggle_comment(
        &mut self,
        _: &DocToggleComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::ToggleComment, false, window, cx);
    }
    fn on_doc_soft_break(&mut self, _: &DocSoftBreak, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::SoftBreak, false, window, cx);
    }
    fn on_doc_link(&mut self, _: &DocLink, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Link, false, window, cx);
    }
    fn on_doc_indent(&mut self, _: &DocIndent, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Indent, false, window, cx);
    }
    fn on_doc_outdent(&mut self, _: &DocOutdent, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Outdent, false, window, cx);
    }
    fn on_doc_list_bullet(
        &mut self,
        _: &DocListBullet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::ListBullet, false, window, cx);
    }
    fn on_doc_list_ordered(
        &mut self,
        _: &DocListOrdered,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::ListOrdered, false, window, cx);
    }
    fn on_doc_list_task(&mut self, _: &DocListTask, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::ListTask, false, window, cx);
    }
    fn on_doc_toggle_task(
        &mut self,
        _: &DocToggleTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::ToggleTask, false, window, cx);
    }
    fn on_doc_bold(&mut self, _: &DocBold, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Wrap("**"), false, window, cx);
    }
    fn on_doc_italic(&mut self, _: &DocItalic, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Wrap("*"), false, window, cx);
    }
    fn on_doc_strike(&mut self, _: &DocStrike, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Wrap("~~"), false, window, cx);
    }
    fn on_doc_highlight(&mut self, _: &DocHighlight, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Wrap("=="), false, window, cx);
    }
    fn on_doc_inline_code(
        &mut self,
        _: &DocInlineCode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc_key(document::DocKey::Wrap("`"), false, window, cx);
    }
    fn on_doc_quote(&mut self, _: &DocQuote, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Quote, false, window, cx);
    }
    fn on_doc_expand(&mut self, _: &DocExpand, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Expand, false, window, cx);
    }
    fn on_doc_h0(&mut self, _: &DocH0, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Heading(0), false, window, cx);
    }
    fn on_doc_h1(&mut self, _: &DocH1, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Heading(1), false, window, cx);
    }
    fn on_doc_h2(&mut self, _: &DocH2, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Heading(2), false, window, cx);
    }
    fn on_doc_h3(&mut self, _: &DocH3, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Heading(3), false, window, cx);
    }
    fn on_doc_h4(&mut self, _: &DocH4, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Heading(4), false, window, cx);
    }
    fn on_doc_h5(&mut self, _: &DocH5, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Heading(5), false, window, cx);
    }
    fn on_doc_h6(&mut self, _: &DocH6, window: &mut Window, cx: &mut Context<Self>) {
        self.doc_key(document::DocKey::Heading(6), false, window, cx);
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
        self.flush_title(cx);
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
                let hash = meku_buffer::hash_text(&text);
                buffer.update(cx, |b, _| {
                    b.dirty = false;
                    b.disk_hash = Some(hash);
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
        let idle_title = self
            .title_pending
            .as_ref()
            .filter(|(_, at)| now.duration_since(*at) > Duration::from_millis(800))
            .map(|(rel, _)| rel.clone());
        if let Some(rel) = idle_title {
            self.commit_title(&rel, false, cx);
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
            // Remember this disk content as seen, so repeated watcher events
            // for it do not raise the banner again.
            if let Ok(disk) = std::fs::read_to_string(&abs) {
                let hash = meku_buffer::hash_text(&disk);
                buffer.update(cx, |b, _| b.disk_hash = Some(hash));
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
        if mode == NamingMode::NewNote {
            // No dialog: the note appears as `Untitled` with its heading
            // selected, and renaming happens by editing that heading.
            self.new_untitled_note(dir, cx);
            return;
        }
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

    /// `new` plus `suffix`, without the trailing separator `join("")` adds
    /// (a file moved as itself has an empty suffix).
    fn join_suffix(new: &Path, suffix: &Path) -> PathBuf {
        if suffix.as_os_str().is_empty() {
            new.to_path_buf()
        } else {
            new.join(suffix)
        }
    }

    fn retarget_tabs(&mut self, old: &Path, new: &Path, cx: &mut Context<Self>) {
        for tab in &mut self.tabs {
            if let Some(rel) = &tab.rel
                && (rel == old || rel.starts_with(old))
            {
                let suffix = rel.strip_prefix(old).unwrap_or(Path::new(""));
                tab.rel = Some(Self::join_suffix(new, suffix));
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
                let moved = Self::join_suffix(new, suffix);
                entity.update(cx, |buffer, _| buffer.rel = moved.clone());
                self.buffers.insert(moved.clone(), entity);
                if let Some(input) = self.title_inputs.remove(&key) {
                    self.title_inputs.insert(moved.clone(), input);
                }
                if let Some(sub) = self.title_subs.remove(&key) {
                    self.title_subs.insert(moved.clone(), sub);
                }
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
        if self.close_lightbox(cx) {
            return;
        }
        if self.slash_dismiss() {
            cx.notify();
        } else if self.palette_open() || self.naming.is_some() || self.pending_delete.is_some() {
            self.close_palette(true, cx);
        } else {
            self.close_find(cx);
        }
        self.cancel_naming(cx);
        self.cancel_delete(cx);
    }

    fn on_open_file_finder(
        &mut self,
        _: &palette::OpenFileFinder,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(palette::PaletteKind::Files, cx);
    }
    fn on_open_command_palette(
        &mut self,
        _: &palette::OpenCommandPalette,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(palette::PaletteKind::Commands, cx);
    }
    fn on_open_vault_search(
        &mut self,
        _: &palette::OpenVaultSearch,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(palette::PaletteKind::Search, cx);
    }
    fn on_open_go_to_line(
        &mut self,
        _: &palette::OpenGoToLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(palette::PaletteKind::GotoLine, cx);
    }

    fn on_open_find(&mut self, _: &find::OpenFind, _: &mut Window, cx: &mut Context<Self>) {
        self.open_find(false, cx);
    }
    fn on_open_replace(&mut self, _: &find::OpenReplace, _: &mut Window, cx: &mut Context<Self>) {
        self.open_find(true, cx);
    }
    fn on_find_next(&mut self, _: &find::FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.find_step(1, cx);
    }
    fn on_find_prev(&mut self, _: &find::FindPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.find_step(-1, cx);
    }

    fn on_nav_back(&mut self, _: &NavBack, _: &mut Window, cx: &mut Context<Self>) {
        self.nav_step(-1, cx);
    }
    fn on_nav_forward(&mut self, _: &NavForward, _: &mut Window, cx: &mut Context<Self>) {
        self.nav_step(1, cx);
    }

    fn on_cycle_theme(&mut self, _: &CycleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let current = theme::active(cx).name;
        let at = theme::PRESETS
            .iter()
            .position(|p| p.name == current)
            .unwrap_or(0);
        theme::choose(theme::PRESETS[(at + 1) % theme::PRESETS.len()].name, cx);
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.show_sidebar = !self.show_sidebar;
        cx.notify();
    }

    // -- chrome ----------------------------------------------------------

    fn render_center(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_strip = self.render_tab_strip(cx);
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
                    .child(self.render_document(&rel, window, cx))
                    .into_any_element()
            }
            _ => self.render_welcome(cx),
        };

        // `min_w_0` + clipping keep a long tab strip from widening the pane
        // (and with it the whole window); the strip scrolls sideways instead.
        v_flex()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .child(tab_strip)
            .child(v_flex().flex_1().min_h_0().min_w_0().child(body))
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
                        "{} changed outside the editor. Reload it, or keep your version.",
                        rel.display()
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("conflict-keep")
                            .label("Keep my version")
                            .ghost()
                            .xsmall()
                            .on_click(move |_, _, cx| {
                                keep_shell.update(cx, |view, cx| view.resolve_keep_mine(cx));
                            }),
                    )
                    .child(
                        Button::new("conflict-load")
                            .label("Reload")
                            .primary()
                            .xsmall()
                            .on_click(move |_, _, cx| {
                                load_shell.update(cx, |view, cx| view.resolve_load_disk(cx));
                            }),
                    ),
            )
            .into_any_element()
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
                // The scrim is its own layer; opacity on a parent would also
                // fade the dialog card inside it.
                .child(div().absolute().inset_0().bg(theme::scrim(cx)))
                .child(
                    div()
                        .w(px(360.))
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background)
                        .p_4()
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
                // The scrim is its own layer; opacity on a parent would also
                // fade the dialog card inside it.
                .child(div().absolute().inset_0().bg(theme::scrim(cx)))
                .child(
                    div()
                        .w(px(360.))
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background)
                        .p_4()
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
        self.ensure_title_input(window, cx);
        self.ensure_palette(window, cx);
        self.ensure_find(window, cx);
        let focus = self.focus_handle.clone();
        if std::mem::take(&mut self.focus_doc) && self.naming.is_none() {
            window.focus(&focus, cx);
        }
        let mut root = v_flex()
            .size_full()
            .relative()
            .track_focus(&focus)
            .key_context("Meku")
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::on_cycle_theme))
            .on_action(cx.listener(Self::on_nav_back))
            .on_action(cx.listener(Self::on_nav_forward))
            .on_action(cx.listener(Self::on_open_find))
            .on_action(cx.listener(Self::on_open_replace))
            .on_action(cx.listener(Self::on_find_next))
            .on_action(cx.listener(Self::on_find_prev))
            .on_action(cx.listener(Self::on_open_file_finder))
            .on_action(cx.listener(Self::on_open_command_palette))
            .on_action(cx.listener(Self::on_open_vault_search))
            .on_action(cx.listener(Self::on_open_go_to_line))
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_open_folder))
            .on_action(cx.listener(Self::on_cancel_overlay))
            .on_action(cx.listener(Self::on_cycle_mode))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_doc_left))
            .on_action(cx.listener(Self::on_doc_right))
            .on_action(cx.listener(Self::on_doc_word_left))
            .on_action(cx.listener(Self::on_doc_word_right))
            .on_action(cx.listener(Self::on_doc_doc_start))
            .on_action(cx.listener(Self::on_doc_doc_end))
            .on_action(cx.listener(Self::on_doc_page_up))
            .on_action(cx.listener(Self::on_doc_page_down))
            .on_action(cx.listener(Self::on_doc_up))
            .on_action(cx.listener(Self::on_doc_down))
            .on_action(cx.listener(Self::on_doc_home))
            .on_action(cx.listener(Self::on_doc_end))
            .on_action(cx.listener(Self::on_doc_select_left))
            .on_action(cx.listener(Self::on_doc_select_right))
            .on_action(cx.listener(Self::on_doc_select_word_left))
            .on_action(cx.listener(Self::on_doc_select_word_right))
            .on_action(cx.listener(Self::on_doc_select_doc_start))
            .on_action(cx.listener(Self::on_doc_select_doc_end))
            .on_action(cx.listener(Self::on_doc_select_page_up))
            .on_action(cx.listener(Self::on_doc_select_page_down))
            .on_action(cx.listener(Self::on_doc_select_up))
            .on_action(cx.listener(Self::on_doc_select_down))
            .on_action(cx.listener(Self::on_doc_select_home))
            .on_action(cx.listener(Self::on_doc_select_end))
            .on_action(cx.listener(Self::on_doc_backspace))
            .on_action(cx.listener(Self::on_doc_delete))
            .on_action(cx.listener(Self::on_doc_delete_word_back))
            .on_action(cx.listener(Self::on_doc_delete_word_fwd))
            .on_action(cx.listener(Self::on_doc_delete_to_line_start))
            .on_action(cx.listener(Self::on_doc_move_line_up))
            .on_action(cx.listener(Self::on_doc_move_line_down))
            .on_action(cx.listener(Self::on_doc_duplicate_line))
            .on_action(cx.listener(Self::on_doc_delete_line))
            .on_action(cx.listener(Self::on_doc_insert_line_below))
            .on_action(cx.listener(Self::on_doc_insert_line_above))
            .on_action(cx.listener(Self::on_doc_join_lines))
            .on_action(cx.listener(Self::on_doc_toggle_comment))
            .on_action(cx.listener(Self::on_doc_soft_break))
            .on_action(cx.listener(Self::on_doc_link))
            .on_action(cx.listener(Self::on_doc_bold))
            .on_action(cx.listener(Self::on_doc_italic))
            .on_action(cx.listener(Self::on_doc_strike))
            .on_action(cx.listener(Self::on_doc_highlight))
            .on_action(cx.listener(Self::on_doc_inline_code))
            .on_action(cx.listener(Self::on_doc_quote))
            .on_action(cx.listener(Self::on_doc_expand))
            .on_action(cx.listener(Self::on_doc_h0))
            .on_action(cx.listener(Self::on_doc_h1))
            .on_action(cx.listener(Self::on_doc_h2))
            .on_action(cx.listener(Self::on_doc_h3))
            .on_action(cx.listener(Self::on_doc_h4))
            .on_action(cx.listener(Self::on_doc_h5))
            .on_action(cx.listener(Self::on_doc_h6))
            .on_action(cx.listener(Self::on_doc_indent))
            .on_action(cx.listener(Self::on_doc_outdent))
            .on_action(cx.listener(Self::on_doc_list_bullet))
            .on_action(cx.listener(Self::on_doc_list_ordered))
            .on_action(cx.listener(Self::on_doc_list_task))
            .on_action(cx.listener(Self::on_doc_toggle_task))
            .on_action(cx.listener(Self::on_doc_enter))
            .on_action(cx.listener(Self::on_doc_select_all))
            .on_action(cx.listener(Self::on_doc_undo))
            .on_action(cx.listener(Self::on_doc_redo))
            .on_action(cx.listener(Self::on_doc_copy))
            .on_action(cx.listener(Self::on_doc_cut))
            .on_action(cx.listener(Self::on_doc_paste))
            .child(TitleBar::new().child(self.render_title_content(cx)))
            .child(
                v_flex().flex_1().min_h_0().overflow_hidden().child(
                    h_resizable("meku-shell")
                        .child(
                            resizable_panel()
                                .visible(self.show_sidebar)
                                .size(px(240.))
                                .size_range(px(200.)..px(400.))
                                .child(self.render_sidebar_panel(cx)),
                        )
                        .child(self.render_center(window, cx).into_any_element()),
                ),
            )
            .child(self.render_status(cx));
        if let Some(modal) = self.render_naming_modal(cx) {
            root = root.child(theme::fade_in(
                div().absolute().inset_0().child(modal),
                "naming-in",
                140,
                0.0,
                cx,
            ));
        } else if let Some(modal) = self.render_delete_modal(cx) {
            root = root.child(theme::fade_in(
                div().absolute().inset_0().child(modal),
                "delete-in",
                140,
                0.0,
                cx,
            ));
        } else if let Some(palette) = self.render_palette(cx) {
            root = root.child(palette);
        }
        if let Some(view) = self.render_lightbox(window, cx) {
            root = root.child(view);
        }
        root
    }
}

/// Prefer a clean humanist sans for the interface/document and a coding
/// mono for code, using whichever of the candidates is installed; anything
/// else keeps the theme default.
fn apply_fonts(cx: &mut App) {
    let installed = cx.text_system().all_font_names();
    let pick = |candidates: &[&str]| -> Option<SharedString> {
        candidates
            .iter()
            .find(|c| installed.iter().any(|n| n == **c))
            .map(|c| SharedString::from(c.to_string()))
    };
    let sans = pick(&["Inter", "Adwaita Sans", "Cantarell", "Liberation Sans"]);
    let mono = pick(&[
        "JetBrains Mono",
        "Adwaita Mono",
        "DejaVu Sans Mono",
        "Liberation Mono",
    ]);
    let theme = cx.global_mut::<Theme>();
    if let Some(sans) = sans {
        theme.font_family = sans;
    }
    if let Some(mono) = mono {
        theme.mono_font_family = mono;
    }
}

fn main() {
    let initial_dir = std::env::args().nth(1);
    let app = gpui_kit::application().with_assets(icons::MekuAssets);

    app.run(move |cx| {
        // Required: initializes gpui-component theming/global state.
        gpui_kit::init(cx);
        apply_fonts(cx);
        theme::init(cx);

        cx.bind_keys([
            KeyBinding::new("ctrl-\\", ToggleSidebar, Some("Meku")),
            KeyBinding::new("ctrl-p", palette::OpenFileFinder, Some("Meku")),
            KeyBinding::new("ctrl-shift-p", palette::OpenCommandPalette, Some("Meku")),
            KeyBinding::new("ctrl-shift-f", palette::OpenVaultSearch, Some("Meku")),
            KeyBinding::new("ctrl-g", palette::OpenGoToLine, Some("Meku")),
            KeyBinding::new("ctrl-f", find::OpenFind, Some("Meku")),
            KeyBinding::new("ctrl-h", find::OpenReplace, Some("Meku")),
            KeyBinding::new("f3", find::FindNext, Some("Meku")),
            KeyBinding::new("shift-f3", find::FindPrev, Some("Meku")),
            KeyBinding::new("up", palette::PaletteUp, Some("Palette")),
            KeyBinding::new("down", palette::PaletteDown, Some("Palette")),
            KeyBinding::new("ctrl-alt-t", CycleTheme, Some("Meku")),
            KeyBinding::new("alt-left", NavBack, Some("Meku")),
            KeyBinding::new("alt-right", NavForward, Some("Meku")),
            KeyBinding::new("ctrl-t", NewTab, Some("Meku")),
            KeyBinding::new("ctrl-w", CloseTab, Some("Meku")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Meku")),
            KeyBinding::new("ctrl-o", OpenFolder, Some("Meku")),
            KeyBinding::new("ctrl-s", Save, Some("Meku")),
            KeyBinding::new("ctrl-e", CycleMode, Some("Meku")),
            KeyBinding::new("escape", CancelOverlay, None),
            KeyBinding::new("left", DocLeft, Some("Meku")),
            KeyBinding::new("right", DocRight, Some("Meku")),
            KeyBinding::new("ctrl-left", DocWordLeft, Some("Meku")),
            KeyBinding::new("ctrl-right", DocWordRight, Some("Meku")),
            KeyBinding::new("up", DocUp, Some("Meku")),
            KeyBinding::new("down", DocDown, Some("Meku")),
            KeyBinding::new("home", DocHome, Some("Meku")),
            KeyBinding::new("end", DocEnd, Some("Meku")),
            KeyBinding::new("ctrl-home", DocDocStart, Some("Meku")),
            KeyBinding::new("ctrl-end", DocDocEnd, Some("Meku")),
            KeyBinding::new("pageup", DocPageUp, Some("Meku")),
            KeyBinding::new("pagedown", DocPageDown, Some("Meku")),
            KeyBinding::new("shift-left", DocSelectLeft, Some("Meku")),
            KeyBinding::new("shift-right", DocSelectRight, Some("Meku")),
            KeyBinding::new("ctrl-shift-left", DocSelectWordLeft, Some("Meku")),
            KeyBinding::new("ctrl-shift-right", DocSelectWordRight, Some("Meku")),
            KeyBinding::new("shift-up", DocSelectUp, Some("Meku")),
            KeyBinding::new("shift-down", DocSelectDown, Some("Meku")),
            KeyBinding::new("shift-home", DocSelectHome, Some("Meku")),
            KeyBinding::new("shift-end", DocSelectEnd, Some("Meku")),
            KeyBinding::new("ctrl-shift-home", DocSelectDocStart, Some("Meku")),
            KeyBinding::new("ctrl-shift-end", DocSelectDocEnd, Some("Meku")),
            KeyBinding::new("shift-pageup", DocSelectPageUp, Some("Meku")),
            KeyBinding::new("shift-pagedown", DocSelectPageDown, Some("Meku")),
            KeyBinding::new("backspace", DocBackspace, Some("Meku")),
            KeyBinding::new("delete", DocDelete, Some("Meku")),
            KeyBinding::new("ctrl-backspace", DocDeleteWordBack, Some("Meku")),
            KeyBinding::new("ctrl-delete", DocDeleteWordFwd, Some("Meku")),
            KeyBinding::new("ctrl-shift-backspace", DocDeleteToLineStart, Some("Meku")),
            KeyBinding::new("alt-up", DocMoveLineUp, Some("Meku")),
            KeyBinding::new("alt-down", DocMoveLineDown, Some("Meku")),
            KeyBinding::new("ctrl-shift-d", DocDuplicateLine, Some("Meku")),
            KeyBinding::new("ctrl-shift-k", DocDeleteLine, Some("Meku")),
            KeyBinding::new("ctrl-enter", DocInsertLineBelow, Some("Meku")),
            KeyBinding::new("ctrl-shift-enter", DocInsertLineAbove, Some("Meku")),
            KeyBinding::new("ctrl-j", DocJoinLines, Some("Meku")),
            KeyBinding::new("ctrl-/", DocToggleComment, Some("Meku")),
            KeyBinding::new("ctrl-k", DocLink, Some("Meku")),
            KeyBinding::new("ctrl-b", DocBold, Some("Meku")),
            KeyBinding::new("ctrl-i", DocItalic, Some("Meku")),
            KeyBinding::new("ctrl-shift-x", DocStrike, Some("Meku")),
            KeyBinding::new("ctrl-shift-h", DocHighlight, Some("Meku")),
            KeyBinding::new("ctrl-`", DocInlineCode, Some("Meku")),
            KeyBinding::new("ctrl-alt-q", DocQuote, Some("Meku")),
            KeyBinding::new("ctrl-d", DocExpand, Some("Meku")),
            KeyBinding::new("ctrl-0", DocH0, Some("Meku")),
            KeyBinding::new("ctrl-1", DocH1, Some("Meku")),
            KeyBinding::new("ctrl-2", DocH2, Some("Meku")),
            KeyBinding::new("ctrl-3", DocH3, Some("Meku")),
            KeyBinding::new("ctrl-4", DocH4, Some("Meku")),
            KeyBinding::new("ctrl-5", DocH5, Some("Meku")),
            KeyBinding::new("ctrl-6", DocH6, Some("Meku")),
            KeyBinding::new("tab", DocIndent, Some("Meku")),
            KeyBinding::new("shift-tab", DocOutdent, Some("Meku")),
            KeyBinding::new("ctrl-shift-8", DocListBullet, Some("Meku")),
            KeyBinding::new("ctrl-shift-7", DocListOrdered, Some("Meku")),
            KeyBinding::new("ctrl-shift-9", DocListTask, Some("Meku")),
            KeyBinding::new("ctrl-l", DocToggleTask, Some("Meku")),
            KeyBinding::new("enter", DocEnter, Some("Meku")),
            KeyBinding::new("ctrl-a", DocSelectAll, Some("Meku")),
            KeyBinding::new("ctrl-z", DocUndo, Some("Meku")),
            KeyBinding::new("ctrl-y", DocRedo, Some("Meku")),
            KeyBinding::new("ctrl-shift-z", DocRedo, Some("Meku")),
            KeyBinding::new("ctrl-c", DocCopy, Some("Meku")),
            KeyBinding::new("ctrl-x", DocCut, Some("Meku")),
            KeyBinding::new("ctrl-v", DocPaste, Some("Meku")),
        ]);

        let bounds = Bounds::centered(None, size(px(1100.0), px(760.0)), cx);
        cx.spawn(async move |cx| {
            let mut options = TitleBar::window_options();
            options.window_bounds = Some(WindowBounds::Windowed(bounds));
            options.window_min_size = Some(size(px(640.0), px(420.0)));
            options.app_id = Some("meku".to_string());
            cx.open_window(options, |window, cx| {
                let view = cx.new(MekuShell::new);
                window
                    .observe_window_appearance(|_, cx| theme::on_system_appearance_changed(cx))
                    .detach();
                if let Some(dir) = initial_dir {
                    view.update(cx, |shell, cx| {
                        shell.open_mekuto(Path::new(&dir), cx);
                        // Optional second argument: a note (relative path) to
                        // open at startup; `MEKU_MODE=edit|live|read` and
                        // `MEKU_CARET=<byte>` set up reproducible states for
                        // screenshots.
                        for file in std::env::args().skip(2) {
                            shell.open_file_inner(Path::new(&file), cx);
                        }
                        shell.apply_startup_state(cx);
                    });
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
        CancelOverlay, CloseTab, CycleMode, DocBackspace, DocBold, DocCopy, DocCut, DocDelete,
        DocDeleteLine, DocDeleteToLineStart, DocDeleteWordBack, DocDeleteWordFwd, DocDocEnd,
        DocDocStart, DocDown, DocDuplicateLine, DocEnd, DocEnter, DocExpand, DocH0, DocH1, DocH2,
        DocH3, DocH4, DocH5, DocH6, DocHighlight, DocHome, DocIndent, DocInlineCode,
        DocInsertLineAbove, DocInsertLineBelow, DocItalic, DocJoinLines, DocLeft, DocLink,
        DocListBullet, DocListOrdered, DocListTask, DocMoveLineDown, DocMoveLineUp, DocOutdent,
        DocPageDown, DocPageUp, DocPaste, DocQuote, DocRedo, DocRight, DocSelectAll,
        DocSelectDocEnd, DocSelectDocStart, DocSelectDown, DocSelectEnd, DocSelectHome,
        DocSelectLeft, DocSelectPageDown, DocSelectPageUp, DocSelectRight, DocSelectUp,
        DocSelectWordLeft, DocSelectWordRight, DocSoftBreak, DocStrike, DocToggleComment,
        DocToggleTask, DocUndo, DocUp, DocWordRight, MekuShell, Mode, NamingMode, NewTab, NextTab,
        OpenFolder, Save, ToggleSidebar,
    };
    use gpui_kit::Focusable as _;
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
                KeyBinding::new("ctrl-\\", ToggleSidebar, Some("Meku")),
                KeyBinding::new("alt-left", crate::NavBack, Some("Meku")),
                KeyBinding::new("alt-right", crate::NavForward, Some("Meku")),
                KeyBinding::new("ctrl-p", crate::palette::OpenFileFinder, Some("Meku")),
                KeyBinding::new(
                    "ctrl-shift-p",
                    crate::palette::OpenCommandPalette,
                    Some("Meku"),
                ),
                KeyBinding::new(
                    "ctrl-shift-f",
                    crate::palette::OpenVaultSearch,
                    Some("Meku"),
                ),
                KeyBinding::new("ctrl-g", crate::palette::OpenGoToLine, Some("Meku")),
                KeyBinding::new("ctrl-f", crate::find::OpenFind, Some("Meku")),
                KeyBinding::new("ctrl-h", crate::find::OpenReplace, Some("Meku")),
                KeyBinding::new("f3", crate::find::FindNext, Some("Meku")),
                KeyBinding::new("shift-f3", crate::find::FindPrev, Some("Meku")),
                KeyBinding::new("up", crate::palette::PaletteUp, Some("Palette")),
                KeyBinding::new("down", crate::palette::PaletteDown, Some("Palette")),
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
                KeyBinding::new("ctrl-right", DocWordRight, Some("Meku")),
                KeyBinding::new("ctrl-home", DocDocStart, Some("Meku")),
                KeyBinding::new("ctrl-end", DocDocEnd, Some("Meku")),
                KeyBinding::new("pageup", DocPageUp, Some("Meku")),
                KeyBinding::new("pagedown", DocPageDown, Some("Meku")),
                KeyBinding::new("shift-left", DocSelectLeft, Some("Meku")),
                KeyBinding::new("shift-right", DocSelectRight, Some("Meku")),
                KeyBinding::new("ctrl-shift-left", DocSelectWordLeft, Some("Meku")),
                KeyBinding::new("ctrl-shift-right", DocSelectWordRight, Some("Meku")),
                KeyBinding::new("shift-up", DocSelectUp, Some("Meku")),
                KeyBinding::new("shift-down", DocSelectDown, Some("Meku")),
                KeyBinding::new("shift-home", DocSelectHome, Some("Meku")),
                KeyBinding::new("shift-end", DocSelectEnd, Some("Meku")),
                KeyBinding::new("ctrl-shift-home", DocSelectDocStart, Some("Meku")),
                KeyBinding::new("ctrl-shift-end", DocSelectDocEnd, Some("Meku")),
                KeyBinding::new("shift-pageup", DocSelectPageUp, Some("Meku")),
                KeyBinding::new("shift-pagedown", DocSelectPageDown, Some("Meku")),
                KeyBinding::new("backspace", DocBackspace, Some("Meku")),
                KeyBinding::new("delete", DocDelete, Some("Meku")),
                KeyBinding::new("ctrl-backspace", DocDeleteWordBack, Some("Meku")),
                KeyBinding::new("ctrl-delete", DocDeleteWordFwd, Some("Meku")),
                KeyBinding::new("ctrl-shift-backspace", DocDeleteToLineStart, Some("Meku")),
                KeyBinding::new("alt-up", DocMoveLineUp, Some("Meku")),
                KeyBinding::new("alt-down", DocMoveLineDown, Some("Meku")),
                KeyBinding::new("ctrl-shift-d", DocDuplicateLine, Some("Meku")),
                KeyBinding::new("ctrl-shift-k", DocDeleteLine, Some("Meku")),
                KeyBinding::new("ctrl-enter", DocInsertLineBelow, Some("Meku")),
                KeyBinding::new("ctrl-shift-enter", DocInsertLineAbove, Some("Meku")),
                KeyBinding::new("ctrl-j", DocJoinLines, Some("Meku")),
                KeyBinding::new("ctrl-/", DocToggleComment, Some("Meku")),
                KeyBinding::new("ctrl-k", DocLink, Some("Meku")),
                KeyBinding::new("ctrl-b", DocBold, Some("Meku")),
                KeyBinding::new("ctrl-i", DocItalic, Some("Meku")),
                KeyBinding::new("ctrl-shift-x", DocStrike, Some("Meku")),
                KeyBinding::new("ctrl-shift-h", DocHighlight, Some("Meku")),
                KeyBinding::new("ctrl-`", DocInlineCode, Some("Meku")),
                KeyBinding::new("ctrl-alt-q", DocQuote, Some("Meku")),
                KeyBinding::new("ctrl-d", DocExpand, Some("Meku")),
                KeyBinding::new("ctrl-0", DocH0, Some("Meku")),
                KeyBinding::new("ctrl-1", DocH1, Some("Meku")),
                KeyBinding::new("ctrl-2", DocH2, Some("Meku")),
                KeyBinding::new("ctrl-3", DocH3, Some("Meku")),
                KeyBinding::new("ctrl-4", DocH4, Some("Meku")),
                KeyBinding::new("ctrl-5", DocH5, Some("Meku")),
                KeyBinding::new("ctrl-6", DocH6, Some("Meku")),
                KeyBinding::new("tab", DocIndent, Some("Meku")),
                KeyBinding::new("shift-tab", DocOutdent, Some("Meku")),
                KeyBinding::new("ctrl-shift-8", DocListBullet, Some("Meku")),
                KeyBinding::new("ctrl-shift-7", DocListOrdered, Some("Meku")),
                KeyBinding::new("ctrl-shift-9", DocListTask, Some("Meku")),
                KeyBinding::new("ctrl-l", DocToggleTask, Some("Meku")),
                KeyBinding::new("shift-enter", DocSoftBreak, Some("Meku")),
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

    fn title_value(cx: &mut TestAppContext, view: &Entity<MekuShell>, rel: &str) -> String {
        cx.update(|cx| {
            view.read_with(cx, |shell, cx| {
                shell.title_inputs[&PathBuf::from(rel)]
                    .read(cx)
                    .value()
                    .to_string()
            })
        })
    }

    #[test]
    fn new_note_is_untitled_and_its_inline_title_renames_the_file() {
        run_ui_test("new_untitled", new_untitled_body);
    }

    fn new_untitled_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let dir = open_fixture(cx, &view);

        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                shell.begin_naming(NamingMode::NewNote, PathBuf::new(), None, cx);
            });
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
        })
        .unwrap();

        // No dialog and no markdown heading: the file is empty, the title is
        // UI above the body showing the file name.
        let path = dir.path().join("Untitled.md");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        let (naming_open, tabs) = cx
            .update(|cx| view.read_with(cx, |shell, _| (shell.naming.is_some(), shell.tabs.len())));
        assert!(!naming_open, "creating a note shows no window");
        assert_eq!(tabs, 1); // Untitled.md replaced the placeholder
        assert_eq!(title_value(cx, &view, "Untitled.md"), "Untitled");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find("doc-title"); // the title element is on screen
        })
        .unwrap();

        // The title is focused with its text selected: typing replaces it,
        // Enter renames the file and hands focus to the body.
        cx.update_window(window, |_, window, cx| {
            window.input("Plan", cx);
            window.press("enter", cx);
        })
        .unwrap();
        assert!(!path.exists(), "old name is gone");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Plan.md")).unwrap(),
            ""
        );
        let active = cx.update(|cx| view.read_with(cx, |shell, _| shell.doc_rel()));
        assert_eq!(
            active,
            Some(PathBuf::from("Plan.md")),
            "tab follows the rename"
        );
        assert_eq!(title_value(cx, &view, "Plan.md"), "Plan");

        // The title cannot be emptied: it snaps back to the file name.
        cx.update_window(window, |_, window, cx| {
            let rel = PathBuf::from("Plan.md");
            let input = view.read_with(cx, |shell, _| shell.title_inputs[&rel].clone());
            input.update(cx, |state, cx| state.set_value("", window, cx));
            view.update(cx, |shell, cx| shell.commit_title(&rel, true, cx));
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(title_value(cx, &view, "Plan.md"), "Plan");
        assert!(dir.path().join("Plan.md").exists());
    }

    #[test]
    fn title_edit_cannot_clobber_another_note() {
        run_ui_test("title_clash", title_clash_body);
    }

    fn title_clash_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let dir = open_fixture(cx, &view);
        std::fs::write(dir.path().join("Taken.md"), "keep").unwrap();
        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                shell.begin_naming(NamingMode::NewNote, PathBuf::new(), None, cx);
            });
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.input("Taken", cx);
            window.press("enter", cx);
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Taken.md")).unwrap(),
            "keep"
        );
        assert!(dir.path().join("Untitled.md").exists(), "rename refused");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(
            title_value(cx, &view, "Untitled.md"),
            "Untitled",
            "snaps back"
        );
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
    fn own_save_event_after_more_typing_is_not_a_conflict() {
        run_ui_test("own_save_event", own_save_event_body);
    }

    fn own_save_event_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let _dir = open_fixture(cx, &view);
        let rel = PathBuf::from("note.md");
        cx.update(|cx| {
            view.update(cx, |shell, cx| shell.open_file_inner(&rel, cx));
        });
        focus_doc_for(cx, &view, window);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("first ", cx);
        })
        .unwrap();
        // Save, then keep typing before the watcher reports our own write.
        cx.update(|cx| view.update(cx, |shell, cx| shell.save_buffer(&rel, cx)));
        cx.update_window(window, |_, window, cx| window.input("more", cx))
            .unwrap();
        assert!(buffer_text(cx, &view, &rel).1, "buffer is dirty again");
        cx.update(|cx| view.update(cx, |shell, cx| shell.sync_buffer_with_disk(&rel, cx)));
        let (conflict, notice) = cx.update(|cx| {
            view.read_with(cx, |shell, _| {
                (shell.conflict.clone(), shell.notice.clone())
            })
        });
        assert_eq!(conflict, None, "our own save is not an outside edit");
        assert_eq!(notice, None);
        assert!(
            buffer_text(cx, &view, &rel).0.contains("more"),
            "typing kept"
        );

        // A genuinely different disk version still raises the banner.
        std::fs::write(_dir.path().join("note.md"), "# Theirs\n").unwrap();
        cx.update(|cx| view.update(cx, |shell, cx| shell.sync_buffer_with_disk(&rel, cx)));
        let conflict = cx.update(|cx| view.read_with(cx, |shell, _| shell.conflict.clone()));
        assert_eq!(conflict, Some(rel.clone()));

        // Keeping mine silences repeat events for that same disk content.
        cx.update(|cx| view.update(cx, |shell, cx| shell.resolve_keep_mine(cx)));
        cx.update(|cx| view.update(cx, |shell, cx| shell.sync_buffer_with_disk(&rel, cx)));
        let conflict = cx.update(|cx| view.read_with(cx, |shell, _| shell.conflict.clone()));
        assert_eq!(conflict, None);
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

    /// Extend the selection from `start` to `end` through the real buffer.
    fn select_doc_range(
        cx: &mut TestAppContext,
        view: &Entity<MekuShell>,
        start: usize,
        end: usize,
    ) {
        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                if let Some(rel) = shell.doc_rel()
                    && let Some(buffer) = shell.buffers.get(&rel).cloned()
                {
                    buffer.update(cx, |b, _| {
                        b.move_caret(start, false);
                        b.move_caret(end, true);
                    });
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
        focus_doc_for(cx, view, window);
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
        let (_view, window, _dir) = open_note_in(
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

    // -- word and line navigation ---------------------------------------------

    fn doc_caret(cx: &mut TestAppContext, view: &Entity<MekuShell>, rel: &Path) -> usize {
        cx.update(|cx| {
            view.read_with(cx, |shell, cx| {
                shell
                    .buffers
                    .get(rel)
                    .unwrap()
                    .read_with(cx, |b, _| b.caret())
            })
        })
    }

    fn doc_selection(
        cx: &mut TestAppContext,
        view: &Entity<MekuShell>,
        rel: &Path,
    ) -> (usize, usize) {
        cx.update(|cx| {
            view.read_with(cx, |shell, cx| {
                shell.buffers.get(rel).unwrap().read_with(cx, |b, _| {
                    (b.anchor().min(b.caret()), b.anchor().max(b.caret()))
                })
            })
        })
    }

    #[test]
    fn ctrl_backspace_deletes_the_previous_word() {
        run_ui_test("delete_word_back", delete_word_back_body);
    }

    fn delete_word_back_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one two\n\nx\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 7); // after "two", before the newline
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-backspace", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "one \n\nx\n");
        assert_eq!(doc_caret(cx, &view, &rel), 4, "caret follows the deletion");
        assert!(buffer_text(cx, &view, &rel).1, "editor must be dirty");
        // Undo returns the word whole.
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-z", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "one two\n\nx\n");
    }

    #[test]
    fn ctrl_right_jumps_by_word_and_shift_selects() {
        run_ui_test("word_jump", word_jump_body);
    }

    fn word_jump_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "# Title\n\ncamelCase end\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        // Caret on the plain row, before "camelCase" (byte offset 8).
        move_doc_caret(cx, &view, "# Title\n\n".len());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-right", cx);
        })
        .unwrap();
        assert_eq!(
            doc_caret(cx, &view, &rel),
            14,
            "stops at the camel hump before Case"
        );
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-shift-right", cx);
        })
        .unwrap();
        assert_eq!(
            doc_selection(cx, &view, &rel),
            (14, 18),
            "Extend selects through the rest of the word"
        );
    }

    #[test]
    fn home_dances_from_indent_to_column_zero() {
        run_ui_test("smart_home", smart_home_body);
    }

    fn smart_home_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "    indented body\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 14); // after "body"
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("home", cx);
        })
        .unwrap();
        assert_eq!(doc_caret(cx, &view, &rel), 4, "first stop: first non-blank");
        cx.update_window(window, |_, window, cx| {
            window.press("home", cx);
        })
        .unwrap();
        assert_eq!(doc_caret(cx, &view, &rel), 0, "second stop: column zero");
    }

    #[test]
    fn ctrl_end_jumps_to_the_document_tail() {
        run_ui_test("doc_end", doc_end_body);
    }

    fn doc_end_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "a\n\nb\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-end", cx);
        })
        .unwrap();
        let (text, _) = buffer_text(cx, &view, &rel);
        assert_eq!(doc_caret(cx, &view, &rel), text.len());

        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-home", cx);
        })
        .unwrap();
        assert_eq!(doc_caret(cx, &view, &rel), 0);
    }

    #[test]
    fn pagedown_pages_a_viewport_of_rows() {
        run_ui_test("page_down", page_down_body);
    }

    fn page_down_body(cx: &mut TestAppContext) {
        let body: String = (0..120).fold(String::new(), |mut s, i| {
            use std::fmt::Write;
            let _ = writeln!(s, "row {i}");
            s
        });
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, &body);
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("pagedown", cx);
        })
        .unwrap();
        let caret = doc_caret(cx, &view, &rel);
        let row = body[..caret].matches('\n').count();
        assert!(
            (20..70).contains(&row),
            "one pagedown jumps ~a viewport of rows, got row {row}"
        );
        cx.update_window(window, |_, window, cx| {
            window.press("pagedown", cx);
        })
        .unwrap();
        let row2 = body[..doc_caret(cx, &view, &rel)].matches('\n').count();
        assert!(
            row2 > 70 && row2 < 120,
            "second page advanced further: {row2}"
        );
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

    // -- line editing shortcuts (F3) -----------------------------------

    #[test]
    fn alt_up_down_swaps_lines_with_the_caret() {
        run_ui_test("line_move", line_move_body);
    }

    fn line_move_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one\ntwo\nthree\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 5); // inside "two"
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("alt-up", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "two\none\nthree\n");
        assert_eq!(doc_caret(cx, &view, &rel), 1, "caret rides the moved line");
        cx.update_window(window, |_, window, cx| {
            window.press("alt-down", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "one\ntwo\nthree\n");
        assert_eq!(doc_caret(cx, &view, &rel), 5, "and back again");
    }

    #[test]
    fn ctrl_shift_k_deletes_the_caret_line() {
        run_ui_test("line_delete", line_delete_body);
    }

    fn line_delete_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one\ntwo\nthree\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 6); // inside "two"
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-shift-k", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "one\nthree\n");
        assert_eq!(
            doc_caret(cx, &view, &rel),
            4,
            "caret lands where the line was"
        );
        assert!(buffer_text(cx, &view, &rel).1, "editor must be dirty");
    }

    #[test]
    fn ctrl_enter_puts_a_blank_line_below_and_above() {
        run_ui_test("line_insert", line_insert_body);
    }

    fn line_insert_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one\ntwo\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 1); // inside "one"
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-enter", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "one\n\ntwo\n");
        assert_eq!(doc_caret(cx, &view, &rel), 4, "caret lands on the new line");
        // Now from the "two" line: an empty blank appears above it.
        move_doc_caret(cx, &view, 5); // start of "two"
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-shift-enter", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "one\n\n\ntwo\n");
        assert_eq!(
            doc_caret(cx, &view, &rel),
            5,
            "caret stays on the fresh line"
        );
    }

    #[test]
    fn ctrl_shift_d_duplicates_the_line() {
        run_ui_test("line_dup", line_dup_body);
    }

    fn line_dup_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one\ntwo\nthree\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 5); // inside "two"
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-shift-d", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "one\ntwo\ntwo\nthree\n");
        assert_eq!(doc_caret(cx, &view, &rel), 9, "caret rides into the copy");
    }

    #[test]
    fn ctrl_j_joins_with_the_next_line() {
        run_ui_test("line_join", line_join_body);
    }

    fn line_join_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one\n  two\nthree\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-j", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "one two\nthree\n");
        assert_eq!(doc_caret(cx, &view, &rel), 4, "caret sits at the seam");
    }

    #[test]
    fn ctrl_slash_comments_only_inside_code_fences() {
        run_ui_test("toggle_comment", toggle_comment_body);
    }

    fn toggle_comment_body(cx: &mut TestAppContext) {
        let (view, window, _dir) =
            open_note_in(cx, 1200.0, 800.0, "# Prose\n\n```rust\nlet x = 1;\n```\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        // Prose line: no fence language, nothing happens.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-/", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "# Prose\n\n```rust\nlet x = 1;\n```\n",
            "markdown prose is not commented"
        );
        // Inside the rust fence: gets a `//` comment.
        move_doc_caret(cx, &view, 20); // inside "let x = 1;"
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-/", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "# Prose\n\n```rust\n// let x = 1;\n```\n"
        );
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-/", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "# Prose\n\n```rust\nlet x = 1;\n```\n",
            "second press uncomments"
        );
    }

    // -- F4: enter & smart continuation -------------------------------

    #[test]
    fn enter_continues_the_bullet_list() {
        run_ui_test("enter_bullet", enter_bullet_body);
    }

    fn enter_bullet_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "- wash dishes\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 6); // after "wash", before the space
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "- wash\n- dishes\n",
            "marker repeats and the space rides below"
        );
        assert_eq!(doc_caret(cx, &view, &rel), 9);
        assert!(buffer_text(cx, &view, &rel).1, "editor must be dirty");
    }

    #[test]
    fn enter_on_an_empty_bullet_exits_the_list() {
        run_ui_test("enter_exit", enter_exit_body);
    }

    fn enter_exit_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "above\n- \nbelow\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 7); // after "- "
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "above\n\nbelow\n",
            "the empty marker is swallowed, not repeated"
        );
        assert_eq!(doc_caret(cx, &view, &rel), 6);
    }

    #[test]
    fn enter_renumbers_the_ordered_list() {
        run_ui_test("enter_renumber", enter_renumber_body);
    }

    fn enter_renumber_body(cx: &mut TestAppContext) {
        let (view, window, _dir) =
            open_note_in(cx, 1200.0, 800.0, "1. one\n2. two\n3. three\n4. four\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 12); // splits "2. tw|o"
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "1. one\n2. tw\n3. o\n4. three\n5. four\n",
            "items behind the split renumber"
        );
        assert_eq!(doc_caret(cx, &view, &rel), 16);
    }

    #[test]
    fn shift_enter_soft_breaks_the_item() {
        run_ui_test("soft_break", soft_break_body);
    }

    fn soft_break_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "- item\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 4); // "it|em"
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("shift-enter", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "- it\n  em\n",
            "the wrap lines up under the item text"
        );
        assert_eq!(doc_caret(cx, &view, &rel), 7);
    }

    #[test]
    fn enter_replaces_the_selection() {
        run_ui_test("enter_selection", enter_selection_body);
    }

    fn enter_selection_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "- one two\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        select_doc_range(cx, &view, 2, 5); // "one"
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "- \n two\n",
            "selection swaps for a plain newline"
        );
    }

    #[test]
    fn enter_inside_a_fence_stays_plain() {
        run_ui_test("enter_fence", enter_fence_body);
    }

    fn enter_fence_body(cx: &mut TestAppContext) {
        let (view, window, _dir) =
            open_note_in(cx, 1200.0, 800.0, "1. numbered\n\n```text\nx\ny\n```\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        // Caret right before "y" (after "x\n": 12 + 1 + 8 + 2 bytes in).
        move_doc_caret(cx, &view, 23);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "1. numbered\n\n```text\nx\n\ny\n```\n",
            "fences get a bare newline, not marker magic"
        );
    }

    // -- F5: indentation & lists --------------------------------------

    #[test]
    fn tab_and_shift_tab_nest_and_unnest_the_item() {
        run_ui_test("tab_nest", tab_nest_body);
    }

    fn tab_nest_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "- a\n- b\n- c\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 6);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("tab", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "- a\n  - b\n- c\n");
        assert_eq!(doc_caret(cx, &view, &rel), 8);
        cx.update_window(window, |_, window, cx| {
            window.press("shift-tab", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "- a\n- b\n- c\n");
        assert!(buffer_text(cx, &view, &rel).1, "editor must be dirty");
    }

    #[test]
    fn tab_in_a_fence_inserts_four_spaces() {
        run_ui_test("tab_fence", tab_fence_body);
    }

    fn tab_fence_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "```rust\nfn\n```\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 8);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("tab", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "```rust\n    fn\n```\n");
    }

    #[test]
    fn backspace_in_leading_whitespace_drops_one_level() {
        run_ui_test("bs_indent", bs_indent_body);
    }

    fn bs_indent_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "    text\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 4);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("backspace", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "  text\n");
        assert_eq!(doc_caret(cx, &view, &rel), 2);
    }

    #[test]
    fn list_conversion_and_checkbox_keys() {
        run_ui_test("list_convert", list_convert_body);
    }

    fn list_convert_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one\ntwo\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-a", cx);
            window.press("ctrl-shift-7", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "1. one\n2. two\n");
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-shift-9", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "- [ ] one\n- [ ] two\n");
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-l", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "- [x] one\n- [x] two\n");
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-shift-8", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "- one\n- two\n",
            "tasks swap to plain bullets"
        );
    }

    // -- F6: auto-pairing ---------------------------------------------

    #[test]
    fn typing_pairs_skips_and_wraps() {
        run_ui_test("auto_pair", auto_pair_body);
    }

    fn auto_pair_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "word\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 4);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input(" (", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "word ()\n");
        assert_eq!(doc_caret(cx, &view, &rel), 6);
        cx.update_window(window, |_, window, cx| {
            window.input("[", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "word ([])\n");
        assert_eq!(doc_caret(cx, &view, &rel), 7);
        cx.update_window(window, |_, window, cx| {
            window.input("]", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "word ([])\n",
            "closer skipped"
        );
        assert_eq!(doc_caret(cx, &view, &rel), 8);
        select_doc_range(cx, &view, 0, 4);
        cx.update_window(window, |_, window, cx| {
            window.input("*", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "*word* ([])\n",
            "selection wrapped"
        );
        move_doc_caret(cx, &view, 9); // between "[" and "]"
        cx.update_window(window, |_, window, cx| {
            window.press("backspace", cx);
        })
        .unwrap();
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "*word* ()\n",
            "empty pair removed"
        );
    }

    // -- Build 6: outline -----------------------------------------------

    #[test]
    fn outline_lists_headings_and_jumps() {
        run_ui_test("outline", outline_body);
    }

    fn outline_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(
            cx,
            1200.0,
            800.0,
            "# One\ntext\n```\n# not a heading\n```\n## Two\n",
        );
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        let outline = cx.update(|cx| view.update(cx, |shell, cx| shell.doc_outline(cx)));
        assert_eq!(
            outline,
            vec![(1, "One".to_string(), 0), (2, "Two".to_string(), 35)],
            "fenced hashes are code, not headings"
        );
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("outline-1", cx);
        })
        .unwrap();
        assert_eq!(doc_caret(cx, &view, &rel), 35, "click jumps to the heading");
    }

    // -- F9: links ----------------------------------------------------

    #[test]
    fn following_links_opens_notes_and_headings() {
        run_ui_test("follow_links", follow_links_body);
    }

    fn follow_links_body(cx: &mut TestAppContext) {
        use meku_markdown::live::{LinkTarget, classify_link};
        let (view, _window, dir) =
            open_note_in(cx, 1200.0, 800.0, "# Top\n[go](other.md#second)\n");
        std::fs::write(dir.path().join("other.md"), "# First\n\n## Second\n").unwrap();
        let go = |cx: &mut TestAppContext, dest: &str| -> bool {
            let from = view.read_with(&*cx, |s, _| s.doc_rel()).unwrap();
            let target: Option<LinkTarget> = classify_link(dest, &from);
            cx.update(|cx| view.update(cx, |s, cx| target.is_some_and(|t| s.follow_link(t, cx))))
        };
        assert!(go(cx, "other.md#second"));
        let rel = view.read_with(&*cx, |s, _| s.doc_rel()).unwrap();
        assert_eq!(rel, PathBuf::from("other.md"), "link opened the note");
        assert_eq!(doc_caret(cx, &view, &rel), 9, "and jumped to the heading");
        assert!(go(cx, "#first"), "in-note anchor by slug");
        assert_eq!(doc_caret(cx, &view, &rel), 0);
        assert!(!go(cx, "missing.md"), "dangling links do nothing");
        assert!(!go(cx, "#nope"));
        assert!(!go(cx, "../../etc/passwd"), "never leaves the mekuto");
        assert_eq!(view.read_with(&*cx, |s, _| s.doc_rel()).unwrap(), rel);
    }

    #[test]
    fn ctrl_k_links_the_selection_and_edits_existing() {
        run_ui_test("ctrl_k", ctrl_k_body);
    }

    fn ctrl_k_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "see docs\n[a](b.md)\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        select_doc_range(cx, &view, 4, 8);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-k", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "see [docs]()\n[a](b.md)\n");
        assert_eq!(doc_caret(cx, &view, &rel), 11);
        // Caret inside the second link: Ctrl+K selects its destination.
        move_doc_caret(cx, &view, 16);
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-k", cx);
            window.input("c.md", cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "see [docs]()\n[a](c.md)\n");
    }

    // -- design guidelines: column, offsets, caret --------------------

    #[test]
    fn document_sits_in_a_centered_reading_column() {
        run_ui_test("doc_column", doc_column_body);
    }

    fn doc_column_body(cx: &mut TestAppContext) {
        let (_view, window, _dir) = open_note_in(cx, 1400.0, 800.0, "# T\n\nbody\n");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let scroll = window.find("md-scroll").bounds();
            let row = window.find(("doc-row", 2usize)).bounds();
            let width = row.size.width.as_f32();
            assert!(
                (500.0..=640.0).contains(&width),
                "column should hold about 68ch, got {width}px"
            );
            let left = row.origin.x.as_f32() - scroll.origin.x.as_f32();
            let right = scroll.right().as_f32() - row.right().as_f32();
            assert!(
                (left - right).abs() <= 1.5,
                "column must be centered: left {left}, right {right}"
            );
            let first = window.find(("doc-row", 0usize)).bounds();
            let top = first.origin.y.as_f32() - scroll.origin.y.as_f32();
            assert!(top >= 56.0, "first line sits 56px down, got {top}");
        })
        .unwrap();
    }

    #[test]
    fn narrow_window_keeps_forty_pixels_either_side() {
        run_ui_test("doc_narrow", doc_narrow_body);
    }

    fn doc_narrow_body(cx: &mut TestAppContext) {
        let (_view, window, _dir) = open_note_in(cx, 760.0, 600.0, "body\n");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let scroll = window.find("md-scroll").bounds();
            let row = window.find(("doc-row", 0usize)).bounds();
            let left = row.origin.x.as_f32() - scroll.origin.x.as_f32();
            assert!(left >= 40.0, "side padding at least 40px, got {left}");
        })
        .unwrap();
    }

    #[test]
    fn caret_width_follows_font_size() {
        run_ui_test("caret_width", caret_width_body);
    }

    fn caret_width_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "# Big\n\nsmall\n");
        let (body, _) = {
            move_doc_caret(cx, &view, 8);
            caret_bounds(cx, window)
        };
        move_doc_caret(cx, &view, 2);
        let (heading, _) = caret_bounds(cx, window);
        assert!(body.size.width.as_f32() >= 1.5, "body caret stays visible");
        assert!(
            heading.size.width.as_f32() >= body.size.width.as_f32(),
            "heading caret is at least as wide as the body caret"
        );
    }
    // -- inline title on existing notes ----------------------------------

    fn dir_files(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.'))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn existing_note_can_be_renamed_repeatedly_through_its_title() {
        run_ui_test("title_existing", title_existing_body);
    }

    fn title_existing_body(cx: &mut TestAppContext) {
        let (view, window, dir) = open_note_in(cx, 1200.0, 800.0, "hello body\n");
        let rel_str = |cx: &mut TestAppContext| -> String {
            view.read_with(&*cx, |s, _| s.doc_rel())
                .unwrap()
                .to_string_lossy()
                .to_string()
        };
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.click("doc-title", cx);
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| window.press("backspace", cx))
            .unwrap();
        cx.update_window(window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        assert_eq!(dir_files(dir.path()), ["do.md"]);
        assert_eq!(rel_str(cx), "do.md", "no trailing separator after a rename");
        // Enter belongs to the title: the body is untouched.
        assert_eq!(
            buffer_text(cx, &view, &PathBuf::from("do.md")).0,
            "hello body\n"
        );

        // Second rename on the same note (this used to fail).
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("doc-title", cx);
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| window.press("backspace", cx))
            .unwrap();
        cx.update_window(window, |_, window, cx| window.input("ne", cx))
            .unwrap();
        cx.update_window(window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        assert_eq!(dir_files(dir.path()), ["dne.md"]);
        assert_eq!(rel_str(cx), "dne.md");

        // The renamed note still saves to its new path.
        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                let rel = shell.doc_rel().unwrap();
                shell.buffers[&rel].update(cx, |b, _| b.insert("more "));
                shell.save_buffer(&rel, cx);
            })
        });
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dne.md")).unwrap(),
            "more hello body\n"
        );
    }

    #[test]
    fn clicking_into_the_body_commits_a_pending_title_edit() {
        run_ui_test("title_flush", title_flush_body);
    }

    fn title_flush_body(cx: &mut TestAppContext) {
        let (view, window, dir) = open_note_in(cx, 1200.0, 800.0, "hello body\n");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.click("doc-title", cx);
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| window.press("backspace", cx))
            .unwrap();
        cx.update_window(window, |_, window, cx| {
            window.click(("doc-row", 0usize), cx)
        })
        .unwrap();
        assert_eq!(dir_files(dir.path()), ["do.md"]);
        assert!(view.read_with(&*cx, |s, _| s.title_pending.is_none()));
    }

    #[test]
    fn idle_title_commit_does_not_rewrite_the_text_being_typed() {
        run_ui_test("title_idle", title_idle_body);
    }

    fn title_idle_body(cx: &mut TestAppContext) {
        let (view, window, dir) = open_note_in(cx, 1200.0, 800.0, "x\n");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.click("doc-title", cx);
        })
        .unwrap();
        // "doc " — trailing space mid-edit
        cx.update_window(window, |_, window, cx| window.input(" ", cx))
            .unwrap();
        cx.update(|cx| {
            view.update(cx, |shell, cx| {
                if let Some((_, at)) = shell.title_pending.as_mut() {
                    *at -= std::time::Duration::from_secs(2);
                }
                shell.autosave(cx);
            })
        });
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert_eq!(
            title_value(cx, &view, "doc.md"),
            "doc ",
            "typing continues past a pause"
        );
        assert_eq!(dir_files(dir.path()), ["doc.md"], "same stem: no rename");
    }

    // -- subsections, checkboxes, easy links --------------------------

    #[test]
    fn tab_nests_to_three_levels_and_stops() {
        run_ui_test("tab_levels", tab_levels_body);
    }

    fn tab_levels_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "- a\n- b\n- c\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        let press = |cx: &mut TestAppContext, key: &'static str| {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.press(key, cx);
            })
            .unwrap();
        };
        // b becomes a child of a (level 2) ...
        move_doc_caret(cx, &view, 6);
        press(cx, "tab");
        assert_eq!(buffer_text(cx, &view, &rel).0, "- a\n  - b\n- c\n");
        // ... c follows it down to level 2, then level 3, and stops there.
        move_doc_caret(cx, &view, 12);
        press(cx, "tab");
        press(cx, "tab");
        assert_eq!(buffer_text(cx, &view, &rel).0, "- a\n  - b\n    - c\n");
        press(cx, "tab");
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "- a\n  - b\n    - c\n",
            "a fourth level is refused"
        );
        press(cx, "shift-tab");
        assert_eq!(buffer_text(cx, &view, &rel).0, "- a\n  - b\n  - c\n");
    }

    #[test]
    fn clicking_a_rendered_checkbox_toggles_the_task() {
        run_ui_test("checkbox_click", checkbox_click_body);
    }

    fn checkbox_click_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "intro\n- [ ] a\n- [x] b\n");
        let rel = PathBuf::from("doc.md");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(("task-box", 1usize), cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "intro\n- [x] a\n- [x] b\n");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(("task-box", 2usize), cx);
        })
        .unwrap();
        assert_eq!(buffer_text(cx, &view, &rel).0, "intro\n- [x] a\n- [ ] b\n");
        assert_eq!(
            doc_caret(cx, &view, &rel),
            0,
            "clicking a box leaves the caret"
        );
        assert!(
            buffer_text(cx, &view, &rel).1,
            "toggle marks the note dirty"
        );
    }

    #[test]
    fn plain_click_follows_rendered_links_but_edits_the_caret_row() {
        run_ui_test("plain_link_click", plain_link_click_body);
    }

    fn plain_link_click_body(cx: &mut TestAppContext) {
        let (view, window, dir) = open_note_in(cx, 1200.0, 800.0, "intro\n[go](other.md)\n");
        std::fs::write(dir.path().join("other.md"), "# other\n").unwrap();
        let rel = PathBuf::from("doc.md");
        let at = gpui_kit::point(px(10.), px(8.));
        // Caret on the link's own row: the source is shown, a click edits.
        move_doc_caret(cx, &view, 8);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click_at(("doc-row", 1usize), at, cx);
        })
        .unwrap();
        assert_eq!(
            view.read_with(&*cx, |s, _| s.doc_rel()),
            Some(rel.clone()),
            "raw row: no navigation"
        );
        // Caret elsewhere: the link is rendered text and one plain click opens it.
        move_doc_caret(cx, &view, 0);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click_at(("doc-row", 1usize), at, cx);
        })
        .unwrap();
        assert_eq!(
            view.read_with(&*cx, |s, _| s.doc_rel()),
            Some(PathBuf::from("other.md")),
            "plain click follows a rendered link"
        );
    }

    // -- palette ------------------------------------------------------

    fn palette_items(cx: &mut TestAppContext, view: &Entity<MekuShell>) -> Vec<String> {
        cx.update(|cx| {
            view.read_with(cx, |shell, _| {
                shell
                    .palette
                    .as_ref()
                    .map(|p| p.items.iter().map(|i| i.label.clone()).collect())
                    .unwrap_or_default()
            })
        })
    }

    fn step(
        cx: &mut TestAppContext,
        window: gpui_kit::AnyWindowHandle,
        f: fn(&mut gpui_kit::Window, &mut gpui_kit::App),
    ) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx);
        })
        .unwrap();
    }

    #[test]
    fn ctrl_p_finds_and_opens_notes() {
        run_ui_test("palette_files", palette_files_body);
    }

    fn palette_files_body(cx: &mut TestAppContext) {
        use gpui_kit::test::TestWindowExt;
        let (view, window) = boot(cx);
        let dir = open_fixture(cx, &view);
        focus_doc_for(cx, &view, window);
        std::fs::write(dir.path().join("Zebra plan.md"), "stripes\n").unwrap();
        cx.update(|cx| view.update(cx, |s, _| s.explorer.refresh()));
        step(cx, window, |w, cx| w.press("ctrl-p", cx));
        step(cx, window, |_, _| {});
        assert!(
            cx.update(|cx| view.read_with(cx, |s, _| s.palette.is_some())),
            "Ctrl+P opens the file finder"
        );
        assert!(palette_items(cx, &view).len() >= 2, "lists every note");
        step(cx, window, |w, cx| w.input("zeb", cx));
        step(cx, window, |_, _| {});
        assert_eq!(palette_items(cx, &view), vec!["Zebra plan".to_string()]);
        step(cx, window, |w, cx| w.press("enter", cx));
        step(cx, window, |_, _| {});
        assert_eq!(
            cx.update(|cx| view.read_with(cx, |s, _| s.doc_rel())),
            Some(PathBuf::from("Zebra plan.md"))
        );
        assert!(cx.update(|cx| view.read_with(cx, |s, _| s.palette.is_none())));
    }

    #[test]
    fn palette_arrows_move_and_escape_closes() {
        run_ui_test("palette_arrows", palette_arrows_body);
    }

    fn palette_arrows_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let _dir = open_fixture(cx, &view);
        focus_doc_for(cx, &view, window);
        step(cx, window, |w, cx| w.press("ctrl-shift-p", cx));
        step(cx, window, |_, _| {});
        let first = palette_items(cx, &view);
        assert!(first.iter().any(|l| l == "Toggle sidebar"), "{first:?}");
        let selected = |cx: &mut TestAppContext| {
            cx.update(|cx| view.read_with(cx, |s, _| s.palette.as_ref().map(|p| p.selected)))
        };
        assert_eq!(selected(cx), Some(0));
        step(cx, window, |w, cx| w.press("down", cx));
        assert_eq!(selected(cx), Some(1), "Down moves the highlight");
        step(cx, window, |w, cx| w.press("up", cx));
        step(cx, window, |w, cx| w.press("up", cx));
        assert_eq!(
            selected(cx),
            Some(first.len().min(10) - 1),
            "Up wraps around"
        );
        // Filtering narrows the commands.
        step(cx, window, |w, cx| w.input("sidebar", cx));
        step(cx, window, |_, _| {});
        assert_eq!(palette_items(cx, &view), vec!["Toggle sidebar".to_string()]);
        step(cx, window, |w, cx| w.press("escape", cx));
        step(cx, window, |_, _| {});
        assert!(
            cx.update(|cx| view.read_with(cx, |s, _| s.palette.is_none())),
            "Esc closes"
        );
    }

    #[test]
    fn command_runs_and_theme_preview_restores_on_cancel() {
        run_ui_test("palette_commands", palette_commands_body);
    }

    fn palette_commands_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let _dir = open_fixture(cx, &view);
        focus_doc_for(cx, &view, window);
        // Run "Toggle sidebar" from the palette.
        step(cx, window, |w, cx| w.press("ctrl-shift-p", cx));
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.input("toggle sid", cx));
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.press("enter", cx));
        step(cx, window, |_, _| {});
        step(cx, window, |_, _| {});
        assert!(
            !cx.update(|cx| view.read_with(cx, |s, _| s.show_sidebar)),
            "command ran"
        );

        // Theme picker previews on arrow and restores on Escape.
        cx.update(|cx| crate::theme::apply(crate::theme::MEKU_LIGHT, cx));
        cx.update(|cx| {
            view.update(cx, |s, cx| {
                s.open_palette(crate::palette::PaletteKind::Themes, cx)
            })
        });
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.press("down", cx)); // Meku Dark
        assert_eq!(
            cx.update(|cx| crate::theme::active(cx).name),
            "Meku Dark",
            "live preview"
        );
        step(cx, window, |w, cx| w.press("escape", cx));
        step(cx, window, |_, _| {});
        assert_eq!(
            cx.update(|cx| crate::theme::active(cx).name),
            "Meku Light",
            "cancel restores"
        );
    }

    #[test]
    fn go_to_line_and_vault_search_jump() {
        run_ui_test("palette_goto", palette_goto_body);
    }

    fn palette_goto_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one\ntwo\nthree\nfour\n");
        let rel = PathBuf::from("doc.md");
        focus_doc_for(cx, &view, window);
        step(cx, window, |w, cx| w.press("ctrl-g", cx));
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.input("3", cx));
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.press("enter", cx));
        step(cx, window, |_, _| {});
        assert_eq!(
            doc_caret(cx, &view, &rel),
            8,
            "caret on the start of line 3"
        );

        step(cx, window, |w, cx| w.press("ctrl-shift-f", cx));
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.input("four", cx));
        step(cx, window, |_, _| {});
        let hits = palette_items(cx, &view);
        assert_eq!(hits, vec!["four".to_string()]);
        step(cx, window, |w, cx| w.press("enter", cx));
        step(cx, window, |_, _| {});
        assert_eq!(
            doc_caret(cx, &view, &rel),
            14,
            "search hit jumps to its line"
        );
    }

    // -- find & replace -----------------------------------------------

    fn find_state(
        cx: &mut TestAppContext,
        view: &Entity<MekuShell>,
    ) -> Option<(usize, Option<usize>)> {
        cx.update(|cx| {
            view.read_with(cx, |s, _| {
                s.find.as_ref().map(|f| (f.matches.len(), f.current))
            })
        })
    }

    #[test]
    fn find_bar_counts_steps_and_closes() {
        run_ui_test("find_steps", find_steps_body);
    }

    fn find_steps_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one Two three\ntwo and TWO\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        step(cx, window, |w, cx| w.press("ctrl-f", cx));
        step(cx, window, |_, _| {});
        assert!(Self_find_open(cx, &view), "Ctrl+F opens the bar");
        step(cx, window, |w, cx| w.input("two", cx));
        step(cx, window, |_, _| {});
        assert_eq!(
            find_state(cx, &view),
            Some((3, Some(0))),
            "case-insensitive: three hits"
        );
        assert_eq!(
            cx.update(|cx| view.read_with(cx, |s, cx| s.buffers[&rel].read(cx).selected_text())),
            "Two",
            "the current match is selected in the note"
        );
        step(cx, window, |w, cx| w.press("enter", cx));
        assert_eq!(
            find_state(cx, &view),
            Some((3, Some(1))),
            "Enter goes to the next"
        );
        step(cx, window, |w, cx| w.press("shift-enter", cx));
        assert_eq!(
            find_state(cx, &view),
            Some((3, Some(0))),
            "Shift+Enter goes back"
        );
        step(cx, window, |w, cx| w.press("f3", cx));
        assert_eq!(find_state(cx, &view), Some((3, Some(1))), "F3 also steps");
        // Typing keeps going to the find field, never the note.
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "one Two three\ntwo and TWO\n"
        );
        step(cx, window, |w, cx| w.press("escape", cx));
        step(cx, window, |_, _| {});
        assert_eq!(find_state(cx, &view), None, "Esc closes the bar");
    }

    #[allow(non_snake_case)]
    fn Self_find_open(cx: &mut TestAppContext, view: &Entity<MekuShell>) -> bool {
        find_state(cx, view).is_some()
    }

    #[test]
    fn replace_one_and_all_with_undo() {
        run_ui_test("find_replace", find_replace_body);
    }

    fn find_replace_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "cat cat\ncat\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        step(cx, window, |w, cx| w.press("ctrl-h", cx));
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.input("cat", cx));
        step(cx, window, |_, _| {});
        // Move focus to the replace field and type the replacement.
        cx.update(|cx| {
            view.update(cx, |s, cx| {
                s.find
                    .as_ref()
                    .unwrap()
                    .replace
                    .update(cx, |_, cx| cx.notify());
            })
        });
        cx.update_window(window, |_, window, cx| {
            let input = view.read_with(cx, |s, _| s.find.as_ref().unwrap().replace.clone());
            let handle = input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        })
        .unwrap();
        step(cx, window, |w, cx| w.input("dog", cx));
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.click("replace-one", cx));
        step(cx, window, |_, _| {});
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "dog cat\ncat\n",
            "replaces the current hit"
        );
        step(cx, window, |w, cx| w.click("replace-all", cx));
        step(cx, window, |_, _| {});
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "dog dog\ndog\n",
            "replace all"
        );
        assert_eq!(find_state(cx, &view).map(|s| s.0), Some(0), "no cat left");
        step(cx, window, |w, cx| w.press("escape", cx));
        step(cx, window, |_, _| {});
        focus_doc_for(cx, &view, window);
        step(cx, window, |w, cx| w.press("ctrl-z", cx));
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "dog cat\ncat\n",
            "replace all is one undo step"
        );
    }

    // -- tables -------------------------------------------------------

    #[test]
    fn tab_and_enter_work_inside_tables() {
        run_ui_test("table_keys", table_keys_body);
    }

    fn table_keys_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "|a|b|\n|-|-|\n|1|2|\n\nend\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 1);
        step(cx, window, |w, cx| w.press("tab", cx));
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "| a   | b   |\n| --- | --- |\n| 1   | 2   |\n\nend\n",
            "Tab aligns the table"
        );
        let selected = |cx: &mut TestAppContext| {
            cx.update(|cx| view.read_with(cx, |s, cx| s.buffers[&rel].read(cx).selected_text()))
        };
        assert_eq!(selected(cx), "b", "and moves to the next cell");
        step(cx, window, |w, cx| w.press("shift-tab", cx));
        assert_eq!(selected(cx), "a");
        // Enter on a body row adds a row below.
        move_doc_caret(cx, &view, 36);
        step(cx, window, |w, cx| w.press("enter", cx));
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "| a   | b   |\n| --- | --- |\n| 1   | 2   |\n|     |     |\n\nend\n"
        );
        // The rendered grid exists for rows the caret is not on.
        step(cx, window, |_, _| {});
        let rows = cx.update(|cx| {
            view.update(cx, |s, cx| {
                s.doc_model(&rel, cx)
                    .map(|m| m.rows.iter().filter(|r| r.table.is_some()).count())
            })
        });
        assert_eq!(rows, Some(4));
    }

    // -- code blocks & callouts ----------------------------------------

    #[test]
    fn copy_button_puts_the_code_on_the_clipboard() {
        run_ui_test("code_copy", code_copy_body);
    }

    fn code_copy_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(
            cx,
            1200.0,
            800.0,
            "intro\n\n```rust\nfn a() {}\nlet b = 1;\n```\nafter\n",
        );
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(("copy-code", 2usize), cx);
        })
        .unwrap();
        let clip = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
        assert_eq!(
            clip.as_deref(),
            Some("fn a() {}\nlet b = 1;"),
            "fence lines are not copied"
        );
        assert!(
            view.read_with(&*cx, |s, _| s.copied.is_some()),
            "button reports Copied"
        );
        // The caret did not move onto the fence row (which would reveal it).
        assert_eq!(doc_caret(cx, &view, &PathBuf::from("doc.md")), 0);
    }

    #[test]
    fn callout_blocks_are_recognised_in_the_model() {
        run_ui_test("callout_model", callout_model_body);
    }

    fn callout_model_body(cx: &mut TestAppContext) {
        let (view, _window, _dir) =
            open_note_in(cx, 1200.0, 800.0, "> [!tip] Hint\n> use it\n\n> plain\n");
        let rel = PathBuf::from("doc.md");
        let kinds = cx.update(|cx| {
            view.update(cx, |s, cx| {
                s.doc_model(&rel, cx).map(|m| {
                    m.rows
                        .iter()
                        .map(|r| r.callout.is_some())
                        .collect::<Vec<_>>()
                })
            })
        });
        assert_eq!(kinds, Some(vec![true, true, false, false, false]));
    }

    // -- formatting shortcuts --------------------------------------------

    #[test]
    fn formatting_shortcuts_wrap_and_set_headings() {
        run_ui_test("format_keys", format_keys_body);
    }

    fn format_keys_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "make bold now\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        select_doc_range(cx, &view, 5, 9);
        step(cx, window, |w, cx| w.press("ctrl-b", cx));
        assert_eq!(buffer_text(cx, &view, &rel).0, "make **bold** now\n");
        step(cx, window, |w, cx| w.press("ctrl-i", cx));
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "make ***bold*** now\n",
            "italic stacks on bold"
        );
        step(cx, window, |w, cx| w.press("ctrl-i", cx));
        step(cx, window, |w, cx| w.press("ctrl-b", cx));
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "make bold now\n",
            "both toggle off again"
        );
        step(cx, window, |w, cx| w.press("ctrl-shift-h", cx));
        assert_eq!(buffer_text(cx, &view, &rel).0, "make ==bold== now\n");
        move_doc_caret(cx, &view, 2);
        step(cx, window, |w, cx| w.press("ctrl-2", cx));
        assert_eq!(buffer_text(cx, &view, &rel).0, "## make ==bold== now\n");
        step(cx, window, |w, cx| w.press("ctrl-0", cx));
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "make ==bold== now\n",
            "Ctrl+0 is a paragraph"
        );
        // The sidebar toggle moved to Ctrl+\ so Ctrl+B can be bold.
        assert!(cx.update(|cx| view.read_with(cx, |s, _| s.show_sidebar)));
        step(cx, window, |w, cx| w.press("ctrl-\\", cx));
        assert!(!cx.update(|cx| view.read_with(cx, |s, _| s.show_sidebar)));
    }

    #[test]
    fn ctrl_d_expands_and_double_click_selects_a_word() {
        run_ui_test("expand_click", expand_click_body);
    }

    fn expand_click_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "alpha beta\ngamma\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        let selected = |cx: &mut TestAppContext| {
            cx.update(|cx| view.read_with(cx, |s, cx| s.buffers[&rel].read(cx).selected_text()))
        };
        move_doc_caret(cx, &view, 2);
        step(cx, window, |w, cx| w.press("ctrl-d", cx));
        assert_eq!(selected(cx), "alpha");
        step(cx, window, |w, cx| w.press("ctrl-d", cx));
        assert_eq!(selected(cx), "alpha beta");
        move_doc_caret(cx, &view, 0);
        step(cx, window, |w, cx| {
            w.double_click(("doc-row", 0usize), cx);
        });
        assert!(!selected(cx).is_empty(), "double click selects something");
    }

    // -- slash menu ---------------------------------------------------

    #[test]
    fn slash_menu_filters_inserts_and_dismisses() {
        run_ui_test("slash_menu", slash_menu_body);
    }

    fn slash_menu_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "intro\n\n");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 7);
        let open =
            |cx: &mut TestAppContext| cx.update(|cx| view.read_with(cx, |s, _| s.slash.is_some()));
        step(cx, window, |w, cx| w.input("/", cx));
        assert!(open(cx), "typing / at the start of a line opens the menu");
        step(cx, window, |w, cx| w.input("tab", cx));
        step(cx, window, |w, cx| w.press("enter", cx));
        assert!(!open(cx), "choosing closes the menu");
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "intro\n\n| Column | Column |\n| ------ | ------ |\n|        |        |",
            "the table template replaces /tab"
        );
        assert_eq!(
            doc_caret(cx, &view, &rel),
            7 + 20 + 20 + 2,
            "caret in the first body cell"
        );

        // Esc dismisses; typing on keeps it closed; deleting the slash resets.
        let (view2, window2, _d2) = open_note_in(cx, 1200.0, 800.0, "x /");
        focus_doc_for(cx, &view2, window2);
        move_doc_caret(cx, &view2, 3);
        step(cx, window2, |w, cx| w.input("h", cx));
        assert!(cx.update(|cx| view2.read_with(cx, |s, _| s.slash.is_some())));
        step(cx, window2, |w, cx| w.press("escape", cx));
        assert!(
            !cx.update(|cx| view2.read_with(cx, |s, _| s.slash.is_some())),
            "Esc closes"
        );
        step(cx, window2, |w, cx| w.input("e", cx));
        assert!(
            !cx.update(|cx| view2.read_with(cx, |s, _| s.slash.is_some())),
            "stays closed"
        );
        // A path-like slash never opens it.
        let (view3, window3, _d3) = open_note_in(cx, 1200.0, 800.0, "a");
        focus_doc_for(cx, &view3, window3);
        move_doc_caret(cx, &view3, 1);
        step(cx, window3, |w, cx| w.input("/", cx));
        assert!(
            !cx.update(|cx| view3.read_with(cx, |s, _| s.slash.is_some())),
            "a/ is not a command"
        );
    }

    #[test]
    fn slash_menu_arrows_and_click() {
        run_ui_test("slash_arrows", slash_arrows_body);
    }

    fn slash_arrows_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        step(cx, window, |w, cx| w.input("/", cx));
        step(cx, window, |w, cx| w.press("down", cx));
        step(cx, window, |w, cx| w.press("down", cx));
        step(cx, window, |w, cx| w.press("enter", cx));
        assert_eq!(
            buffer_text(cx, &view, &rel).0,
            "### ",
            "third entry is Heading 3"
        );
        // Click an entry with the mouse.
        step(cx, window, |w, cx| w.press("enter", cx));
        step(cx, window, |w, cx| w.input("/", cx));
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.click(("slash-item", 6usize), cx));
        assert!(
            buffer_text(cx, &view, &rel).0.ends_with("> "),
            "clicked the Quote entry: {:?}",
            buffer_text(cx, &view, &rel).0
        );
    }

    #[test]
    fn slash_menu_does_not_depend_on_the_caret_blink() {
        run_ui_test("slash_blink", slash_blink_body);
    }

    fn slash_blink_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "");
        focus_doc_for(cx, &view, window);
        step(cx, window, |w, cx| w.input("/", cx));
        // Blink the caret off: the menu must stay on screen.
        cx.update(|cx| {
            view.update(cx, |s, cx| {
                s.caret_on = false;
                cx.notify();
            })
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("slash-menu").is_some(),
                "menu drawn with the caret hidden"
            );
            assert!(
                window.try_find("doc-caret").is_none(),
                "caret really is hidden"
            );
        })
        .unwrap();
    }

    // -- motion ---------------------------------------------------------

    #[test]
    fn motion_follows_the_reduce_motion_setting() {
        run_ui_test("motion_setting", motion_setting_body);
    }

    fn motion_setting_body(cx: &mut TestAppContext) {
        assert!(
            !cx.update(|cx| crate::theme::motion(cx)),
            "no settings loaded (tests): motion off"
        );
        cx.update(|cx| cx.set_global(crate::theme::Settings::default()));
        assert!(
            cx.update(|cx| crate::theme::motion(cx)),
            "default: motion on"
        );
        cx.update(|cx| {
            let mut s = cx.global::<crate::theme::Settings>().clone();
            s.reduce_motion = true;
            cx.set_global(s);
        });
        assert!(
            !cx.update(|cx| crate::theme::motion(cx)),
            "reduce motion turns it off"
        );
    }

    // -- chrome ---------------------------------------------------------

    #[test]
    fn tab_strip_selects_closes_and_adds() {
        run_ui_test("tab_strip", tab_strip_body);
    }

    fn tab_strip_body(cx: &mut TestAppContext) {
        let (view, window, dir) = open_note_in(cx, 1200.0, 800.0, "first\n");
        std::fs::write(dir.path().join("second.md"), "second\n").unwrap();
        cx.update(|cx| {
            view.update(cx, |s, cx| {
                s.explorer.refresh();
                s.open_file_inner(Path::new("second.md"), cx);
            })
        });
        let active = |cx: &mut TestAppContext| view.read_with(&*cx, |s, _| s.doc_rel());
        assert_eq!(active(cx), Some(PathBuf::from("second.md")));
        step(cx, window, |w, cx| w.click(("tab", 0usize), cx));
        assert_eq!(
            active(cx),
            Some(PathBuf::from("doc.md")),
            "clicking a tab selects it"
        );
        step(cx, window, |w, cx| w.click("close-tab-0", cx));
        assert_eq!(
            view.read_with(&*cx, |s, _| s.tabs.len()),
            1,
            "close icon closes the tab"
        );
        assert_eq!(active(cx), Some(PathBuf::from("second.md")));
        step(cx, window, |w, cx| w.click("new-tab", cx));
        assert_eq!(
            view.read_with(&*cx, |s, _| s.tabs.len()),
            2,
            "+ creates a note"
        );
    }

    #[test]
    fn title_bar_mode_switch_and_status_counts() {
        run_ui_test("title_modes", title_modes_body);
    }

    fn title_modes_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "one two three\nfour\n");
        step(cx, window, |w, cx| w.click("mode-read", cx));
        assert_eq!(view.read_with(&*cx, |s, _| s.tab_mode()), Mode::Read);
        step(cx, window, |w, cx| w.click("mode-edit", cx));
        assert_eq!(view.read_with(&*cx, |s, _| s.tab_mode()), Mode::Edit);
        // The status bar counts words and reports the caret position.
        let (words, lc) = cx.update(|cx| {
            view.update(cx, |s, cx| {
                let rel = s.doc_rel().unwrap();
                (
                    s.doc_model(&rel, cx).map(|m| m.words),
                    s.buffers[&rel].read(cx).line_col(15),
                )
            })
        });
        assert_eq!(words, Some(4));
        assert_eq!(lc, (2, 2));
        step(cx, window, |w, cx| w.click("title-search", cx));
        step(cx, window, |_, _| {});
        assert!(
            view.read_with(&*cx, |s, _| s.palette.is_some()),
            "title search opens the finder"
        );
    }

    #[test]
    fn sidebar_search_and_outline_toggle() {
        run_ui_test("sidebar_chrome", sidebar_chrome_body);
    }

    fn sidebar_chrome_body(cx: &mut TestAppContext) {
        let (view, window, _dir) = open_note_in(cx, 1200.0, 800.0, "# Head\nbody\n");
        step(cx, window, |w, cx| w.click("sidebar-search", cx));
        step(cx, window, |_, _| {});
        assert!(
            view.read_with(&*cx, |s, _| s.palette.is_some()),
            "search field opens the finder"
        );
        step(cx, window, |w, cx| w.press("escape", cx));
        assert!(view.read_with(&*cx, |s, _| s.outline_open));
        step(cx, window, |w, cx| w.click("section-Outline", cx));
        assert!(
            !view.read_with(&*cx, |s, _| s.outline_open),
            "outline header collapses it"
        );
        step(cx, window, |w, cx| w.click("sidebar-toggle", cx));
        assert!(
            !view.read_with(&*cx, |s, _| s.show_sidebar),
            "title bar toggles the sidebar"
        );
    }

    // -- images & lightbox -------------------------------------------------

    /// Smallest PNG header: signature + IHDR of a 1x1 image (enough for the
    /// size probe, which never decodes).
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89,
    ];

    #[test]
    fn image_frames_keep_proportions_and_caps() {
        use crate::document::{ImageInfo, ImageSrc, image_frame_size};
        let mk = |dims: Option<(u32, u32)>, width: Option<u32>| ImageInfo {
            src: ImageSrc::Url("x".into()),
            caption: String::new(),
            width,
            dims,
        };
        let f = |i: &ImageInfo, avail: f32| {
            let (w, h) = image_frame_size(i, px(avail));
            (f32::from(w), f32::from(h))
        };
        assert_eq!(
            f(&mk(Some((1200, 675)), None), 600.0),
            (600.0, 337.5),
            "wide: fills the column"
        );
        assert_eq!(
            f(&mk(Some((200, 100)), None), 600.0),
            (200.0, 100.0),
            "small: never upscaled"
        );
        let (w, h) = f(&mk(Some((600, 900)), None), 600.0);
        assert_eq!(h, 460.0, "tall images are capped");
        assert!(
            (w - 306.67).abs() < 0.1,
            "…with the width scaled to match: {w}"
        );
        assert_eq!(
            f(&mk(Some((1200, 675)), Some(240)), 600.0),
            (240.0, 135.0),
            "|240 width hint"
        );
        assert_eq!(
            f(&mk(None, None), 800.0),
            (480.0, 270.0),
            "unknown size: 16:9"
        );
    }

    #[test]
    fn clicking_an_image_opens_the_lightbox_and_zoom_steps_work() {
        run_ui_test("lightbox", lightbox_body);
    }

    fn lightbox_body(cx: &mut TestAppContext) {
        let (view, window, dir) = open_note_in(
            cx,
            1200.0,
            800.0,
            "intro\n\n![the cap|50](pic one.png)\n\nafter\n",
        );
        std::fs::write(dir.path().join("pic one.png"), PNG_1X1).unwrap();
        // Rebuild the model so the file's size is probed.
        cx.update(|cx| view.update(cx, |s, _| s.doc_models.clear()));
        let state = |cx: &mut TestAppContext| {
            view.read_with(&*cx, |s, _| (s.lightbox.is_some(), s.lightbox_zoom))
        };
        step(cx, window, |_, _| {});
        let info = cx.update(|cx| {
            view.update(cx, |s, cx| {
                let rel = s.doc_rel().unwrap();
                s.doc_model(&rel, cx).and_then(|m| m.images[2].clone())
            })
        });
        let info = info.expect("image row is recognised (spaces in the path, |width hint)");
        assert_eq!(
            (info.caption.as_str(), info.width, info.dims),
            ("the cap", Some(50), Some((1, 1)))
        );
        step(cx, window, |w, cx| w.click(("image-row", 2usize), cx));
        assert_eq!(state(cx), (true, None), "click opens the zoom view at fit");
        assert_eq!(
            doc_caret(cx, &view, &PathBuf::from("doc.md")),
            0,
            "the click did not move the caret onto the image row"
        );
        step(cx, window, |w, cx| w.click("lightbox-in", cx));
        assert_eq!(
            state(cx).1,
            Some(4.0),
            "zoom in from fit (3x for a 1px image)"
        );
        step(cx, window, |w, cx| w.click("lightbox-out", cx));
        assert_eq!(state(cx).1, Some(3.0));
        step(cx, window, |w, cx| w.click("lightbox-fit", cx));
        assert_eq!(state(cx).1, None, "fit button resets");
        step(cx, window, |w, cx| w.press("escape", cx));
        assert!(!state(cx).0, "Esc closes the view");
        // Closing also works from the close button and the backdrop.
        step(cx, window, |w, cx| w.click(("image-row", 2usize), cx));
        step(cx, window, |w, cx| w.click("lightbox-close", cx));
        assert!(!state(cx).0);
    }

    #[test]
    fn pasting_an_image_saves_it_and_links_it() {
        run_ui_test("paste_image", paste_image_body);
    }

    fn paste_image_body(cx: &mut TestAppContext) {
        let (view, window, dir) = open_note_in(cx, 1200.0, 800.0, "intro");
        focus_doc_for(cx, &view, window);
        let rel = PathBuf::from("doc.md");
        move_doc_caret(cx, &view, 5);
        cx.update(|cx| {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_image(&gpui_kit::Image {
                format: gpui_kit::ImageFormat::Png,
                bytes: PNG_1X1.to_vec(),
                id: 7,
            }))
        });
        step(cx, window, |w, cx| w.press("ctrl-v", cx));
        let text = buffer_text(cx, &view, &rel).0;
        assert!(
            text.starts_with("intro\n![](attachments/pasted-"),
            "{text:?}"
        );
        assert!(
            text.ends_with(".png)\n"),
            "own line, with a trailing newline: {text:?}"
        );
        let saved: Vec<_> = std::fs::read_dir(dir.path().join("attachments"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            std::fs::read(dir.path().join("attachments").join(&saved[0])).unwrap(),
            PNG_1X1,
            "the bytes are saved as pasted"
        );
    }

    #[test]
    fn back_and_forward_walk_visited_notes() {
        run_ui_test("nav_history", nav_history_body);
    }

    fn nav_history_body(cx: &mut TestAppContext) {
        let (view, window, dir) = open_note_in(cx, 1200.0, 800.0, "a\n");
        std::fs::write(dir.path().join("b.md"), "b\n").unwrap();
        std::fs::write(dir.path().join("c.md"), "c\n").unwrap();
        cx.update(|cx| {
            view.update(cx, |s, cx| {
                s.explorer.refresh();
                s.open_file_inner(Path::new("b.md"), cx);
                s.open_file_inner(Path::new("c.md"), cx);
            })
        });
        focus_doc_for(cx, &view, window);
        let at = |cx: &mut TestAppContext| view.read_with(&*cx, |s, _| s.doc_rel().unwrap());
        assert_eq!(at(cx), PathBuf::from("c.md"));
        step(cx, window, |w, cx| w.press("alt-left", cx));
        assert_eq!(at(cx), PathBuf::from("b.md"), "Alt+Left goes back");
        step(cx, window, |w, cx| w.click("nav-back", cx));
        assert_eq!(at(cx), PathBuf::from("doc.md"), "the back button too");
        assert!(
            !view.read_with(&*cx, |s, _| s.can_nav(-1)),
            "nothing before the first note"
        );
        step(cx, window, |w, cx| w.press("alt-right", cx));
        assert_eq!(at(cx), PathBuf::from("b.md"));
        // Visiting something new drops the forward history.
        cx.update(|cx| view.update(cx, |s, cx| s.open_file_inner(Path::new("doc.md"), cx)));
        assert!(!view.read_with(&*cx, |s, _| s.can_nav(1)));
        // Closing a visited tab does not break going back to it.
        cx.update(|cx| view.update(cx, |s, cx| s.close_all_tabs(cx)));
        step(cx, window, |w, cx| w.press("alt-left", cx));
        assert_eq!(at(cx), PathBuf::from("b.md"), "a closed note reopens");
    }

    #[test]
    fn welcome_actions_and_status_items_are_clickable() {
        run_ui_test("welcome_status", welcome_status_body);
    }

    fn welcome_status_body(cx: &mut TestAppContext) {
        let (view, window) = boot(cx);
        let _dir = open_fixture(cx, &view);
        cx.update(|cx| view.update(cx, |s, cx| s.close_all_tabs(cx)));
        step(cx, window, |w, cx| w.click(("welcome-action", 1usize), cx));
        step(cx, window, |_, _| {});
        assert!(
            view.read_with(&*cx, |s, _| s.palette.is_some()),
            "'Open a note' opens the finder"
        );
        cx.update(|cx| view.update(cx, |s, cx| s.close_palette(true, cx)));
        cx.update(|cx| view.update(cx, |s, cx| s.open_file_inner(Path::new("note.md"), cx)));
        step(cx, window, |_, _| {});
        step(cx, window, |w, cx| w.click("status-mode", cx));
        assert_eq!(
            view.read_with(&*cx, |s, _| s.tab_mode()),
            Mode::Read,
            "status mode cycles Live → Read"
        );
        step(cx, window, |w, cx| w.click("status-position", cx));
        step(cx, window, |_, _| {});
        assert!(
            view.read_with(&*cx, |s, _| s.palette.is_some()),
            "Ln/Col opens go-to-line"
        );
    }
}
