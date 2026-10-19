# Product

<!-- impeccable:product-schema 1 -->

## Register

product

## Platform

desktop (native GPUI app, Linux first; not web, iOS or Android)

## Users
Writers and developers who keep notes in plain markdown every day. They work in a folder of `.md` files (a "mekuto"), mostly from the keyboard, and want an Obsidian-like vault experience with the fast, quiet feel of Zed.

## Product Purpose
A GPU-accelerated native markdown editor where the document is a single live-preview surface: markup hides away from the caret line and reveals as you move into it. Success is writing without friction: typing never lags, files are never lost, and nothing the user did not ask for is rewritten or renamed.

## Positioning
Native and fast (Rust + GPUI) rather than Electron, over plain files with no proprietary vault format, and no plugin or sync layer in v0.1.

## Operating Context
Long sessions with many notes open as tabs, a file tree on the left, an outline of the current note, keyboard-heavy editing (line moves, smart lists, auto-pairing), and files that other tools may also touch (hence external-change detection). Perf budgets are hard constraints: cold start under 300ms, typing under 16ms p95, idle RSS under 120MB.

## Capabilities and Constraints
- A mekuto is a plain folder of `.md` files plus a `.meku/session.json` sidecar.
- No wikilinks, backlinks, graph, embeds, tags or properties (explicitly out of scope).
- Rendering is one row per source line; the caret row shows raw source.
- Each note has an inline title above the body (not part of the markdown) that mirrors, and renames, the file.
- Lists nest three levels; tasks are real checkboxes; links open on a plain click when rendered.
- Fonts and colours come from the gpui-kit theme; body prefers Inter, code JetBrains Mono. `design-guidelines.md` exists but is advisory, not binding.
- Open decisions: light/dark theming approach, settings UI, column width setting.

## Product Principles
- The file on disk is the source of truth; the UI never silently rewrites, renames or reloads what the user wrote.
- Quiet chrome: the document is the interface; controls appear when needed.
- Keyboard first, mouse friendly: every editing action has a key, and the obvious click does the obvious thing.
- Fast by construction: visible-only work on the main thread, everything else in the background.
- Predictable over clever: smart behaviour (pairing, lists, renames) must be reversible with one undo and never surprising.

## Accessibility & Inclusion
Not specified by the user. Working assumption (inferred, to confirm): WCAG AA contrast for text, full keyboard operability, visible focus, and no information carried by colour alone.
