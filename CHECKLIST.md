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
- [ ] Freeze keymap defaults (Linux): `Ctrl-P` finder · `Ctrl-Shift-P` palette · `Ctrl-B` explorer · `Ctrl-,` settings · `Ctrl-N/W/T` note/close/tab · `Ctrl-S` save · `Ctrl-F` find
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

- [ ] Create workspace `Cargo.toml` with exact `gpui-pre` pins + `Cargo.lock` committed
- [ ] Enforce direction: `markdown` knows no FS/GPUI · `buffer/vault/index` know no views · `editor` never writes FS (via `workspace.request_save`) · `theme` is a leaf
- [ ] Add `meku-ui` facade over `gpui-component` (so kit swap doesn't ripple) + feature-audit (import only used modules)

### 2B. Live-preview editor LLD

Pipeline: `keystroke → OpenBuffer.apply_edit (<1ms) → debounce 80–120ms Task → background_spawn parse(snapshot,version) → drop if stale → StyledDoc → Entity update → Render (cached doc, rebuild active line only)`.

- [ ] Define `meku-markdown` types: `SourceSpan{start,end}` · `BlockKind{Para,Heading(u8),CodeFence(Option<String>),List,Quote,Hr,Table}` · `StyledSpan{start,end,style}` · `StyledBlock{kind,span,spans}` · `StyledDoc{version,blocks,headings}` · `parse_incremental()` + `parse_fallback()` (single-Para, never blank)
- [ ] Define `meku-editor` view: `LivePreviewView{buffer: Entity<BufferModel>, doc, pending: Task}` · `schedule_reparse()` · `on_parse_done()` (stale-version drop) · `active_byte_range()` (cursor line → `\n` boundaries) · `offset_to_visual()`/`visual_to_offset()` (binary search, round-trip invariant)
- [ ] Render rule: every block styled **except** cursor line → raw monospace with dimmed markers; selection resolved in source-offset space, painted per-block; `catch_unwind` → keep old doc + `preview stale` toast
- [ ] Targets: raw-line keystroke→paint <16ms · styled settle ≤150ms

### 2C. Workspace / mekuto LLD

- [ ] Define `meku-vault`: `Mekuto{root}` · `VaultFile{rel,abs,mtime}` · `VaultEvent{Created,Modified,Removed,SessionChanged}` · `open_root/scan_md/start_watcher/session_path`
- [ ] Define `meku-index`: `FileIndex{rel→FileMeta}` · `FileMeta{title,headings,mtime}` · `rebuild/upsert/remove/fuzzy_match`
- [ ] Define `meku-buffer`: `OpenBuffer{rel,text:Rope,version,dirty,disk_mtime}` · `load/apply_edit/mark_saved/snapshot`
- [ ] Define `meku-workspace`: `Workspace{vault,index,tabs,dirty}` · `TabState{rel,buffer_id,cursor,scroll_px}` · `Session{root,open_tabs,active}` · `open_folder/open_file/on_watcher_event/request_save(atomic tmp+rename)/poll_autosave 800ms/resolve_external_change{KeepMine,LoadDisk}`
- [ ] Tree rule: `*.md` sorted case-insensitive, ignore `.meku/`+dotfiles, incremental `VaultEvent` updates (no full rescan); watcher: `notify` inotify + 50ms coalesce → `cx.spawn` channel

### 2D. Theme / keymap / shell LLD

- [ ] Extend `gpui-component` `Theme` (don't fork): `MekuTokens{editor_bg,line_active,caret,md_h1…}` · `apply_zed_like_theme()` (radius 8px, Zed dark `#171717/#1e1e2e`, accent `#89b4fa`, border white/10) · `font_stack(["Zed Plex Mono","JetBrains Mono",monospace])`
- [ ] Layout: `DockArea` + `DockLayout::h_split` (sidebar 240px | center tabs | outline 220px stub) · `Sidebar/TabBar/Tree/StatusBar/CommandPalette(searchable_list)`
- [ ] Actions: `MekuAction{OpenFolder,OpenPalette,Save,CloseTab,NextTab,ToggleSidebar}` via `cx.bind_keys` + `on_action`; palette = actions + `FileIndex::fuzzy_match`; Linux `Ctrl` bindings verified on Mesa+NVIDIA

### 2E. Testing seams (per module, no GUI unless noted)

- [ ] `meku-markdown`: golden `md→StyledDoc` snapshots + fuzz unclosed fences/tables + offset round-trip assert
- [ ] `meku-buffer`: version bumps, snapshot isolation, tempdir round-trip
- [ ] `meku-vault`: tempdir `scan_md` ignores `.meku/`, watcher delivers <500ms, corrupt session → empty no-panic
- [ ] `meku-index`: upsert/remove headings, fuzzy ranking
- [ ] `meku-editor`: active-line calc, stale-drop, one headless GPUI type→styled test
- [ ] `meku-workspace`: fake watcher → dirty dot / external prompt / tmp+rename autosave / session round-trip
- [ ] `meku-theme`: token hex snapshot, keymap contains `Ctrl+O/P/S/W`, manual contrast check

---

## Section 3 — Build (implementation order, commit after each)

- [x] **0. Scaffold + hello-window** — workspace + `crates/meku` on latest `gpui-kit 0.7.1` (pins `gpui-pre 0.3.8`), `gpui_kit::init` + `Root` pattern, `cargo check -p meku` green (2026-10-08). → _ask user to commit_
- [x] **1. App shell (Zed theme)** — TitleBar + Sidebar (Explorer/Search/Outline) + TabBar (new/select/close) + StatusBar, `h_resizable` 240px shell, keymap `Ctrl-B/T/W/Tab` under `Meku` context, empty states. `check`+`fmt`+`clippy` green (2026-10-08). → _ask user to commit_
- [ ] **2. Mekuto open + file tree** — open folder, tree ops (new/rename/delete/folder), watcher refresh, session restore. → _commit_
- [ ] **3. Editor buffer + tabs** — multi-tab, dirty dot, undo/redo, autosave + reload prompt, 10k-line <16ms. → _commit_
- [ ] **4. Live preview (inline GFM)** — background parse → styled spans, active-line raw, cursor never jumps, GFM round-trip fixture passes. → _commit_
- [ ] **5. Preview mode + highlight** — Edit/Live/Read toggle, tree-sitter 5 langs, images, scroll-sync. → _commit_
- [ ] **6. Search + palette + outline** — `Ctrl-P` finder, `Ctrl-Shift-P` palette, full-text (grep lib), heading tree + follow-cursor. → _commit_
- [ ] **7. Perf + polish** — virtualized lists everywhere, 120ms debounce audit, lazy images/grammars, benches green, RSS <120MB, docs + Linux packaging. → _commit_

_Deferred past v0.1: links/backlinks/graph, vim, plugins/sync, export PDF/HTML, minimap, >100k-line mmap paging, tantivy, settings UI beyond ~10 keys._
