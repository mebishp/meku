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
- [x] Freeze explicit deferrals: wikilinks (dropped for good), embeds, backlinks, graph/canvas, tags pane, frontmatter UI, math, mermaid, vim, plugins/sync/publish, terminal/AI
- [ ] Write MVP acceptance criteria into `docs/acceptance.md` (12 criteria: open mekuto <1s/100 files, file ops, live-preview round-trip, tabs+dirty, Ctrl-P <100ms/1k files, search, palette, outline ≤200ms, status bar, settings persist, crash recovery ≤2s loss, GFM fixture zero panics)
- [x] Freeze keymap defaults (Linux): `Ctrl-P` finder · `Ctrl-Shift-P` palette · `Ctrl-B` explorer · `Ctrl-,` settings · `Ctrl-N/W/T` note/close/tab · `Ctrl-S` save · `Ctrl-F` find · `Ctrl-E` edit/live/read cycle (Build 5) · `Ctrl+←/→` word jump · `Ctrl+Backspace/Delete` word delete · `Ctrl+Shift+Backspace` delete-line-start · `Ctrl+Home/End` doc ends · PageUp/Down · smart `Home` (Build 5.x) · `Alt+↑/↓` move line · `Ctrl+Shift+D/K` duplicate/delete line · `Ctrl+Enter/Shift+Enter` insert below/above · `Ctrl+J` join lines · `Ctrl+/` comment toggle in code (F3) · `Shift+Enter` soft break, fence-aware `Enter` continuation (F4)
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
Rules: only `*.md/*.markdown` are notes · `.meku/` hidden from tree + search · rename rewrites nothing (no link rewriting yet) · delete confirms · external edit → Reload/Keep-mine prompt.

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
- [ ] **6. Search + palette + outline** _(outline landed 2026-10-19: sidebar OUTLINE section from the live row model — fenced `#` excluded, caret-following highlight, click-to-jump, headless-tested; remaining: Ctrl-P finder, Ctrl-Shift-P palette, full-text search)_ — — `Ctrl-P` finder, `Ctrl-Shift-P` palette, full-text (grep lib), heading tree + follow-cursor. → _commit_
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
- [x] Select word / line / paragraph + expand selection — double-click word, triple-click line, `Ctrl+D` expands word → line → paragraph → document
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
- [x] Jump to heading from outline (click an Outline entry → caret + scroll; landed with Build 6 outline)
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

### F4 Enter and Smart Continuation (§4) — **landed 2026-10-19**
- [x] Enter continues bullets / exits empty bullets, numbered lists increment + renumber, task items `[ ]` — `OpenBuffer::smart_enter`: content split rides to the new marker (leading whitespace of the suffix stays below, one undo step); empty item exits the list; ordered lists increment from the caret item and renumber everything behind (raw `apply_edit`s batched), first number + `.`/`)` separator preserved; task markers always spawn unchecked `[ ]`; nested quote+list continues both
- [x] Enter continues blockquotes/callouts, keeps indentation, soft break (Shift+Enter) — quote `>` prefixes repeat (callout `> [!x]` tags do **not**), the exact inner indent of the current item is reused, `soft_break` copies the marker column as spaces (no marker invented); heading/plain lines get a bare newline (Enter after a heading starts a normal paragraph)
- [x] Enter inside code blocks inserts a plain newline (fence-aware via `fence_at`, list/quote magic disabled there); table-row Enter and renumber-on-delete deferred to F13/later
- Buffer unit tests (18) + 6 headless UI tests drive the real keystrokes; `test(44+65)` + fmt/clippy green (2026-10-19).

### F5 Indentation and Lists (§5) — **landed 2026-10-19**
- [x] Tab/Shift+Tab indent/outdent — collapsed caret on a list item nests/unnests the item **with its children** (2 spaces, 3 under ordered); selection shifts every touched line (selection rides along); prose Tab inserts a level at the caret; in code fences a level is 4 spaces and list logic is off. One undo step each
- [x] Backspace in leading whitespace removes one indent level (snaps to the 2-space stop; a tab goes whole); anywhere else unchanged
- [x] Convert line(s) to bullet/numbered/task (Ctrl+Shift+8/7/9, same kind toggles off, numbering counts converted lines, blanks/quotes skipped, indent kept); toggle checkbox `Ctrl+L` (flips tasks, promotes bullets/plain to `- [ ]`) — `Ctrl+Enter` stays insert-line-below from F3, so the toggle took `Ctrl+L`
- [x] Ordered-list renumbering now steps over nested children, and indent/outdent restart (`1.`) or rejoin the nested run and straighten both levels
- [ ] Click a rendered checkbox to toggle (not wired yet — no click target exists in `document.rs`) · fold list items (defer to Build 7 polish)
- Buffer unit tests (10) + 4 headless UI tests drive the real keystrokes; `test(48+75)` + fmt/clippy green (2026-10-19). Also restored `crates/meku-index` (had been moved under `meku-markdown/`, breaking the workspace manifest).

### F6 Auto-Pairing and Smart Typing (§6) — **landed 2026-10-19**
- [x] Auto-close `()[]{}` `""` `''` `` ` `` and markdown markers `**` `*` `==` `~~`; typing a closer over its twin skips it; selection wraps on `* _ \` [ ( { "` — `OpenBuffer::type_char` (single-char plain typing only; IME composition, ranged input and paste never pair). Guards: no pair before a word/after an alphanumeric (`don't`, `2*3`), line-start `*` stays a bullet, ``` ``` ``` types a fence opener, markers never pair inside code fences, `[[` yields `[[|]]`
- [x] Backspace in an empty pair deletes both characters
- [x] Auto-close can be turned off (`OpenBuffer::auto_close`; user-facing toggle lands with the settings UI)
- [ ] Smart quotes / smart dashes (off by default, not built)
- Buffer unit tests (8) + 1 headless UI test; `test(49+83)` + fmt/clippy green (2026-10-19).

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

### F8 Source / Live / Reading modes (§8) — **Build 5, landed**
- [x] Source/Edit mode: raw markdown (fences highlighted)
- [x] Live preview mode as default
- [x] Reading mode fully rendered + read-only
- [x] Ctrl+E cycle + buttons; mode preserved per document session (needs session.json write check)
- [x] Reading mode shares theme/typography

### F9 Links (§9) — plain markdown links only (wikilinks/embeds/backlinks are out of scope; `[[x]]` renders literally)
- [x] Links render as clickable text with the URL hidden until the caret enters (Build 4)
- [x] Ctrl+Click follows a link (any click in Read mode too): http(s)/mailto → system handler; relative `.md` (percent-decoded, `..` resolved, never leaves the mekuto) → opens/switches tab; `#heading` / `note.md#heading` → GitHub-style slug jump. Pure logic in `meku_markdown::live::{link_at, classify_link, heading_slug}`; missing targets do nothing
- [x] Ctrl+K inserts or edits a link — selection → `[text](|)` (URL selection → `[|](url)`), empty → `[|]()`, caret inside an existing link selects its destination
- [x] Paste a URL over selected text creates `[text](url)`; a lone URL stays bare (`OpenBuffer::paste`)
- [ ] Broken-link highlighting when a relative target doesn't exist
- Wikilinks removed from scope entirely (checklist + feature file). Tests: markdown(3) + buffer(2) + 2 headless UI; fmt/clippy green (2026-10-19).

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
- [ ] Drag-and-drop / paste-to-insert into attachment folder
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

## Section 5 — `design-guidelines.md` adoption (spacing / layout only)

_Fonts and colors are **excluded** by decision; only spacing, margins, layout, caret and copy are followed. Items judged to hurt the look or fight the row-per-line model are deliberately skipped._

- [x] Reading column ≈68ch (37.5em), centered; 40px minimum side padding; 56px above the first line; 45% of the window as bottom scroll room
- [x] Headings: space above 1.6em (H1–2) / 1.3em (H3+) and below 0.5em / 0.4em of the heading size (minus the blank row that already precedes it); first-row heading gets no extra top pad
- [x] `---` renders as a centered 40px × 1px rule; list nesting 1.5em; list item gap 0.25em; blockquote indent 1.2em per level
- [x] Caret width `max(1.5px, 0.125em)` snapped to whole pixels (scales with headings); stepped blink 530ms on / 530ms off, solid for 600ms after any key/click/drag; no blink in Read mode
- [x] Window default 1100×760, minimum 640×420; "Outline" label in sentence case (no tracked-out all-caps); file-changed banner uses guideline copy (`Reload` / `Keep my version`)
- [ ] Skipped on purpose: 1.7 line height (caret geometry depends on text metrics) · caret glide / marker fade (needs reserved marker slots) · drawn 5px bullets · per-line rounded code corners (seams) · slash menu, find bar, scrollbars, toasts, typewriter mode, overwrite caret (belong to their own builds)
- Tests: column width/centering, 56px top offset, 40px narrow-window padding, caret width; `test(55+…)` + fmt/clippy green (2026-10-19). Not verified visually — needs a graphical session.

## Section 6 — Inline title + new note flow + fonts (2026-10-19)

- [x] **Inline title (Obsidian style).** Every open note shows its file name as a large bold, borderless title above the body. It is UI, not markdown: never in the file, scrolls with the document, can't be removed. Editing it renames the file — on Enter or blur immediately, otherwise after 800ms idle. An empty/unusable title snaps back to the current name; a name already taken is refused with a notice and snaps back. Tabs, buffers and the tree follow the rename. (`title_to_stem` strips `/ \ : * ? " < > |`, controls, edge dots/spaces; 100 chars.)
- [x] **New note = no dialog.** `+ Note`, the context menu and `Ctrl+T` (when a mekuto is open) create an empty `Untitled.md` (`Untitled 2.md`, … on collision) and open it with the title focused and selected, so typing names it. Folder create and rename keep their dialog.
- [x] Fonts: body/UI prefers **Inter** → Adwaita Sans → Cantarell → Liberation Sans; code and inline code **JetBrains Mono** → Adwaita Mono → DejaVu Sans Mono → Liberation Mono (first installed wins; app entry only so tests keep default metrics). Headings bold at every level.
- Replaces the earlier `# Untitled` heading ⇄ file-name sync (removed). Tests: untitled → type title → Enter renames + tab follows + title can't be emptied; name clash refused and snaps back; full suite + fmt/clippy green. Title styling (size/alignment) not verified visually.
- **Title bug-fix pass (2026-10-19, from user screenshot):** (1) renaming moved tabs/buffers to `name.md/` (trailing separator from `join("")`), so every later save/rename of that note failed — also affected the context-menu rename; fixed with `join_suffix`. (2) Enter (and any doc key) in the title also edited the body; document key actions now ignore input while the title is focused. (3) Title text was clipped (kit `Input` is 24px tall for custom sizes); size/height now set directly. (4) Idle rename no longer rewrites the text under the caret (normalizing only on Enter/blur). (5) A pending title edit is committed on body click, tab switch/close, open and save. Closing a tab drops its title input. Regression tests: repeated rename of an existing note (+ save to new path, no trailing slash, body untouched by Enter), click-into-body flush, idle commit keeps typed trailing space. Note: gpui delivers subscription events after the enclosing `update_window`, so multi-step UI tests must split steps across calls.
- **False "changed outside the editor" banner (2026-10-19):** the watcher reports our own save a moment later; if more had been typed meanwhile the dirty buffer no longer equalled the disk, so it looked like an outside edit. Buffers now remember a hash of the content last loaded/saved (`OpenBuffer::disk_hash`, `meku_buffer::hash_text`); a watcher event whose disk content matches it is our own write and only refreshes the mtime. "Keep my version" and disk reloads update the hash too, so the same external content can't re-raise the banner. Test: save → keep typing → event = no banner; real outside write = banner; keep-mine silences repeats.

## Section 7 — Subsections, checkboxes, easy links (2026-10-19)

- [x] **Three nesting levels.** List depth is computed by level (an item is one below the nearest shallower open item, so 2-space bullets and 3-space ordered lists nest alike) and capped at 3 (`meku_markdown::live::{nest_depth, MAX_LIST_DEPTH}`). **Tab** nests an item with its children one level and is refused when it wouldn't really go deeper (no sibling above to nest under) or would pass level 3; **Shift+Tab** un-nests. Applies to bullets, tasks and ordered items.
- [x] **Bullets follow the level** when rendered: `•` → `◦` → `▪`. Nested items are indented 1.5em per level with a faint vertical guide per ancestor so they read as subsections.
- [x] **Togglable checkboxes** using the gpui-component `Checkbox` control: rendered task rows show the control instead of `☐/☑` glyphs; clicking flips `[ ]`/`[x]` in the source (`OpenBuffer::toggle_task_box`, one undo step, caret untouched, note marked dirty). Presses on the box never move the caret. `Ctrl+L` still toggles from the keyboard.
- [x] **Links are one click.** On any rendered row (Live mode away from the caret, and everywhere in Read) a plain click on link text follows it, with a pointing-hand cursor while hovering; on the caret row / Edit mode the raw source is shown, so a plain click just edits. `Ctrl+click` follows anywhere.
- Tests: markdown (nesting levels, glyph per level, task run), buffer (3-level cap, box toggle), UI (Tab to level 3 then stop, checkbox click, plain link click vs caret row). Look/alignment of the checkbox and guides not verified visually.

## Section 8 — Big run: themes, features, motion (2026-10-19)

_Decisions (user): several theme presets · tables as grids · callouts + code-block chrome · find/replace + Ctrl-P + palette + vault search · slash menu + formatting shortcuts · subtle & fast motion · verify each step with screenshots · commit per major change. Screenshot harness: `MEKU_THEME`, `MEKU_MODE`, `MEKU_CARET`, `MEKU_ROW`, `MEKU_MODAL` env hooks + extra CLI args open notes at startup; capture through the XDG screenshot portal (scripts in the session scratchpad)._

- [x] Audit P1 fixes: tab strip no longer widens the pane/window (scrolls), modal scrim no longer fades the dialog card.
- [x] **Theme presets** (`crates/meku/src/theme.rs`): Meku Light, Meku Dark, Solarized Light/Dark, Nord, Gruvbox Dark, Dracula. A `Palette` drives the kit `ThemeConfig` (every shell surface) *and* the editor's syntax/inline-code colors; contrast of body/muted/code/syntax colors is asserted per preset in a unit test. Settings persist in `~/.config/meku/settings.json` (`theme`, `follow_system`, `reduce_motion`); default follows the OS light/dark appearance (live re-resolve when the OS flips). `Ctrl+Alt+T` cycles presets until the command palette lands. Fenced code no longer inherits the inline-code color; caret uses the theme `caret` token. Verified by screenshots (light/dark/solarized/nord/dracula).
- [x] **Palette overlay** (`crates/meku/src/palette.rs`): `Ctrl+P` fuzzy file finder (all notes, also inside collapsed folders), `Ctrl+Shift+P` command palette (14 commands with shortcut hints), theme picker with **live preview** (arrow through themes, Enter keeps, Esc restores), `Ctrl+Shift+F` vault-wide text search (jumps to the line), `Ctrl+G` go to line. Up/Down/Enter/Esc/click; scrim always darkens (`theme::scrim`). Tests: open/filter/open note, arrows+wrap+filter+Esc, command run, theme preview restore, go-to-line + search jump. Checks F2 "Go to line" and Build 6 palette/search items.
- [x] **Find & replace in note** (`crates/meku/src/find.rs`): `Ctrl+F` / `Ctrl+H`, floating bar with count ("3 of 12"), prev/next (↑ ↓, Enter / Shift+Enter, F3 / Shift+F3), toggles for case (`Aa`), whole word (`W`) and regex (`.*`, errors shown as "Invalid"), replace one / all (`$1` groups in regex mode; replace-all is one undo step via `OpenBuffer::replace_ranges`), all matches highlighted in the document (current stronger), the current match is selected, seeded from a single-line selection. Incremental search is anchored to where the search started (no drift per keystroke). Palette commands added. Tests: matcher (case/word/regex/replacement), step/close flow, replace one/all + undo.
- [x] **Bug fix: caret on soft-wrapped lines** (found via screenshot) — caret geometry now shapes with the real wrap width (column width tracked from the painted scroll area) and uses the wrapped line's own position; previously the caret of a long wrapped paragraph was drawn at the end of the first visual line.
- [x] **Tables as real grids** — new dependency-free crate `meku-table` (cell splitting with `\|`/code-span awareness, optional edge pipes, delimiter/alignment parsing, column-aligned formatter) used by the markdown model (`Row::table` with role/aligns/cells) and the buffer. Rendered (non-caret) table lines draw as aligned cells: measured column widths cached per document version, bold header, rule under the header, hairlines between rows, `:--:`/`--:` alignment, inline markup inside cells, click a cell to put the caret there; the caret row shows aligned monospace source. Editing: `Tab`/`Shift+Tab` align the table and walk cells (selecting the cell; past the last cell adds a row), `Enter` adds a row (empty last row leaves the table), palette command "Format table". Tests: table crate (7), markdown grouping (2), buffer (6), UI keys. Screenshot-verified in light and dark.
- [x] **Callouts** — `> [!type] Title` blocks (note/info/tip/success/question/warning/danger/bug/example/quote + aliases like `caution`, `tldr`, `fail`) render as one tinted rounded block with a kind icon (kit icons), bold colored title (default title when none), body lines inside; colors come from the active theme; the caret row shows raw `> [!type]` source. (Fold markers `+`/`-` are accepted and hidden; folding itself is not built.) No side stripe (design rule). Model: `Row::callout` (`CalloutKind/Role`), `mark_callouts` pass.
- [x] **Code-block chrome** — fenced blocks render as one rounded container: hidden fence lines become a header strip (language label + **Copy** button → clipboard, "Copied" feedback) and a closing strip; uniform 14px padding; copy excludes the fence lines and never moves the caret. Model: `Row::fence` (`Fence{open, lang}`).
- Screenshot-verified in Meku Light, Meku Dark and Solarized Light.
- [x] **Formatting shortcuts** — `Ctrl+B` bold, `Ctrl+I` italic (stacks correctly with bold via star-run parity), `Ctrl+Shift+X` strikethrough, `Ctrl+Shift+H` highlight, ``Ctrl+` `` inline code (toggle: wrap/unwrap the selection or the word under the caret; markers inside or just outside the selection are recognised; nothing to wrap inserts an empty pair), `Ctrl+1..6` set heading level (same level again or `Ctrl+0` → paragraph), `Ctrl+Alt+Q` toggle quote on selected lines. Skipped inside code fences. **Sidebar toggle moved from `Ctrl+B` to `Ctrl+\`** to free bold (same as Obsidian).
- [x] **Slash menu** (`crates/meku/src/slash.rs`) — `/` at a line start or after a space opens a menu anchored under the caret (Heading 1-3, bulleted/numbered/task list, quote, callout, warning callout, code block, table, divider, link, image, bold, inline code); type to filter (fuzzy), Up/Down, Enter/Tab or click to insert (caret lands in the template's first useful spot), Esc dismisses until that `/` is gone; never opens inside paths/URLs/code fences. The menu stays visible regardless of the caret blink phase (bug found by screenshot). Not yet: date/time entries; menu does not flip above the caret near the window bottom.
- Not built (stay unchecked): multiple cursors, column/box selection.
- [x] **Motion (subtle & fast)** — `theme::{motion, fade_in, fade_from}` (ease-out-quint, 120–180ms): palette panel (140ms fade + 6px rise), slash menu (120ms), find bar (160ms, slides down 6px), naming/delete dialogs (140ms), a note fading in when it becomes the visible tab (160ms), and the row that becomes the editing row eases from 60% opacity over 120ms so markup appearing reads as a reveal. Everything is off with **Reduce motion** (palette command, saved in settings) and automatically in headless tests. Caret movement stays instant (typing latency matters more than glide). Screenshot-verified: overlays settle fully on light and dark.
- [x] The empty "Welcome" placeholder tab is dropped as soon as a real note opens.
- [x] **Inline fixes found by screenshot (Read/Edit modes):** inline `code` showed its backticks when the line was not being edited (the span included the fences) — ticks are now markup; `==highlight==` now renders (bright marker on light themes, warm gold on dark; `==` hidden off the caret row; literal inside code, ignored when it has inner spaces). `Ctrl+Shift+H` toggles it.
- [x] **Perf pass (audit P1/P2):** measured `build_rows` on a 47,500-line note: 147ms (dev) / 24ms (release) → **36ms dev** after raising `opt-level` to 3 for the hot crates in the dev profile (`meku-markdown/-buffer/-table`, `pulldown-cmark`, `ropey`, `regex*`, `memchr`) — ≈8ms for a 10k-line note, within the 16ms typing budget even in dev. The per-row image table moved into the cached `DocModel` (rebuilt per edit, no longer on every caret-blink render). Ignored timing test: `cargo test -p meku-markdown --release perf -- --ignored --nocapture`. Still open: incremental re-parse (only changed lines) for >50k-line notes. Re-measured end to end (ignored test `cargo test -p meku --release perf_typing -- --ignored --nocapture`, 10k-line note, keystroke + frame): p50 14.5ms / p95 18ms release; ~9.4ms of that is the full `build_rows` per edit (pulldown spans 4.4ms + per-line classify 5ms), so incremental re-parse is the only real lever left for the 16ms p95 budget. **Done:** `rebuild_rows` re-parses only the blank-line-bounded block around an edit (falls back to the full build for fences, indented lines, HTML/link-reference docs, list-level changes), fuzz-tested equal to `build_rows` (400k random edits). Typing in a 10k-line note with paragraphs: p50 14.9ms → 5.1ms, p95 17.5ms → 6.9ms. Notes with no blank lines still rebuild fully.

## Section 9 — Chrome redesign, images, polish (2026-10-19)

- [x] Code blocks: the block's first/last rows carry the 8px corners in every mode (also with fences revealed or in Edit mode); `---` separators are full column width.
- [x] **Icon set: Iconoir** (MIT; not in the Lucide/Feather/Heroicons family) — 53 glyphs vendored under `crates/meku/assets/icons/meku/` with its license, generated into `crates/meku/src/icons.rs` by `tools/gen-icons.py` (`Ico` enum, `icon()` helper, `MekuAssets` source layered over the kit's assets). Extend by editing the list in the script.
- [x] **Chrome redesign** (`crates/meku/src/chrome.rs`): title bar = sidebar toggle + breadcrumb (vault › folders › note) on the left, **icon segmented control** for Source / Live / Read (tooltips) + search / command palette / theme buttons on the right; **tab strip** with note icons, dirty dot that becomes ✕ for closing, connected active tab, middle-click close, `+` new note, horizontal scroll that keeps the active tab in view; **sidebar** with vault header and new note / new folder icon buttons, a search field (opens the finder), *Notes* tree with chevron + folder/note icons and hover/selected tints (context menu kept), collapsible *Outline* section, footer with theme + command buttons; **status bar** with Saved/Unsaved state, word count, `Ln, Col` and the view mode (icons). The old Edit/Live/Read text row above the document is gone. Screenshot-verified on Meku Light/Dark and Nord. Tests: tab select/close/add, mode switch, status counts, sidebar search, outline toggle.
- [x] **Images** — standalone `![alt](path)` lines render at their real proportions (size probed from the file header with `imagesize`, cached by mtime; never wider than the column, capped at 460px tall, never upscaled), in a rounded hairline frame with the alt text as caption; `![alt|320](x.png)` sets a width; targets may have spaces (`<…>`), `%20`, a `"title"`; `..` cannot escape the mekuto; missing files show "Image unavailable". **Click an image → lightbox** (`crates/meku/src/lightbox.rs`): fit-to-window view over a dimmed backdrop, zoom −/+ (25%…400%), fit button, percent readout, click the picture to toggle fit ⇄ actual size, scroll when larger than the window, click outside / ✕ / Esc closes; clicking never moves the caret. **Paste an image** from the clipboard (screenshots etc.): saved to `attachments/pasted-<time>.<ext>` next to the note and linked on its own line. Tests: size rules, parsing, lightbox open/zoom/close, paste. Screenshot-verified (light, dark).
- [x] **More chrome features:** back/forward navigation history (title-bar buttons, `Alt+←/→`; reopens closed notes; forward history dropped on a new visit), clickable breadcrumb (folder crumbs reveal that folder in the tree), tab context menu (Close / Close others / Close all / Copy path / Reveal in sidebar), the sidebar tree auto-reveals the active note, clickable status items (`Ln, Col` → go to line, mode → cycle), and an Obsidian-style **empty state** (New note / Open a note / Command palette with shortcuts; "Open a folder" before a mekuto exists). Callout icons and the code-block Copy button now use Iconoir (no Lucide icons left in Meku's own UI; kit-internal widgets such as the checkbox tick keep theirs).

## Section 10 — Seamless live editing, Obsidian images (2026-10-19)

- [x] **Code blocks no longer split:** blank lines inside a fence stay code rows (they used to become blank rows and cut the container in two); empty code lines get a line of height and a caret.
- [x] **Rendered while editing.** In Live the line you are typing on stays *rendered* — bullets, checkboxes, bold/italic/code, links — and raw markdown appears only where you work on the markup itself: lines touched by a **selection** (double-click, drag, Shift+arrows, Ctrl+D…), **headings** (unchanged: their `#` shows while the caret is there), table rows, rules and fence lines. Typing `- ` shows the bullet immediately and Enter continues it (same for `1. ` and `- [ ] `). Caret and deletion follow what is visible: Left/Right step over hidden markers, Up/Down keep the *visual* column, Backspace/Delete remove the visible character (never an invisible `*`), and Backspace at the start of a list item peels it in steps (un-nest → task becomes bullet → marker removed). `row_is_revealed` is the single rule used by rendering, link clicks and keys. Tests: bullets/tasks typed live, visible-editing keys, selection reveal, heading exception.
- [x] **Images, Obsidian style.** Image-only lines hold any number of `![[file.png]]` / `![[file.png|300]]` / `![[file.png|300x200]]` embeds and `![alt](path)` images, side by side (wrapping when a picture fills the column). `![[name]]` finds the file by name anywhere in the vault like Obsidian (note folder → vault path → closest same-named file, case-insensitive, dot-folders skipped, files added later are found on the next lookup); `![[a note]]` is not an image. Images are always rendered — also under the caret (accent ring on the row) — and the row behaves as one atom: Left/Right jump over it, Up/Down land after it, Backspace removes the last image; select the line (triple-click) to edit the source. Pasting an image now writes an Obsidian-style `![[Pasted image YYYYMMDDHHMMSS.png]]` (UTC stamp) into `attachments/`. Click any picture → lightbox (unchanged). Tests: parser (md + wiki mixes), name resolver, side-by-side layout, atom caret/Backspace, paste, stamp helper.

## Section 11 — UX pass (2026-10-19)

- [x] **Zoom** — `Ctrl +` / `Ctrl -` zoom the *document* (text, headings, column, caret; chrome keeps its size) 70–200% in 10% steps, `Ctrl+Alt+0` resets; saved in settings (older settings files still load), shown and click-to-reset in the status bar, palette commands included.
- [x] **Narrow windows** — below ~820px the sidebar folds away and returns when the window widens, unless the user toggled it themselves meanwhile.
- [x] **Empty note hint** — a new/empty note shows "Start writing, or type / for blocks".
- [x] **Keyboard shortcuts reference** — palette command "Keyboard shortcuts" lists every default binding, searchable (palette lists now scroll).
- [x] **Drag and drop** — drop a folder to open it as the mekuto; a note to open it (copied into the mekuto first when it lives elsewhere); an image onto an open note to copy it into `attachments/` and embed it.
- Safety: `Settings::save` is a no-op under `cfg!(test)` so test runs can never overwrite a user's real settings.
- Still open from the audit: accessible names/focus stops for chrome controls, arrow-key navigation in the file tree, background vault search, caret-only repaint on blink.


## Section 12 — Settings (2026-10-19)

- [x] **Layered config** (`config.rs`, GPUI-free): one registry (`DEFS`, ~30 settings in Appearance / Layout / Editor / Markdown / Files) drives defaults, validation and the page. Layers: defaults ← global `~/.config/meku/settings.json` ← mekuto `<mekuto>/.meku/settings.json`. Old global files (`theme`, `follow_system`, `reduce_motion`, `zoom`) still load.
- [x] **Wrong JSON is handled**: a file that does not parse is reported with line/column and a hint (trailing comma, single quotes, comments…), the rest of the app keeps running on defaults, and the file is never overwritten until "Back up and start fresh" (`settings.json.bak-<time>`). Bad *values* (wrong type, unknown choice, out of range) are reported per setting, clamped/ignored, and the other keys still apply. Writes are atomic and keep unknown keys and order.
- [x] **Settings page** — `Ctrl+,`, the gear in the sidebar footer (below Outline), or the palette ("Open settings"). Section nav, search, Global / This mekuto scope switch, modified dot + reset per setting, banners for broken files, "Open settings.json".
- [x] **Themes** (first in Appearance; no theme buttons in the main UI) — one Theme dropdown, "Follow system" and, only while following, Light and Dark dropdowns. Custom themes from a `themes` list in either settings file (`name`, `base`, `dark`, any of `bg surface border fg muted accent keyword string number func ty constant prop comment code` as `#rrggbb`); "Add theme" appends a starter entry. A mekuto theme overrides a global one of the same name.
- [x] **Wired to features**: zoom, fonts, column width, motion, sidebar/outline/tab bar/status bar visibility, fold breakpoint, default view, markdown-reveal rule, auto-close pairs, smart lists, caret blink, image max height / click-to-zoom, table/callout/image rendering, autosave (+delay), attachments folder, delete confirmation, tab restore. Edits to the JSON by hand are picked up live (mtime poll).
- Also wired: slash menu on/off, link click (plain / Ctrl), folder for new notes.

## Section 13 — Critique fixes (2026-10-19)

- [x] Callout types get distinct hues (Question teal, Bug magenta from the theme's syntax colours).
- [x] Mode switch is labelled Edit / Live / Read; footer terminal icon and "Meku" label removed; title-bar search icon only shows while the sidebar is hidden; empty Outline section hidden.
- [x] Settings: scope sentence, subheads in Editor/Files, keyboard tab stops + focus styles on settings and chrome controls.
- [x] Muted text on surface ≥4.5:1 in every preset (Solarized tweaked).
- [x] Title bar: the mode switch became a three-dot menu (Edit / Live / Read with check, Find, Replace, Go to line, Shortcuts, Settings).
- [x] Themes: `Ctrl+Alt+Z` / "Theme: undo last change" (a second press redoes); "Theme: copy as JSON" puts the active theme on the clipboard as a `themes` entry; "Theme: add from clipboard" validates it, adds it to the global themes and applies it.
- [x] **Wheel scrolling everywhere** (`smooth.rs`): precise, not floaty. Half of each notch lands instantly, the rest settles in ~100ms (30ms time constant), whole pixels only, direction reversal cancels the old tail. Document, tree, outline, tabs, settings, palette, lightbox; touchpads untouched. Tune `TAU_SECS` / `NOTCH_LINES` / `INSTANT`.
- [x] **List markers convert on completion**: `-`, `+`, `1.` stay literal text until the space is typed (then become a bullet / number); the checkbox appears once `[ ] ` is complete. `*` is never a bullet (emphasis only), in both the live view and Enter/Tab list handling.

### 14. Session and tabs

- [x] The app reopens the last mekuto at launch (`state.json` beside the global settings; setting **Reopen last mekuto**, default on). A command-line folder still wins; a deleted folder is skipped.
- [x] Tabs restore from `.meku/session.json` (setting **Reopen last tabs**) as placeholders: only the active note is read at launch, others load when first activated; a note that vanished loses its tab.
- [x] Back/forward moved from the title bar to the head of the tab strip.
- [x] Tabs shrink to fit the strip (floor 40px, then it scrolls); the active tab keeps its width, inactive ones drop the icon and close button when crowded (middle-click or the context menu still close them), with a full-title tooltip.
- [x] Scroll bars (kit `Scrollbar`) on the document, file tree, outline and settings page. Setting **Scroll bars**: always (default) / hover / auto-hide. Draggable thumb; wheel scrolling still goes through the smooth layer.
- [x] Click anywhere in the editor focuses it: below the text (or in the room under the last row) puts the caret at the end of the last line of text; margins beside a row land on that row.
- [x] Packaging: `tools/package.sh` (Linux deb, AppImage, Arch, tar.gz; built on ubuntu-22.04 for glibc 2.35+, real dependency lists, AppImage bundles its small libs) and `tools/package-macos.sh` (macOS arm64 .app/.dmg/.zip), built by `.github/workflows/release.yml` (macOS arm64 + Windows x64). Shortcuts use `secondary-` (Ctrl, or Cmd on macOS); the settings file follows each OS convention.
- [x] Image clipboard: paste screenshots/picture data, paste picture files from a file manager (paths on macOS/Windows, `file://` links or paths as text on Linux), copy a picture out (Ctrl+C on an image line, or the Copy button in the zoom view). Wayland has no native image copy in gpui, so it uses `xclip`/`wl-copy` there. Pasted/dropped pictures follow the **Attachments folder** setting.
- [x] Update check: once a day (setting **Check for updates**, default on; off = never online) asks the GitHub releases API via the system `curl`, shows an "Update x.y.z available" chip in the status bar and a dialog with steps/commands for how Meku was installed (deb, Arch, AppImage, tar.gz, macOS app, Windows, source). Command palette: **Check for updates**. Skip-this-version is remembered in `state.json`.
- [x] Right-click menu in the editor (`context_menu.rs`): selection → Cut/Copy/Paste, inline formats, Paragraph styles, Find in note; caret → Undo/Redo, Paste, Select all, Format/Paragraph/Insert submenus; link → Open, Copy address, Remove link; picture → Open, Copy image, Copy path/address, Show in file manager, Size (240/480/720/original), Remove from note (undoable; file stays); code block → Copy code. Shortcut hints come from the real actions; the menu pops in with a small bounce (not with reduced motion).
