//! The right-click menu of the document surface.
//!
//! What it offers depends on what was pressed: a selection gets cut/copy and
//! the inline formats, a link gets open/copy/unlink, a picture gets zoom,
//! copy, reveal, resize and remove, a code block gets "copy code", and plain
//! text gets paragraph styles and block inserts. Everything routes through
//! the same commands the keyboard uses, so the menu and the shortcuts can
//! never disagree.
//!
//! The menu is a kit [`PopupMenu`] hosted here (not through the kit's
//! context-menu trigger) so the target is decided at the moment of the press
//! and the menu can pop in with a small bounce.

use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use meku_markdown::live::{LinkTarget, RowKind, classify_link, link_at, row_at};

use crate::document::{DocKey, ImageInfo, ImageSrc};
use crate::{
    DocBold, DocCopy, DocCut, DocH0, DocH1, DocH2, DocH3, DocH4, DocH5, DocH6, DocHighlight,
    DocInlineCode, DocItalic, DocLink, DocListBullet, DocListOrdered, DocListTask, DocPaste,
    DocQuote, DocRedo, DocSelectAll, DocStrike, DocUndo, MekuShell, Mode,
};

/// An open menu: the kit menu, where it opened and the subscription that
/// closes it. `serial` restarts the pop-in animation for every opening.
pub(crate) struct DocMenu {
    pub(crate) menu: Entity<PopupMenu>,
    pos: Point<Pixels>,
    serial: usize,
    _dismiss: Subscription,
}

/// What a right-click landed on.
pub(crate) enum Target {
    /// Text of row `ix` (or the empty space around the text when `None`).
    Text(Option<usize>),
    /// A picture of the open note.
    Image(ImageInfo),
}

/// Everything the menu builder needs, decided once at press time.
#[derive(Clone)]
struct Ctx {
    read_only: bool,
    selection: bool,
    link: Option<LinkHit>,
    code: Option<String>,
}

#[derive(Clone)]
struct LinkHit {
    /// Absolute source range of `[text](dest)`.
    range: Range<usize>,
    /// The destination as written.
    dest: String,
    /// The link text, for unlinking.
    text: Option<String>,
    /// Where it leads, when Meku can follow it.
    target: Option<LinkTarget>,
}

/// Item whose click runs `f` on the shell with the document focused.
fn run(
    shell: &Entity<MekuShell>,
    label: &'static str,
    disabled: bool,
    f: impl Fn(&mut MekuShell, &mut Window, &mut Context<MekuShell>) + 'static,
) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new(label)
        .disabled(disabled)
        .on_click(move |_, window, cx| {
            shell.update(cx, |view, cx| {
                let focus = view.focus_handle.clone();
                window.focus(&focus, cx);
                f(view, window, cx);
            });
        })
}

/// An editing command, labelled with its shortcut through `action`.
fn cmd(
    shell: &Entity<MekuShell>,
    label: &'static str,
    action: impl Action,
    key: DocKey,
    disabled: bool,
) -> PopupMenuItem {
    run(shell, label, disabled, move |view, window, cx| {
        view.doc_key(key, false, window, cx)
    })
    .action(Box::new(action))
}

fn format_items(menu: PopupMenu, s: &Entity<MekuShell>, ro: bool) -> PopupMenu {
    menu.item(cmd(s, "Bold", DocBold, DocKey::Wrap("**"), ro))
        .item(cmd(s, "Italic", DocItalic, DocKey::Wrap("*"), ro))
        .item(cmd(s, "Strikethrough", DocStrike, DocKey::Wrap("~~"), ro))
        .item(cmd(s, "Highlight", DocHighlight, DocKey::Wrap("=="), ro))
        .item(cmd(s, "Inline code", DocInlineCode, DocKey::Wrap("`"), ro))
        .item(cmd(s, "Link", DocLink, DocKey::Link, ro))
}

fn paragraph_items(menu: PopupMenu, s: &Entity<MekuShell>, ro: bool) -> PopupMenu {
    menu.item(cmd(s, "Paragraph", DocH0, DocKey::Heading(0), ro))
        .item(cmd(s, "Heading 1", DocH1, DocKey::Heading(1), ro))
        .item(cmd(s, "Heading 2", DocH2, DocKey::Heading(2), ro))
        .item(cmd(s, "Heading 3", DocH3, DocKey::Heading(3), ro))
        .item(cmd(s, "Heading 4", DocH4, DocKey::Heading(4), ro))
        .item(cmd(s, "Heading 5", DocH5, DocKey::Heading(5), ro))
        .item(cmd(s, "Heading 6", DocH6, DocKey::Heading(6), ro))
        .separator()
        .item(cmd(s, "Quote", DocQuote, DocKey::Quote, ro))
        .item(cmd(
            s,
            "Bulleted list",
            DocListBullet,
            DocKey::ListBullet,
            ro,
        ))
        .item(cmd(
            s,
            "Numbered list",
            DocListOrdered,
            DocKey::ListOrdered,
            ro,
        ))
        .item(cmd(s, "Task list", DocListTask, DocKey::ListTask, ro))
}

fn insert_items(menu: PopupMenu, s: &Entity<MekuShell>, ro: bool) -> PopupMenu {
    ["Table", "Code block", "Callout", "Divider", "Image", "Link"]
        .into_iter()
        .fold(menu, |menu, label| {
            menu.item(run(s, label, ro, move |view, _, cx| {
                view.insert_block(label, cx)
            }))
        })
}

fn copy_text(text: String) -> impl Fn(&mut MekuShell, &mut Window, &mut Context<MekuShell>) {
    move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
}

fn text_menu(
    base: PopupMenu,
    s: &Entity<MekuShell>,
    ctx: &Ctx,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let ro = ctx.read_only;
    let mut menu = base.action_context(focus.clone()).min_w(px(224.));

    if let Some(link) = &ctx.link {
        let open = link.target.clone();
        menu = menu
            .item(run(s, "Open link", open.is_none(), move |view, _, cx| {
                if let Some(target) = open.clone() {
                    view.follow_link(target, cx);
                }
            }))
            .item(run(
                s,
                "Copy link address",
                false,
                copy_text(link.dest.clone()),
            ));
        if let (Some(text), false) = (link.text.clone(), ro) {
            let range = link.range.clone();
            menu = menu.item(run(s, "Remove link", false, move |view, _, cx| {
                view.replace_source(range.clone(), &text, cx)
            }));
        }
        menu = menu.separator();
    }
    if let Some(code) = &ctx.code {
        menu = menu
            .item(run(s, "Copy code", false, copy_text(code.clone())))
            .separator();
    }

    if ctx.selection {
        menu = menu
            .item(cmd(s, "Cut", DocCut, DocKey::Cut, ro))
            .item(cmd(s, "Copy", DocCopy, DocKey::Copy, false))
            .item(cmd(s, "Paste", DocPaste, DocKey::Paste, ro))
            .separator();
        menu = format_items(menu, s, ro).separator();
        {
            let s = s.clone();
            let focus = focus.clone();
            menu = menu.submenu("Paragraph", window, cx, move |m, _, _| {
                paragraph_items(m.action_context(focus.clone()), &s, ro)
            });
        }
        menu = menu
            .item(run(s, "Find in note", false, |view, _, cx| {
                view.open_find(false, cx)
            }))
            .separator()
            .item(cmd(s, "Select all", DocSelectAll, DocKey::SelectAll, false));
    } else {
        menu = menu
            .item(cmd(s, "Undo", DocUndo, DocKey::Undo, ro))
            .item(cmd(s, "Redo", DocRedo, DocKey::Redo, ro))
            .separator()
            .item(cmd(s, "Paste", DocPaste, DocKey::Paste, ro))
            .item(cmd(s, "Select all", DocSelectAll, DocKey::SelectAll, false))
            .separator();
        for (label, build) in [
            (
                "Format",
                format_items as fn(PopupMenu, &Entity<MekuShell>, bool) -> PopupMenu,
            ),
            ("Paragraph", paragraph_items),
            ("Insert", insert_items),
        ] {
            let s = s.clone();
            let focus = focus.clone();
            menu = menu.submenu(label, window, cx, move |m, _, _| {
                build(m.action_context(focus.clone()), &s, ro)
            });
        }
    }
    menu
}

fn image_menu(
    base: PopupMenu,
    s: &Entity<MekuShell>,
    info: &ImageInfo,
    read_only: bool,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let mut menu = base.action_context(focus.clone()).min_w(px(224.));
    let local: Option<PathBuf> = match &info.src {
        ImageSrc::Path(p) if p.is_file() => Some(p.clone()),
        _ => None,
    };
    let open = info.clone();
    menu = menu.item(run(s, "Open image", false, move |view, _, cx| {
        view.open_lightbox(open.clone(), cx)
    }));
    let copy_path = local.clone();
    menu = menu.item(run(s, "Copy image", local.is_none(), move |view, _, cx| {
        if let Some(path) = &copy_path {
            view.copy_image_file(path, cx);
        }
    }));
    let (where_label, where_text) = match &info.src {
        ImageSrc::Path(p) => ("Copy file path", p.display().to_string()),
        ImageSrc::Url(u) => ("Copy image address", u.clone()),
    };
    menu = menu.item(run(s, where_label, false, copy_text(where_text)));
    let reveal = local.clone();
    menu = menu.item(run(
        s,
        "Show in file manager",
        local.is_none(),
        move |_, _, cx| {
            if let Some(path) = &reveal {
                cx.reveal_path(path);
            }
        },
    ));
    if let ImageSrc::Url(url) = &info.src {
        let url = url.clone();
        menu = menu.item(run(s, "Open in browser", false, move |_, _, cx| {
            cx.open_url(&url)
        }));
    }
    if !read_only {
        menu = menu.separator();
        let (s2, focus2, range) = (s.clone(), focus.clone(), info.range.clone());
        menu = menu.submenu("Size", window, cx, move |m, _, _| {
            let mut m = m.action_context(focus2.clone());
            for (label, width) in [
                ("Small", Some(240)),
                ("Medium", Some(480)),
                ("Large", Some(720)),
                ("Original", None),
            ] {
                let range = range.clone();
                m = m.item(run(&s2, label, false, move |view, _, cx| {
                    view.resize_image(range.clone(), width, cx)
                }));
            }
            m
        });
        let remove = info.range.clone();
        menu = menu
            .separator()
            .item(run(s, "Remove from note", false, move |view, _, cx| {
                view.remove_image(remove.clone(), cx)
            }));
    }
    menu
}

/// `![alt](url)` / `![[name]]` with its `|width` set (or cleared).
fn with_width(token: &str, width: Option<u32>) -> Option<String> {
    let suffix = width.map(|w| format!("|{w}")).unwrap_or_default();
    let strip = |inner: &str| -> String {
        match inner.rsplit_once('|') {
            Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => {
                head.to_string()
            }
            _ => inner.to_string(),
        }
    };
    if let Some(inner) = token
        .strip_prefix("![[")
        .and_then(|rest| rest.strip_suffix("]]"))
    {
        return Some(format!("![[{}{suffix}]]", strip(inner)));
    }
    let rest = token.strip_prefix("![")?;
    let close = rest.find("](")?;
    Some(format!(
        "![{}{suffix}]{}",
        strip(&rest[..close]),
        &rest[close + 1..]
    ))
}

/// `[text](dest)` reduced to `text`.
fn link_text(whole: &str) -> Option<String> {
    let inner = whole.strip_prefix('[')?;
    let close = inner.rfind("](")?;
    Some(inner[..close].to_string())
}

/// Source range to delete so the image on row `row` vanishes with its line,
/// without leaving a double blank line behind.
fn image_removal(text: &str, row: Range<usize>, token: Range<usize>) -> Range<usize> {
    let line = &text[row.clone()];
    let rest = format!(
        "{}{}",
        &text[row.start..token.start],
        &text[token.end..row.end]
    );
    if !rest.trim().is_empty() || !line.contains("![") {
        return token;
    }
    let mut start = row.start;
    let mut end = row.end;
    if text[end..].starts_with('\n') {
        end += 1;
    } else {
        start = start.saturating_sub(1);
    }
    if text[..start].ends_with("\n\n") && text[end..].starts_with('\n') {
        end += 1;
    }
    start..end
}

/// Text of the fenced block around row `ix` (without the fences).
fn code_block_text(rows: &[meku_markdown::live::Row], text: &str, ix: usize) -> Option<String> {
    if rows.get(ix)?.kind != RowKind::Code {
        return None;
    }
    let (mut a, mut b) = (ix, ix);
    while a > 0 && rows[a - 1].kind == RowKind::Code {
        a -= 1;
    }
    while b + 1 < rows.len() && rows[b + 1].kind == RowKind::Code {
        b += 1;
    }
    let lines: Vec<&str> = rows[a..=b]
        .iter()
        .filter(|r| r.fence.is_none())
        .map(|r| &text[r.src.clone()])
        .collect();
    Some(lines.join("\n"))
}

impl MekuShell {
    /// Open the menu for a right-click at `pos`.
    pub(crate) fn open_doc_menu(
        &mut self,
        target: Target,
        pos: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        let Some(rel) = self.doc_rel() else {
            return;
        };
        let Some(buffer) = self.buffers.get(&rel).cloned() else {
            return;
        };
        self.touch_caret();
        self.flush_title(cx);
        let read_only = self.tab_mode() == Mode::Read;
        let focus = self.focus_handle.clone();
        window.focus(&focus, cx);

        let mut ctx = Ctx {
            read_only,
            selection: false,
            link: None,
            code: None,
        };
        if let Target::Text(ix) = &target {
            if let Some(ix) = *ix
                && let Some(src) = self.row_source(ix, pos)
            {
                let sel = buffer.read(cx).selection();
                // A press inside the selection keeps it (to cut, copy, format);
                // anywhere else moves the caret there, like any editor.
                if sel.is_empty() || src < sel.start || src > sel.end {
                    buffer.update(cx, |b, _| b.move_caret(src, false));
                }
                if let Some(model) = self.doc_model(&rel, cx)
                    && let Some(row) = model.rows.get(ix)
                {
                    let over_text = self
                        .doc_views
                        .borrow()
                        .get(ix)
                        .and_then(|v| v.as_ref())
                        .is_none_or(|v| v.layout.bounds().contains(&pos));
                    if row.kind != RowKind::Code && over_text {
                        let line = &model.text[row.src.clone()];
                        ctx.link = link_at(line, src.saturating_sub(row.src.start)).map(|l| {
                            let dest = l.target;
                            let whole = &line[l.range.clone()];
                            LinkHit {
                                range: row.src.start + l.range.start..row.src.start + l.range.end,
                                dest: dest.clone(),
                                text: link_text(whole),
                                target: classify_link(&dest, &rel),
                            }
                        });
                    }
                    ctx.code = code_block_text(&model.rows, &model.text, ix);
                }
            }
            ctx.selection = !buffer.read(cx).selection().is_empty();
        }

        let shell = cx.entity();
        let menu = PopupMenu::build(window, cx, move |base, window, cx| match &target {
            Target::Text(_) => text_menu(base, &shell, &ctx, &focus, window, cx),
            Target::Image(info) => {
                image_menu(base, &shell, info, ctx.read_only, &focus, window, cx)
            }
        });
        let handle = menu.focus_handle(cx);
        window.focus(&handle, cx);
        let dismiss = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.doc_menu = None;
            let focus = this.focus_handle.clone();
            window.focus(&focus, cx);
            cx.notify();
        });
        self.doc_menu_serial += 1;
        self.doc_menu = Some(DocMenu {
            menu,
            pos,
            serial: self.doc_menu_serial,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    /// The open menu, popping in with a small bounce.
    pub(crate) fn render_context_menu(&self, cx: &App) -> Option<AnyElement> {
        let open = self.doc_menu.as_ref()?;
        let body = div().child(open.menu.clone());
        let body = if crate::theme::motion(cx) {
            body.with_animation(
                ("doc-menu-in", open.serial),
                Animation::new(Duration::from_millis(200)).with_easing(bounce_out),
                |el, t| el.opacity((t * 2.5).min(1.0)).mt(px(12.0 * (1.0 - t))),
            )
            .into_any_element()
        } else {
            body.into_any_element()
        };
        Some(
            deferred(
                anchored()
                    .position(open.pos)
                    .snap_to_window_with_margin(px(8.))
                    .anchor(Anchor::TopLeft)
                    .child(body),
            )
            .with_priority(100)
            .into_any_element(),
        )
    }

    /// Insert the slash-menu block called `label` at the caret, on its own line.
    pub(crate) fn insert_block(&mut self, label: &str, cx: &mut Context<Self>) {
        let Some((_, _, snippet, back)) = crate::slash::ITEMS.iter().find(|i| i.0 == label) else {
            return;
        };
        let Some(rel) = self.doc_rel() else {
            return;
        };
        if let Some(buffer) = self.buffers.get(&rel).cloned() {
            buffer.update(cx, |b, _| {
                let text = if b.caret_line_text().trim().is_empty() {
                    snippet.to_string()
                } else {
                    format!("\n{snippet}")
                };
                b.insert(&text);
                let caret = b.caret().saturating_sub(*back);
                b.move_caret(caret, false);
            });
            self.doc_note_edit(&rel);
        }
        cx.notify();
    }

    /// Replace `range` of the open note with `with` as one undoable edit.
    pub(crate) fn replace_source(
        &mut self,
        range: Range<usize>,
        with: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(rel) = self.doc_rel() else {
            return;
        };
        if let Some(buffer) = self.buffers.get(&rel).cloned() {
            buffer.update(cx, |b, _| b.replace(range, with, false));
            self.doc_note_edit(&rel);
        }
        cx.notify();
    }

    /// Set or clear the display width of the image token at `range`.
    pub(crate) fn resize_image(
        &mut self,
        range: Range<usize>,
        width: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        let Some(rel) = self.doc_rel() else {
            return;
        };
        let Some(buffer) = self.buffers.get(&rel).cloned() else {
            return;
        };
        let text = buffer.read(cx).text_string();
        let Some(token) = text.get(range.clone()) else {
            return;
        };
        if let Some(next) = with_width(token, width) {
            self.replace_source(range, &next, cx);
        }
    }

    /// Take the image at `range` out of the note (the file stays on disk).
    pub(crate) fn remove_image(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let Some(rel) = self.doc_rel() else {
            return;
        };
        let Some(buffer) = self.buffers.get(&rel).cloned() else {
            return;
        };
        let text = buffer.read(cx).text_string();
        if !text.get(range.clone()).is_some_and(|t| t.starts_with("![")) {
            return;
        }
        let Some(model) = self.doc_model(&rel, cx) else {
            return;
        };
        let Some(row) = model.rows.get(row_at(&model.rows, range.start)) else {
            return;
        };
        let cut = image_removal(&text, row.src.clone(), range);
        self.replace_source(cut, "", cx);
        self.notice = Some("Image removed from the note. Undo brings it back.".to_string());
        cx.notify();
    }
}

/// Ease-out with a small overshoot: the menu rises a touch past its place and
/// settles.
fn bounce_out(t: f32) -> f32 {
    const C1: f32 = 2.0;
    let u = t - 1.0;
    1.0 + (C1 + 1.0) * u * u * u + C1 * u * u
}

#[cfg(test)]
mod tests {
    use super::{bounce_out, image_removal, link_text, with_width};

    #[test]
    fn width_is_set_replaced_and_cleared() {
        assert_eq!(
            with_width("![a](x.png)", Some(240)).unwrap(),
            "![a|240](x.png)"
        );
        assert_eq!(
            with_width("![a|300](x.png)", Some(480)).unwrap(),
            "![a|480](x.png)"
        );
        assert_eq!(with_width("![a|300](x.png)", None).unwrap(), "![a](x.png)");
        assert_eq!(
            with_width("![](x.png)", Some(240)).unwrap(),
            "![|240](x.png)"
        );
        assert_eq!(
            with_width("![[pic.png]]", Some(240)).unwrap(),
            "![[pic.png|240]]"
        );
        assert_eq!(with_width("![[pic.png|10]]", None).unwrap(), "![[pic.png]]");
        assert_eq!(with_width("not an image", Some(1)), None);
    }

    #[test]
    fn unlink_keeps_the_text() {
        assert_eq!(link_text("[hello](https://a.b)").as_deref(), Some("hello"));
        assert_eq!(link_text("<https://a.b>"), None);
    }

    #[test]
    fn removing_an_image_takes_its_line_and_one_blank() {
        let text = "intro\n\n![a](x.png)\n\nafter\n";
        let row = 7..19;
        let cut = image_removal(text, row, 7..19);
        let mut out = text.to_string();
        out.replace_range(cut, "");
        assert_eq!(out, "intro\n\nafter\n");

        // An image sharing its line with text only loses the token.
        let text = "see ![a](x.png) here";
        assert_eq!(image_removal(text, 0..text.len(), 4..15), 4..15);

        // The last line takes the newline before it.
        let text = "intro\n![a](x.png)";
        let cut = image_removal(text, 6..text.len(), 6..text.len());
        let mut out = text.to_string();
        out.replace_range(cut, "");
        assert_eq!(out, "intro");
    }

    #[test]
    fn the_bounce_overshoots_then_lands() {
        assert!(bounce_out(0.0).abs() < 1e-6);
        assert!((bounce_out(1.0) - 1.0).abs() < 1e-6);
        assert!((0..100).any(|i| bounce_out(i as f32 / 100.0) > 1.0));
    }
}
