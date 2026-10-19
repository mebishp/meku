# Meku — Master Checklist

> Living checklist. Check off each box when done. **Commit to upstream after every subsection** (user runs `git add/commit/push`; agent never auto-commits).
> Decisions locked: live-preview editing · `gpui-component`/`gpui-kit` foundation · `mekuto` folder-vault · no wikilinks/backlinks/graph in v0.1 · GFM core first · Linux-first.
> Perf budgets: cold start <300ms · 10k-line open <16ms main-thread · typing <16ms p95 · full reparse <30ms bg · idle RSS <120MB.

---

## Section 1 — Feature Discovery + Performance Optimisation Planning (pre-execution)

_Plan efficiently before writing app code. Research done by 4 subagents (features / perf / stack / LLD); boxes below track turning that research into frozen decisions._

### 1A. Feature discovery (Obsidian × Zed → Meku MVP)

- [x] Inventory Obsidian core plugins (explorer, tabs, switcher, search, palette, outline, status, settings, recovery) — triaged Must/Should/Defer
- [x] Inventory Zed UX patterns (titlebar, tabs, palette+finder split, project panel, outline, theme, keymap, status, toasts)
- [x] Freeze `mekuto` v0.1 definition: plain folder + `.md` files + `.meku/session.json` (sidecar only, portable, git-clean)
- [x] Freeze GFM subset: headings, emphasis/strong/strike, lists, task lists, code fence/inline, tables, quote, hr, links/images, autolinks, raw-HTML passthrough (sanitized)
- [x] Freeze explicit deferrals: `[[wikilinks]]`, embeds, backlinks, graph/canvas, tags pane, frontmatter UI, math, mermaid, vim, plugins/sync/publish, terminal/AI
- [ ] Write MVP acceptance criteria into `docs/acceptance.md` (12 criteria: open mekuto <1s/100 files, file ops, live-preview round-trip, tabs+dirty, Ctrl-P <100ms/1k files, search, palette, outline ≤200ms, status bar, settings persist, crash recovery ≤2s loss, GFM fixture zero panics)
- [x] Freeze keymap defaults (Linux): `Ctrl-P` finder · `Ctrl-Shift-P` palette · `Ctrl-B` explorer · `Ctrl-,` settings · `Ctrl-N/W/T` note/close/tab · `Ctrl-S` save · `Ctrl-F` find · `Ctrl-E` edit/live/read cycle (Build 5) · `Ctrl+←/→` word jump · `Ctrl+Backspace/Delete` word delete · `Ctrl+Shift+Backspace` delete-line-start · `Ctrl+Home/End` doc ends · PageUp/Down · smart `Home` (Build 5.x) · `Alt+↑/↓` move line · `Ctrl+Shift+D/K` duplicate/delete line · `Ctrl+Enter/Shift+Enter` insert below/above · `Ctrl+J` join lines · `Ctrl+/` comment toggle in code (F3)
- [ ] Freeze `session.json` schema v1 (`version`, `active_tab`, `open_tabs[{path,cursor,scroll,pinned}]`, `explorer{expanded,auto_reveal}`) + corrupt-file rule (backup to `session.corrupt-<ts>.json`, start empty, never crash)

**Mekuto layout (frozen):**
```
my-mekuto/
├── notes/*.md (+ subfolders)
├── assets/* (images, greyed in tree)
└── .meku/
    ├── session.json
    └── settings.json (optional override; else global)
```
Rules: only `*.md/*.markdown` are notes · `.meku/` hidden from tree + search · rename rewrites nothing (no links yet) · delete confirms · external edit → Reload/Keep-mine prompt.

### 1B. Performance optimisation plan (budgets + techniques, before code)

- [x] Set measurable budgets (table at top of this file) + measurement method (`hyperfine`, criterion, `/usr/bin/time -v`, in-app first-paint log)
- [x] Choose buffer: `ropey` for v0.1 (B-tree rope, streaming load, line-offset cache, coalesced undo ≤100 steps) — defer Zed `Rope/sum_tree/text::Buffer` to v0.2+ (trigger: >100k-line stutter or summary need)
- [x] Choose parse pipeline: `pulldown-cmark` pull parser (`ENABLE_TABLES|STRIKETHROUGH|TASKLISTS`), `into_offset_iter()` → `BlockMeta+InlineSpan`, never on UI thread, 120ms debounce, stale-version drop, fence-aware dirty expansion (±50 lines), frontmatter strip fast-path
- [x] Choose render virtualization: block model (not line model) + `uniform_list`/`ListState::with_uniform_item_height` + overscan ±5–10 blocks + height cache keyed by width + stable shaped-line handles
- [x] Choose file index: `walkdir`+`ignore` lazy tree (no content read at startup), `notify`/inotify debounced 150–200ms single-file reindex, in-memory `HashMap<PathId,FileMeta>` + path interning, no SQLite in v0.1
- [x] Choose image/code-highlight laziness: placeholder boxes, 2–4-thread decode pool, LRU 64–128 entries/128MB cap, viewport-only; code fences plain-first then tree-sitter for 5 langs (`md/rust/python/js-ts/bash`) visible-only, `OnceLock` grammars
- [x] Choose startup order: parse args → `Application::new` → empty window+theme → first paint → idle: load last file → idle: walk/index → idle: grammars; release `thin-LTO + strip + codegen-units 16 + panic abort`; dep hygiene (`cargo-machete/udeps/deny`, no `tokio-full/reqwest/regex`)
- [ ] Create `benches/corpus/` generator (seeded `small 200` / `medium 2k` / `large 10k` / `edge` unclosed-fence/CRLF/CJK) + criterion groups (`open`, `edit`, `parse`, `layout`, `walk`) with gates (10% `open_10k`, 15% `parse_10k` regression fail)
- [ ] Add hidden `--quit-after-first-paint` flag design (for `hyperfine` cold-start automation) + `/proc/self/status` RSS sampler design

### 1C. Stack validation (pin before scaffold)

- [x] Reject `crates.io gpui 0.2.2` (stale ~11mo) — pin Longbridge snapshot: `gpui = { package="gpui-pre", version="=0.3.8" }` + `gpui-pre-platform = "=0.3.8"` with `features=["font-kit","x11","wayland","runtime_shaders"]`, or umbrella `gpui-kit = "0.7.1"` (`use gpui_kit::*`)
- [x] Map `gpui-component` modules: `dock::{DockArea,DockSkin,dock_area}` · `sidebar` (+ `cx.theme().sidebar*` tokens) · `tab::{Tab,TabBar}` · `h_resizable`+`resizable_panel` · `Theme/ActiveTheme` · verify against `cargo run --example dock/markdown`
- [x] Confirm markdown path: `pulldown-cmark` editor rendering (`TextMergeStream`, own `Event→element` map) + `html::push_html` only for export; keep `comrak` for later export fidelity only
- [x] Record Linux gotchas: need Wayland/X11 session + Vulkan driver (`vulkan-radeon/intel` or `nvidia-utils`, `vulkaninfo` must pass), `fontconfig/freetype2` dev headers, `mold` linker, dev-profile `opt-level=3` for `gpui-pre/taffy/ttf-parser/rustybuzz`
- [ ] Freeze pinning policy: exact `=` pins for all `gpui-pre-*` + committed `Cargo.lock` + pin-check CI script (bump all snapshot crates together) · Rust ≥1.92 (we have 1.98.1)

---

## Section 2 — Low-Level Design (LLD)

### 2A. Crate layout + dependency direction (frozen, acyclic)

```
meku/                  # binary: Application::new, window, keymap wiring
crates/
  meku-vault/          # Mekuto, scan_md, watcher, session.json I/O — no GPUI dep
  meku-index/          # FileIndex, FileMeta, Heading, fuzzy_match — deps: vault
  meku-buffer/         # OpenBuffer (ropey Rope + version + dirty) — deps: vault
  meku-markdown/       # pure StyledDoc pipeline — deps: pulldown-cmark only
  meku-editor/         # LivePreviewView, offset<->visual mapping — deps: buffer+markdown+theme
  meku-workspace/      # Sidebar/Tree, Dock/Tabs, Palette, Workspace state — deps: all above
  meku-theme/          # Zed-like tokens, font stack, MekuAction/keymap — deps: gpui+gpui-component
```

- [x] Create workspace `Cargo.toml` with exact `gpui-pre` pins + `Cargo.lock` committed (5 members; `gpui-kit = "=0.7.1"` umbrella only, `Cargo.lock` committed on pre-release 2026-10-08)
- [x] Enforce direction: `markdown` knows no FS/GPUI · `buffer/vault/index` know no views · `editor` never writes FS (via `workspace.request_save`) · `theme` is a leaf
- [ ] Add `meku-ui` facade over `gpui-component` (so kit swap doesn't ripple) + feature-audit (import only used modules)

### 2B. Live-preview editor LLD

Pipeline: `keystroke → OpenBuffer.apply_edit (<1ms) → debounce 80–120ms Task → background_spawn parse(snapshot,version) → drop if stale → StyledDoc → Entity update → Render (cached doc, rebuild active line only)`.

- [x] Define `meku-markdown` types: `SourceSpan{start,end}` · `BlockKind{Para,Heading(u8),CodeFence(Option<String>),List,Quote,Hr,Table}` · `StyledSpan{start,end,style}` · `StyledBlock{kind,span,spans}` · `StyledDoc{version,blocks,headings}` · `parse()` + `parse_fallback()` (single-Para, never blank) — `crates/meku-markdown`, 8 tests green
- [ ] Define `meku-editor` view: `LivePreviewView{buffer: Entity<BufferModel>, doc, pending: Task}` · `schedule_reparse()` · `on_parse_done()` (stale-version drop) · `active_byte_range()` (cursor line → `\n` boundaries) · `offset_to_visual()`/`visual_to_offset()` (binary search, round-trip invariant)
- [x] Render rule: every row styled **except** caret row → raw with dimmed markers; selection resolved in source-offset space, painted per-row; single surface, no editor swap (done in `document.rs` + `meku-markdown::live`)
- [ ] Targets: raw-line keystroke→paint <16ms · styled settle ≤150ms

### 2C. Workspace / mekuto LLD

- [x] Define `meku-vault`: `Mekuto{root}` · `VaultFile{rel,abs,mtime}` · `VaultEvent{Created,Modified,Removed}` · `open_root/scan_md(sorted, skips .meku)/start_watcher(.meku-filtered)/session_path` · session v1 + atomic save + corrupt-backup — `crates/meku-vault` (`notify 8` stable, not the 9 RC)
- [x] Define `meku-index`: `FileIndex{rel→FileMeta}` · `FileMeta{title,headings,mtime}` · `rebuild/upsert/remove/fuzzy_match` (ATX scan, fenced-code aware) — `crates/meku-index`
- [x] Define `meku-buffer`: `OpenBuffer{rel,text:Rope,version,dirty,disk_mtime}` · `load(streaming)/from_text/apply_edit(byte-range)/mark_saved/snapshot` — `crates/meku-buffer` (`ropey 2.0.0-beta.1`, byte-native API, `metric_lines_lf`)
- [ ] Define `meku-workspace`: `Workspace{vault,index,tabs,dirty}` · `TabState{rel,buffer_id,cursor,scroll_px}` · `Session{root,open_tabs,active}` · `open_folder/open_file/on_watcher_event/request_save(atomic tmp+rename)/poll_autosave 800ms/resolve_external_change{KeepMine,LoadDisk}`
- [ ] Tree rule: `*.md` sorted case-insensitive, ignore `.meku/`+dotfiles, incremental `VaultEvent` updates (no full rescan); watcher: `notify` inotify + 50ms coalesce → `cx.spawn` channel

### 2D. Theme / keymap / shell LLD

- [ ] Extend `gpui-component` `Theme` (don't fork): `MekuTokens{editor_bg,line_active,caret,md_h1…}` · `apply_zed_like_theme()` (radius 8px, Zed dark `#171717/#1e1e2e`, accent `#89b4fa`, border white/10) · `font_stack(["Zed Plex Mono","JetBrains Mono",monospace])`
- [ ] Layout: `DockArea` + `DockLayout::h_split` (sidebar 240px | center tabs | outline 220px stub) · `Sidebar/TabBar/Tree/StatusBar/CommandPalette(searchable_list)`
- [ ] Actions: `MekuAction{OpenFolder,OpenPalette,Save,CloseTab,NextTab,ToggleSidebar}` via `cx.bind_keys` + `on_action`; palette = actions + `FileIndex::fuzzy_match`; Linux `Ctrl` bindings verified on Mesa+NVIDIA

### 2E. Testing seams (per module, no GUI unless noted)

- [x] `meku-markdown`: golden `md→StyledDoc` asserts (headings/offsets, strong/emph/strike/code spans, tasklist, fence lang, table) + unclosed-fence no-panic + fallback single-para
- [x] `meku-buffer`: version bumps, replace/multibyte byte-ranges, snapshot isolation, tempdir load round-trip
- [x] `meku-vault`: tempdir `scan_md` ignores `.meku/` + sorted, watcher delivers Created, sidecar writes ignored, corrupt session → empty no-panic
- [x] `meku-index`: upsert/remove headings, fenced-code exclusion, filename-fallback title, fuzzy ranking
- [ ] `meku-editor`: active-line calc, stale-drop, one headless GPUI type→styled test
- [ ] `meku-workspace`: fake watcher → dirty dot / external prompt / tmp+rename autosave / session round-trip
- [ ] `meku-theme`: token hex snapshot, keymap contains `Ctrl+O/P/S/W`, manual contrast check

---

## Section 3 — Build (implementation order, commit after each)

- [x] **0. Scaffold + hello-window** — workspace + `crates/meku` on latest `gpui-kit 0.7.1` (pins `gpui-pre 0.3.8`), `gpui_kit::init` + `Root` pattern, `cargo check -p meku` green (2026-10-08). → _ask user to commit_
- [x] **1. App shell (Zed theme)** — TitleBar + Sidebar (Explorer/Search/Outline) + TabBar (new/select/close) + StatusBar, `h_resizable` 240px shell, keymap `Ctrl-B/T/W/Tab` under `Meku` context, empty states. `check`+`fmt`+`clippy` green (2026-10-08). → _ask user to commit_
- [x] **2. Mekuto open + file tree** — `rfd` dialog (`Ctrl-O`) + CLI arg, file tree (dirs-first, expand/collapse, click-to-open tabs), context-menu ops (new note/folder via modal, inline-validated rename, delete confirm modal), 200ms watcher pump (external edit reload + remove closes tabs), session restore/save. `test(43)`+`check`+`fmt`+`clippy` green (2026-10-09). → _committed on pre-release_
  - 2026-10-09 shell layout fix (status bar floated mid-window): root was a block `div()`, so `flex_1` on the middle was ignored and the bar followed the content. Root is now `v_flex().size_full()`; middle / centre body / document / sidebar tree are flex containers with `flex_1().min_h_0()` (+ `overflow_y_scroll` on scroll areas); status bar is `StatusBar` (gpui-kit) in a `flex_none` wrapper as the last child. Guarded by 3 headless geometry tests (1200×800, 1158×763, 60-paragraph note); verified each fails without its fix.
  - Theme note: the status bar uses gpui-kit's `StatusBar` (user decision), so Kit theme tokens are used there; this is an accepted exception to the `meku-ui` facade rule in 2A until the facade lands.
  - 2026-10-09 fix (from user screenshot: empty sidebar/tree): the virtualized `Tree`/`TreeState` **and** `Sidebar` (both `uniform_list`/`ListState`-driven) render zero rows when starved of measured height in this layout — replaced with plain `ListItem` rows from `Explorer::flat_rows()` + plain themed sidebar divs. Also fixed: empty notes showed no editor (no blocks → no active region; now defaults to `{0,0}`).
  - Esc **does** cancel the naming modal while the Input is focused (verified by headless UI test, not just analysis): Input's `escape()` ends in `cx.propagate()`, and our later-added no-context `CancelOverlay` binding dispatches next. No fix needed.
  - Headless UI tests live in `crates/meku/src/main.rs::ui_tests` (manual `run_ui_test` harness mirroring the `#[gpui::test]` expansion — see AGENTS.md for why the attribute macro can't be used). Covered: Esc-cancel, Enter-commit, invalid-name-keeps-modal.
- [x] **3. Editor buffer + tabs** — `Editor`/`EditorState` per open note (lazy, buffer-synced via Change events), dirty `•` dots + status bar, built-in undo/redo (`Ctrl-Z/Y`), `Ctrl-S` + 800ms-idle autosave, external-change conflict banner (Load disk/Keep mine), identical-content reload is a no-op (no cursor jumps on own saves). `test(35)`+`check`+`fmt`+`clippy` green (2026-10-09). → _committed on pre-release_
- [x] **4. Live preview (inline GFM)** — single document surface (`crates/meku/src/document.rs`): one `list()` row per source line, caret row raw with dimmed markers, other rows markup-hidden (`meku-markdown::live::build_rows/project`); `StyledText` runs (bold/italic/strike/code/link, heading scale), quote borders, code-fence rows, HR; buffer-native caret/selection/undo + `EntityInputHandler` (IME, clipboard, arrows/Home/End/Backspace/Delete/Enter); click/drag moves caret, edits re-render in place (no editor swap). `test(19+16+29)`+`check`+`fmt`+`clippy` green (2026-10-10). → _ask user to commit_
  - Replaces the per-block `Editor`-swap prototype (UX failure: caret jumps, block-local undo, two-click cursor). Old `preview.rs` removed.
  - Caret is a row-local element positioned by synchronous shaping (`shape_line` + `split_at` prefix width, same text/runs/size as painted); `TextLayout::position_for_index` is paint-phase-only (panics in render) and reports window-space, so canvas overlays are out. Guarded by `caret_matches_painted_glyphs` (±1.5px vs painted glyphs).
  - Known v0.1 simplifications: tables render as raw monospace (no cell grid yet) · all rows render every frame (virtualization is Build 7).
- [x] **5. Preview mode + highlight** — Edit/Live/Read toggle (`Ctrl-E` cycles, right-aligned switch above the document; Read hides the caret and blocks edits), tree-sitter highlighting for fence languages rust/python/js-ts/bash/markdown (`crates/meku-markdown/src/highlight.rs`, official grammar queries, OnceLock configs + 256-entry span cache; unknown languages stay plain), standalone `![alt](target)` image rows render `gpui_kit img()` in Live/Read hidden rows with fixed 480×240 frame + loading/fallback placeholder (paths resolved relative to the note, fused to the vault root), mode switches keep the scroll position (ListState untouched). `test(27+38)`+`check`+`fmt`+`clippy` green (2026-10-18). → _commit_
  - Grammar crates (latest available): `tree-sitter-rust 0.24` / `tree-sitter-(python|bash|javascript) 0.25` / `tree-sitter-md-025 0.5.6` (md grammar fork compiled against the unified tree-sitter 0.25 runtime); highlight queries come from the grammar crates themselves (`HIGHLIGHTS_QUERY`/`HIGHLIGHT_QUERY`), markdown block+inline wired via injection callback.
  - Tests guard: Read blocks typing + IME + no caret; mode cycle Edit→Live→Read→Edit via key AND buttons; scroll kept across Read/Edit; image drawn only when its row is hidden (Edit shows raw); rust fence keyword span from the model (`DocModel.hl`).
  - Caret is centered on the glyph box, not the line-box top: gpui paints glyphs with `padding_top = (line_height − ascent − descent)/2` (see `text_system/line.rs::paint_line`), so `caret_geometry` returns `(advance, padding_top, font_box)` and blank rows offset by `(row_h − font_size)/2`. Glyph-match test now asserts against the painted (padded) position via `line_layout_for_index` metrics.
- [ ] **6. Search + palette + outline** — `Ctrl-P` finder, `Ctrl-Shift-P` palette, full-text (grep lib), heading tree + follow-cursor. → _commit_
  - Landed with Build 5.x (from the live-markdown feature file §2, suggested-build-order step 2): word/line navigation — `Ctrl+←/→` word jump (camelCase humps, punctuation runs as one stop, whitespace skipped), `Ctrl+Shift+←/→` extend, `Ctrl+Backspace`/`Ctrl+Delete` word deletes, `Ctrl+Shift+Backspace` delete-to-line-start (joins lines at line start), smart `Home` (first non-blank ⇄ column 0), `Ctrl+Home/End` doc start/end, `PageUp/PageDown` + Shift variants (±1 viewport of rows, column kept) — buffer logic in `meku-buffer` (`word_left/word_right/delete_*`, unit-tested incl. rope-chunk-crossing long doc), bindings + handlers + 5 headless UI tests. `test(32+27+38)`+`check`+`fmt`+`clippy` green. → _commit_
- [ ] **7. Perf + polish** — virtualized lists everywhere, 120ms debounce audit, lazy images/grammars, benches green, RSS <120MB, docs + Linux packaging. → _commit_

## Section 4 — `live-markdown-editor-features.md` tracking

_Reconciled from the feature file (`live-markdown-editor-features.md`, editor scope only). One commit after each subsection, same workflow as Section 3. Unchecked items stay unchecked; the GFM/v0.1 deferral rules still apply where noted. Suggested build order lives in the file (§Suggested Build Order 1–13)._

### F1 Core Text Input (§1)
- [x] Plain text typing with IME support (EntityInputHandler composition, Build 3/4)
- [x] Undo/redo with typing-burst coalescing (`meku-buffer` coalescing, Build 3)
- [x] Cut/copy/paste plain text (Build 3)
- [x] Select all (Ctrl+A)
- [x] Auto-save (800ms idle) + dirty dots (Build 3)
- [ ] Multi-copy paste rich-to-markdown conversion
- [ ] Select word / line / paragraph + expand selection
- [ ] Multiple cursors (Ctrl+Alt+Up/Down, Ctrl+D)
- [ ] Column/box selection (Alt+drag)
- [ ] Soft wrap toggle
- [ ] Line numbers toggle (optional, source mode)
- [ ] Spellcheck with underline + right-click suggestions (defer v0.1 — Linux spelllib decision)

### F2 Word and Line Navigation (§2) — **Build 5.x, landed**
- [x] Ctrl+Left/Right jump by word (camelCase humps, punctuation runs are one stop, whitespace/cross-line skipping)
- [x] Ctrl+Backspace deletes the previous word · Ctrl+Delete deletes the next word
- [x] Ctrl+Shift+Backspace deletes to line start (joins lines when already at the start)
- [x] Home: first non-whitespace, second press column 0
- [x] End goes to end of line
- [x] Ctrl+Home / Ctrl+End go to document start/end (+ Shift variants)
- [x] PageUp / PageDown (+ Shift variants; viewport-sized, column kept)
- [x] Shift+Arrow extends selection (existing, Build 4) + Ctrl+Shift+Left/Right extends by word
- [ ] Word jump respects markdown syntax more granularly (link targets, fence strings)
- [ ] Go to line (Ctrl+G)
- [ ] Jump to heading from outline (builds with palette/outline)
- [ ] Bookmarks / jump back (Alt+Left/Right)

### F3 Line Editing Shortcuts (§3) — **landed 2026-10-19**
- [x] Move line up/down (Alt+Up/Alt+Down) — block-aware (selection spans move as a block), caret/selection ride along; no-op at the first line / over the phantom line after a trailing newline
- [x] Duplicate line/selection (Ctrl+Shift+D) — copy lands below; a last line without a newline gains its own separator; caret keeps its offset in the copy
- [x] Delete line (Ctrl+Shift+K) — block-aware, newlines included, caret to block start
- [x] Insert line below/above (Ctrl+Enter / Ctrl+Shift+Enter)
- [x] Join lines (Ctrl+J) — drops the newline + next line's indent; one space at the seam unless either side is empty or already has whitespace
- [x] Toggle comment in code (Ctrl+/) — only inside a code fence; prefix from the fence language (`//` rust/js/ts, `#` python/bash/etc.), indent kept, blank lines untouched, one undo step; no-op outside fences
- [x] Sort selected lines — `OpenBuffer::sort_selected_lines` (stable byte order), wired as a palette command in Build 6/F11 (no default key)
- [x] Transform case (upper/lower/title/toggle) — `OpenBuffer::transform_case(CaseKind)`; collapsed caret targets the alphanumeric word under it; palette command in Build 6/F11
- Buffer unit tests (15) + 6 headless UI tests drive the real keystrokes; `test(38+53)` + fmt/clippy green (2026-10-19).

### F4 Enter and Smart Continuation (§4) — **next chunk**
- [ ] Enter continues bullets / exits empty bullets, numbered lists increment + renumber, task items `[ ]`
- [ ] Enter continues blockquotes/callouts, verts indentation, soft break (Shift+Enter)
- [ ] Enter inside code blocks / tables; Enter after a heading starts a paragraph

### F5 Indentation and Lists (§5) — **next chunk**
- [ ] Tab/Shift+Tab indent/outdent (lines, selection, list children)
- [ ] Backspace removes one indent level
- [ ] Convert line to bullet/numbered/task; toggle checkbox (Ctrl+Enter or click)
- [ ] Ordered-list renumbering; fold list items (defer folding to Build 7 polish)

### F6 Auto-Pairing and Smart Typing (§6) — **later**
- [ ] Auto-close `()[]{}""''\`\`` + markdown markers `** * == ~~`; skip-over closers; wrap selection
- [ ] Backspace deletes an empty pair; `[[` wikilink autocomplete (with F9)
- [ ] Settings toggles (auto-close off, smart quotes/dashes off by default)

### F7 Live Preview Rendering (§7)
- [x] Render while typing, single surface; syntax hidden unless on that line; reveal on caret entry (Build 4)
- [x] Headings styled by level (H1–H6 scale), bold/italic/strike/code/links, HR, blockquote border, task checkboxes + clickable, images (Build 4/5)
- [x] Code fences with syntax highlighting (5 langs; Build 5) — [ ] language label chips
- [ ] Code block language selector · inline `==highlight==` spans
- [ ] Inline math `$...$` / block `$$...$$` (defer — v0.1 deferral list)
- [ ] Mermaid (optional, defer)
- [ ] Tables as grids (Build 6/13)
- [ ] Footnotes with click preview; comments `%%…%%` hidden; emoji shortcodes (defer)
- [ ] Superscript/subscript
- [ ] `![[embed]]` inspection — deferred (no wikilinks in v0.1)

### F8 Source / Live / Reading modes (§8) — **Build 5, landed**
- [x] Source/Edit mode: raw markdown (fences highlighted)
- [x] Live preview mode as default
- [x] Reading mode fully rendered + read-only
- [x] Ctrl+E cycle + buttons; mode preserved per document session (needs session.json write check)
- [x] Reading mode shares theme/typography

### F9 Links / Wikilinks / Embeds (§9) — **defer v0.1** (no wikilinks/backlinks; `[[x]]` renders literally)
- [ ] Full set per feature file (autocomplete, alias, anchors, block ids, follow, hover preview, broken-link highlighting)

### F10 Tags / Properties / Frontmatter (§10) — **defer v0.1**
- [ ] Tags autocomplete + highlight; YAML frontmatter parsed as properties + properties UI; aliases/titles

### F11 Slash Commands and Command Palette (§11) — **Build 6**
- [ ] Inline `/` command menu + commands set (headings, lists, task, quote, callout, code, table, divider, image, link, date/time)
- [ ] Fuzzy search + keyboard navigation
- [ ] Ctrl+P finder, Ctrl+Shift-P palette; every command bindable; recent commands ranked

### F12 Formatting Shortcuts (§12) — **later**
- [ ] Ctrl+B/I, strikethrough, Ctrl+K link, Ctrl+\` code, Ctrl+1..6 heading toggles, quote/callout toggle, current-word without selection, clear formatting

### F13 Tables (§13) — later (Build 6/13)
- [ ] Insert + picker, Tab/Shift+Tab cell nav + auto-add row, add/remove rows/cols, alignment toggle, sort, pipe auto-format, CSV/TSV paste

### F14 Code Blocks (§14) — partially landed
- [x] Fenced blocks + language identifier + highlighting (5 langs, Build 5)
- [ ] ≥50 languages (add more grammar crates), selector + autocomplete, copy button
- [ ] Tab-in-code setting, auto-indent on Enter, bracket matching, inline code double-backticks, line numbers/folding

### F15 Headings / Outline / Structure (§15) — **Build 6**
- [ ] Outline panel from headings + click to jump
- [ ] Fold/unfold heading sections + all (Build 7 polish)
- [ ] Heading anchors; sticky breadcrumb (optional); promote/demote level

### F16 Find / Replace / Search (§16) — **Build 6**
- [ ] Ctrl+F find + highlight all, Ctrl+H replace + replace-all, case/whole-word/regex toggles, match count + next/prev, group captures, search-in-selection
- [ ] Highlights in both source and preview

### F17 Images and Media (§17) — partially landed
- [x] Standalone image rows render inline (Build 5)
- [ ] Drag-and-drop / paste-to-insert into attachment folder; `![[img.png|300]]` sizes
- [ ] Video/audio/PDF embeds (optional); alt-text captions polish

### F18 Paste and Clipboard Handling (§18) — later
- [ ] HTML→markdown paste, Ctrl+Shift+V plain, URL-over-selection link, list paste continuation, spreadsheet→table, copy as rendered

### F19 Selection / Movement refinements (§19) — partially landed
- [x] Shift+Arrow / Ctrl+Shift+word extend (Build 4 + 5.x)
- [ ] Double-click selects word, triple-click selects line
- [ ] Shift+click extends selection
- [ ] Drag text to move; Escape clears selection/multi-cursors (Escape top priority when implemented)

### F20 Visual and Theme (§20) — later
- [x] Syntax Highlighting colors for markdown tokens (Build 5)
- [ ] Light/dark + system follow, custom variables, adjustable font family/size/line height, width options/centered column, zoom Ctrl+=/-, focus mode, typewriter mode, distraction-free, show/hide frontmatter

### F21 Status and Feedback (§21) — partially landed
- [x] Save state indicator (dirty digits/status, Build 3)
- [ ] Word/character count + reading time, selection counts, line:column indicator, non-blocking save failure

### F22 Accessibility and Input Options (§22) — defer v0.1
- [ ] Keyboard-completeness audit, screen-reader labels, focus indicators, reduced-motion, vim/emacs modes, keymap customization, RTL, spellcheck language

### F23 Performance and Robustness (§23) — Build 7
- [ ] 100k+ line docs, incremental re-render, off-main parsing, lazy images/languages, large-file guard, crash snapshots
- [ ] Undo history survives tab switching (partially — buffer state persists in, verify), cursor/scroll restore (needs session.json write)

### F24 Extensibility hooks (§24) — defer
- [ ] Events, custom commands/hotkeys/renderers/slash/autocomplete providers, sandboxing

_Deferred past v0.1: links/backlinks/graph, vim, plugins/sync, export PDF/HTML, minimap, >100k-line mmap paging, tantivy, settings UI beyond ~10 keys._

