# Meku — Agent Instructions

GPU-accelerated native markdown editor (Rust, Linux-first). Obsidian-like vault UX, Zed-like styling. Single binary crate today; multi-crate workspace planned (see `CHECKLIST.md`).

## Dependencies — strict rules

- Depend **only** on the `gpui-kit` umbrella (`gpui-kit = "0.7.1"`, workspace dep). It pins the matching `gpui-pre` set (currently 0.3.8). **Never add `gpui`/`gpui-pre`/`gpui-component` directly** — mismatched GPUI types break the build.
- User rule: use the **latest** crate version unless it causes issues; verify with `cargo search <crate>` before bumping.
- User rule: **check Context7 docs before** any library work (`resolve-library-id` → `query-docs`). Library IDs: `/longbridge/gpui-kit`, `/websites/rs_gpui-pre`.

## App entry pattern (mandatory)

Every window must follow this shape (`crates/meku/src/main.rs`):

```rust
gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| {
    gpui_kit::init(cx); // FIRST, before any component use
    cx.spawn(async move |cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(|_| MyView);
            cx.new(|cx| Root::new(view, window, cx)) // Root MUST be first level
        })
    }).detach();
});
```

## Commands

- `cargo check -p meku --message-format short` — primary verification. First build takes ~2.5 min; subsequent checks are fast. `cargo run` needs a graphical Wayland/X11 session + Vulkan driver (`vulkaninfo` must pass).
- `cargo check` compiles the whole workspace; `-p meku` is enough until more crates exist.
- Dev profile forces `opt-level = 3` for `gpui-pre`/`gpui-kit` (root `Cargo.toml`) — plain debug GPUI is unusably slow. Don't remove.

## Planned architecture (don't violate)

`CHECKLIST.md` Section 2 defines the target crates and **acyclic** deps: `markdown` knows no FS/GPUI · `buffer/vault/index` know no views · `editor` never writes FS (via `workspace.request_save`) · `theme` is a leaf behind a `meku-ui` facade. New code must respect this direction.

`mekuto` = plain folder of `.md` + `.meku/session.json` sidecar. No wikilinks/backlinks/graph in v0.1 — render `[[x]]` literally.

## UI tests (headless, `gpui-kit/test-support` dev-dep)

- Run: `cargo test -p meku`. Tests live in `crates/meku/src/main.rs::ui_tests` (binary crate, so no `tests/` dir yet).
- **Never use `#[gpui_kit::test]` / `#[test]` under a `use gpui_kit::*` glob**: the glob imports GPUI's `test` proc-macro, which shadows the builtin and sends rustc into macro-expansion stack overflow. Import test-module items explicitly (`use gpui_kit::test::TestWindowExt;` etc., never `super::*` where super has the globs).
- Use the local `run_ui_test(name, fn_ptr)` harness (plain `#[test]` + manual `TestAppContext::build`/`run_until_parked`/`quit` teardown). Takes a `fn` pointer, not a closure (unwind-safety across the harness boundary).
- Pattern: `cx.update(gpui_kit::init)` → bind keys → `cx.open_window(size, |window, cx| Root::new(view, window, cx))` → `cx.update_window(handle.into(), |_, window, cx| { window.render_frame(cx); window.click/find/press/input(...) })`. Give interactive elements `.id("...")` for `find`/`click`.

## Workflow (user-imposed, follow exactly)

- `CHECKLIST.md` is the living tracker: check boxes as work lands.
- After **every** subsection, stop and ask the user to commit/push upstream. **Never `git commit`, amend, or push yourself.**
- Perf budgets are hard constraints: cold start <300ms · 10k-line open <16ms main-thread · typing <16ms p95 · idle RSS <120MB. Main thread does visible-only work; parse/index/decode go to background tasks.
