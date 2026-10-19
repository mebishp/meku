//! The `/` command menu: type `/` at the start of a line (or after a space)
//! and pick a block to insert. Anchored at the caret; Up/Down/Enter/Tab/Esc
//! and the mouse work.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;

use crate::MekuShell;

/// `(label, hint, snippet, bytes back from the snippet end for the caret)`.
pub(crate) const ITEMS: &[(&str, &str, &str, usize)] = &[
    ("Heading 1", "#", "# ", 0),
    ("Heading 2", "##", "## ", 0),
    ("Heading 3", "###", "### ", 0),
    ("Bulleted list", "-", "- ", 0),
    ("Numbered list", "1.", "1. ", 0),
    ("Task list", "[ ]", "- [ ] ", 0),
    ("Quote", ">", "> ", 0),
    ("Callout", "!", "> [!note] Title\n> ", 0),
    ("Warning callout", "!", "> [!warning] Heads up\n> ", 0),
    ("Code block", "```", "```\n\n```", 4),
    (
        "Table",
        "|",
        "| Column | Column |\n| ------ | ------ |\n|        |        |",
        17,
    ),
    ("Divider", "---", "---\n", 0),
    ("Link", "[]()", "[](url)", 6),
    ("Image", "![]()", "![](url)", 6),
    ("Bold", "**", "****", 2),
    ("Inline code", "`", "``", 1),
];

/// Slash menu state: the `/` offset and the highlighted row.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SlashState {
    pub start: usize,
    pub selected: usize,
}

/// What a caret row needs to draw the open menu.
#[derive(Clone)]
pub(crate) struct SlashView {
    pub items: Vec<(&'static str, &'static str)>,
    pub selected: usize,
}

/// Menu entries matching `query`, best match first.
pub(crate) fn matching(query: &str) -> Vec<usize> {
    let mut scored: Vec<(u64, usize)> = ITEMS
        .iter()
        .enumerate()
        .filter_map(|(i, (label, ..))| meku_index::fuzzy_score(query, label).map(|s| (s, i)))
        .collect();
    if !query.is_empty() {
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    }
    scored.into_iter().map(|(_, i)| i).collect()
}

impl MekuShell {
    /// Re-derive the menu from the buffer after any edit or caret move.
    pub(crate) fn update_slash(&mut self, in_code: bool, cx: &App) {
        let trigger = self
            .doc_rel()
            .and_then(|rel| self.buffers.get(&rel))
            .and_then(|b| b.read(cx).slash_trigger())
            .filter(|_| !in_code);
        self.slash = match trigger {
            Some((start, query))
                if self.slash_dismissed != Some(start) && !matching(&query).is_empty() =>
            {
                let selected = self
                    .slash
                    .filter(|s| s.start == start)
                    .map_or(0, |s| s.selected);
                Some(SlashState { start, selected })
            }
            Some((start, _)) => {
                // Keep a dismissal alive while its `/` is still being typed.
                if self.slash_dismissed != Some(start) {
                    self.slash_dismissed = None;
                }
                None
            }
            None => {
                self.slash_dismissed = None;
                None
            }
        };
    }

    fn slash_query(&self, cx: &App) -> Option<String> {
        let rel = self.doc_rel()?;
        self.buffers
            .get(&rel)?
            .read(cx)
            .slash_trigger()
            .map(|(_, q)| q)
    }

    /// The open menu as drawable data (labels, hints, selection).
    pub(crate) fn slash_view(&self, cx: &App) -> Option<SlashView> {
        let state = self.slash?;
        let query = self.slash_query(cx)?;
        let items: Vec<_> = matching(&query)
            .into_iter()
            .map(|i| (ITEMS[i].0, ITEMS[i].1))
            .collect();
        (!items.is_empty()).then(|| SlashView {
            selected: state.selected.min(items.len() - 1),
            items,
        })
    }

    pub(crate) fn slash_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(query) = self.slash_query(cx) else {
            return;
        };
        let n = matching(&query).len() as isize;
        if let Some(state) = self.slash.as_mut()
            && n > 0
        {
            state.selected = (state.selected as isize + delta).rem_euclid(n) as usize;
        }
        cx.notify();
    }

    /// Insert the highlighted (or clicked) entry in place of `/query`.
    pub(crate) fn slash_confirm(&mut self, pick: Option<usize>, cx: &mut Context<Self>) {
        let Some(state) = self.slash else {
            return;
        };
        let Some(query) = self.slash_query(cx) else {
            return;
        };
        let hits = matching(&query);
        let Some(&item) = hits.get(pick.unwrap_or(state.selected)) else {
            return;
        };
        let (_, _, snippet, back) = ITEMS[item];
        let Some(rel) = self.doc_rel() else {
            return;
        };
        if let Some(buffer) = self.buffers.get(&rel).cloned() {
            let end = buffer.read(cx).caret();
            buffer.update(cx, |b, _| b.insert_snippet(state.start..end, snippet, back));
        }
        self.slash = None;
        self.slash_dismissed = None;
        self.doc_note_edit(&rel);
        self.touch_caret();
        cx.notify();
    }

    /// Esc: close the menu and stay closed until that `/` is gone.
    pub(crate) fn slash_dismiss(&mut self) -> bool {
        match self.slash.take() {
            Some(s) => {
                self.slash_dismissed = Some(s.start);
                true
            }
            None => false,
        }
    }
}

/// The floating menu, positioned by the caller relative to its row.
pub(crate) fn menu_element(view: &SlashView, shell: &Entity<MekuShell>, cx: &App) -> AnyElement {
    let rows: Vec<AnyElement> = view
        .items
        .iter()
        .take(8)
        .enumerate()
        .map(|(i, (label, hint))| {
            let shell = shell.clone();
            let selected = i == view.selected;
            let mut active = cx.theme().list_active;
            active.a = 0.9;
            h_flex()
                .id(("slash-item", i))
                .test_support()
                .w_full()
                .px_2()
                .py_1()
                .gap_3()
                .justify_between()
                .items_center()
                .rounded_md()
                .text_sm()
                .when_selected(selected, active)
                .child(div().child(*label))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(*hint),
                )
                .on_click(move |_, _, cx| {
                    shell.update(cx, |view, cx| view.slash_confirm(Some(i), cx));
                })
                .into_any_element()
        })
        .collect();
    v_flex()
        .id("slash-menu")
        .test_support()
        .w(px(280.))
        .p_1()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .shadow_lg()
        // Presses on the menu must not move the caret under it.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .children(rows)
        .into_any_element()
}

/// Highlight helper for the selected menu row.
trait WhenSelected: Sized {
    fn when_selected(self, selected: bool, color: Hsla) -> Self;
}

impl<T: Styled> WhenSelected for T {
    fn when_selected(self, selected: bool, color: Hsla) -> Self {
        if selected { self.bg(color) } else { self }
    }
}
