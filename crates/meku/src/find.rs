//! Find & replace inside the active note: matcher, state and the floating bar.

use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use regex::{Regex, RegexBuilder};

use crate::MekuShell;

actions!(meku_find, [OpenFind, OpenReplace, FindNext, FindPrev]);

/// How the query is matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FindOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

const MAX_MATCHES: usize = 5000;

/// Compile a query under `opts`. Plain queries are escaped; whole-word wraps
/// the pattern in word boundaries. `Err` carries the regex syntax error.
pub(crate) fn compile(query: &str, opts: FindOptions) -> Result<Regex, String> {
    let base = if opts.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    let pattern = if opts.whole_word {
        format!(r"\b(?:{base})\b")
    } else {
        base
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!opts.case_sensitive)
        .multi_line(true)
        .build()
        .map_err(|e| {
            e.to_string()
                .lines()
                .last()
                .unwrap_or("invalid pattern")
                .to_string()
        })
}

/// Byte ranges of every (non-empty) match, capped at [`MAX_MATCHES`].
pub(crate) fn find_matches(text: &str, re: &Regex) -> Vec<Range<usize>> {
    re.find_iter(text)
        .filter(|m| !m.is_empty())
        .take(MAX_MATCHES)
        .map(|m| m.range())
        .collect()
}

/// `(range, replacement)` pairs for replacing the given matches. In regex
/// mode `$1`-style groups expand; otherwise the text is literal.
pub(crate) fn replacements(
    text: &str,
    re: &Regex,
    matches: &[Range<usize>],
    with: &str,
    regex_mode: bool,
) -> Vec<(Range<usize>, String)> {
    matches
        .iter()
        .map(|range| {
            let new = if regex_mode {
                // Re-run on the matched slice so capture groups resolve.
                re.replace(&text[range.clone()], with).into_owned()
            } else {
                with.to_string()
            };
            (range.clone(), new)
        })
        .collect()
}

pub(crate) struct FindState {
    pub query: Entity<InputState>,
    pub replace: Entity<InputState>,
    pub _subs: Vec<Subscription>,
    pub opts: FindOptions,
    pub show_replace: bool,
    pub rel: PathBuf,
    pub matches: Vec<Range<usize>>,
    pub current: Option<usize>,
    /// Where searching started: typing re-picks the first match at or after
    /// this offset instead of drifting forward with each keystroke.
    pub origin: usize,
    /// Buffer version the matches were computed for.
    pub version: u64,
    pub error: Option<String>,
}

impl MekuShell {
    pub(crate) fn open_find(&mut self, replace: bool, cx: &mut Context<Self>) {
        if self.doc_rel().is_some() {
            self.find_request = Some(replace);
            cx.notify();
        }
    }

    /// Build the bar's inputs on the next render (they need a window).
    pub(crate) fn ensure_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(show_replace) = self.find_request.take() else {
            return;
        };
        let Some(rel) = self.doc_rel() else {
            return;
        };
        if let Some(f) = self.find.as_mut() {
            // Already open: just (re)focus, switching on replace if asked.
            f.show_replace |= show_replace;
            let target = if show_replace && f.show_replace {
                f.replace.clone()
            } else {
                f.query.clone()
            };
            window.focus(&target.read(cx).focus_handle(cx), cx);
            cx.notify();
            return;
        }
        // Seed the query with a single-line selection.
        let seed = self
            .buffers
            .get(&rel)
            .map(|b| b.read(cx).selected_text())
            .filter(|t| !t.is_empty() && !t.contains('\n'))
            .or_else(|| std::env::var("MEKU_FIND").ok())
            .unwrap_or_default();
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Find")
                .default_value(seed)
        });
        let replace = cx.new(|cx| InputState::new(window, cx).placeholder("Replace"));
        let mut subs = Vec::new();
        subs.push(
            cx.subscribe(&query, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => this.recompute_find(true, cx),
                InputEvent::PressEnter { shift, .. } => {
                    this.find_step(if *shift { -1 } else { 1 }, cx)
                }
                _ => {}
            }),
        );
        subs.push(cx.subscribe(&replace, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.replace_current(cx);
            }
        }));
        window.focus(&query.read(cx).focus_handle(cx), cx);
        let rel_for_origin = rel.clone();
        self.find = Some(FindState {
            query,
            replace,
            _subs: subs,
            opts: FindOptions::default(),
            show_replace,
            rel,
            matches: Vec::new(),
            current: None,
            origin: self
                .buffers
                .get(&rel_for_origin)
                .map_or(0, |b| b.read(cx).selection().start),
            version: u64::MAX,
            error: None,
        });
        self.recompute_find(true, cx);
    }

    pub(crate) fn close_find(&mut self, cx: &mut Context<Self>) {
        self.find_request = None;
        if self.find.take().is_some() {
            self.focus_doc = true;
            cx.notify();
        }
    }

    /// Recompute matches for the current query/options. With `pick`, the
    /// current match becomes the first one at or after the caret.
    pub(crate) fn recompute_find(&mut self, pick: bool, cx: &mut Context<Self>) {
        let Some(f) = self.find.as_ref() else {
            return;
        };
        let (rel, opts) = (f.rel.clone(), f.opts);
        let query = f.query.read(cx).value().to_string();
        let Some(buffer) = self.buffers.get(&rel).cloned() else {
            return;
        };
        let (version, text) = {
            let b = buffer.read(cx);
            (b.version, b.text_string())
        };
        let caret = self.find.as_ref().map_or(0, |f| f.origin);
        let (matches, error) = if query.is_empty() {
            (Vec::new(), None)
        } else {
            match compile(&query, opts) {
                Ok(re) => (find_matches(&text, &re), None),
                Err(e) => (Vec::new(), Some(e)),
            }
        };
        if let Some(f) = self.find.as_mut() {
            let keep = f.current.filter(|_| !pick);
            f.current = if matches.is_empty() {
                None
            } else if let Some(i) = keep {
                Some(i.min(matches.len() - 1))
            } else {
                Some(matches.iter().position(|m| m.start >= caret).unwrap_or(0))
            };
            f.matches = matches;
            f.error = error;
            f.version = version;
        }
        if pick {
            self.reveal_current_match(cx);
        }
        cx.notify();
    }

    /// Select the current match in the note and scroll it into view.
    fn reveal_current_match(&mut self, cx: &mut Context<Self>) {
        let Some((rel, range)) = self
            .find
            .as_ref()
            .and_then(|f| Some((f.rel.clone(), f.matches.get(f.current?)?.clone())))
        else {
            return;
        };
        if let Some(buffer) = self.buffers.get(&rel).cloned() {
            buffer.update(cx, |b, _| b.select_range(range.clone()));
        }
        self.scroll_to_offset(range.start, cx);
    }

    pub(crate) fn find_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(f) = self.find.as_mut() else {
            return;
        };
        if f.matches.is_empty() {
            return;
        }
        let n = f.matches.len() as isize;
        let at = f.current.map_or(0, |i| i as isize);
        f.current = Some((at + delta).rem_euclid(n) as usize);
        self.reveal_current_match(cx);
        cx.notify();
    }

    pub(crate) fn find_toggle(&mut self, which: u8, cx: &mut Context<Self>) {
        if let Some(f) = self.find.as_mut() {
            match which {
                0 => f.opts.case_sensitive = !f.opts.case_sensitive,
                1 => f.opts.whole_word = !f.opts.whole_word,
                _ => f.opts.regex = !f.opts.regex,
            }
        }
        self.recompute_find(true, cx);
    }

    fn replacement_text(&self, cx: &App) -> String {
        self.find
            .as_ref()
            .map(|f| f.replace.read(cx).value().to_string())
            .unwrap_or_default()
    }

    pub(crate) fn replace_current(&mut self, cx: &mut Context<Self>) {
        let with = self.replacement_text(cx);
        let Some(f) = self.find.as_ref() else {
            return;
        };
        let (rel, opts, at) = (f.rel.clone(), f.opts, f.current);
        let Some(range) = at.and_then(|i| f.matches.get(i).cloned()) else {
            return;
        };
        let query = f.query.read(cx).value().to_string();
        let (Some(buffer), Ok(re)) = (self.buffers.get(&rel).cloned(), compile(&query, opts))
        else {
            return;
        };
        let text = buffer.read(cx).text_string();
        let edits = replacements(&text, &re, std::slice::from_ref(&range), &with, opts.regex);
        buffer.update(cx, |b, _| b.replace_ranges(&edits));
        self.doc_note_edit(&rel);
        // Keep stepping forward from where the replacement ended.
        let resume = range.start + edits[0].1.len();
        buffer.update(cx, |b, _| b.move_caret(resume, false));
        if let Some(f) = self.find.as_mut() {
            f.origin = resume;
        }
        self.recompute_find(true, cx);
    }

    pub(crate) fn replace_all(&mut self, cx: &mut Context<Self>) {
        let with = self.replacement_text(cx);
        let Some(f) = self.find.as_ref() else {
            return;
        };
        let (rel, opts) = (f.rel.clone(), f.opts);
        let query = f.query.read(cx).value().to_string();
        let (Some(buffer), Ok(re)) = (self.buffers.get(&rel).cloned(), compile(&query, opts))
        else {
            return;
        };
        let text = buffer.read(cx).text_string();
        let matches = find_matches(&text, &re);
        if matches.is_empty() {
            return;
        }
        let count = matches.len();
        let edits = replacements(&text, &re, &matches, &with, opts.regex);
        buffer.update(cx, |b, _| b.replace_ranges(&edits));
        self.doc_note_edit(&rel);
        self.notice = Some(format!(
            "Replaced {count} occurrence{}",
            if count == 1 { "" } else { "s" }
        ));
        self.recompute_find(false, cx);
    }

    /// Match ranges to highlight in the document: `(range, is_current)`.
    pub(crate) fn find_highlights(&self, rel: &std::path::Path) -> Vec<(Range<usize>, bool)> {
        match self.find.as_ref() {
            Some(f) if f.rel == rel => f
                .matches
                .iter()
                .enumerate()
                .map(|(i, m)| (m.clone(), Some(i) == f.current))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn find_focused(&self, window: &Window, cx: &App) -> bool {
        self.find.as_ref().is_some_and(|f| {
            f.query.read(cx).focus_handle(cx).is_focused(window)
                || f.replace.read(cx).focus_handle(cx).is_focused(window)
        })
    }

    pub(crate) fn render_find_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        // Matches follow edits made while the bar is open.
        let stale = {
            let f = self.find.as_ref()?;
            self.buffers
                .get(&f.rel)
                .is_some_and(|b| b.read(cx).version != f.version)
        };
        if stale {
            self.recompute_find(false, cx);
        }
        let f = self.find.as_ref()?;
        let shell = cx.entity();
        let count = match (&f.error, f.matches.len(), f.current) {
            (Some(_), _, _) => "Invalid".to_string(),
            (None, 0, _) if f.query.read(cx).value().is_empty() => String::new(),
            (None, 0, _) => "No results".to_string(),
            (None, n, cur) => format!("{} of {}", cur.map_or(0, |c| c + 1), n),
        };
        let toggle = |id: &'static str, label: &'static str, on: bool, which: u8| {
            let shell = shell.clone();
            Button::new(id)
                .label(label)
                .xsmall()
                .ghost()
                .selected(on)
                .on_click(move |_, _, cx| {
                    shell.update(cx, |view, cx| view.find_toggle(which, cx));
                })
        };
        let step = |id: &'static str, label: &'static str, delta: isize| {
            let shell = shell.clone();
            Button::new(id)
                .label(label)
                .xsmall()
                .ghost()
                .on_click(move |_, _, cx| {
                    shell.update(cx, |view, cx| view.find_step(delta, cx));
                })
        };
        let close_shell = shell.clone();
        let mut bar = v_flex()
            .id("find-bar")
            .test_support()
            .absolute()
            .top(px(8.))
            .right(px(16.))
            .w(px(420.))
            .gap_1()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .shadow_lg()
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div().flex_1().child(
                            Input::new(&f.query)
                                .small()
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false),
                        ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(if f.error.is_some() {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(count),
                    )
                    .child(toggle("find-case", "Aa", f.opts.case_sensitive, 0))
                    .child(toggle("find-word", "W", f.opts.whole_word, 1))
                    .child(toggle("find-regex", ".*", f.opts.regex, 2))
                    .child(step("find-prev", "↑", -1))
                    .child(step("find-next", "↓", 1))
                    .child(
                        Button::new("find-close")
                            .label("×")
                            .xsmall()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                close_shell.update(cx, |view, cx| view.close_find(cx));
                            }),
                    ),
            );
        if f.show_replace {
            let one = shell.clone();
            let all = shell.clone();
            bar = bar.child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div().flex_1().child(
                            Input::new(&f.replace)
                                .small()
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false),
                        ),
                    )
                    .child(
                        Button::new("replace-one")
                            .label("Replace")
                            .xsmall()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                one.update(cx, |view, cx| view.replace_current(cx));
                            }),
                    )
                    .child(
                        Button::new("replace-all")
                            .label("All")
                            .xsmall()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                all.update(cx, |view, cx| view.replace_all(cx));
                            }),
                    ),
            );
        }
        Some(bar.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{FindOptions, compile, find_matches, replacements};
    use std::ops::Range;

    fn run(text: &str, q: &str, opts: FindOptions) -> Vec<Range<usize>> {
        find_matches(text, &compile(q, opts).unwrap())
    }

    #[test]
    fn plain_search_is_case_insensitive_and_literal() {
        let o = FindOptions::default();
        assert_eq!(run("Foo foo FOO", "foo", o), vec![0..3, 4..7, 8..11]);
        assert_eq!(run("a.b axb", "a.b", o), vec![0..3], "dot is literal");
        assert!(run("abc", "", o).is_empty(), "empty matches nothing");
    }

    #[test]
    fn case_and_word_options() {
        let case = FindOptions {
            case_sensitive: true,
            ..Default::default()
        };
        assert_eq!(run("Foo foo", "foo", case), vec![4..7]);
        let word = FindOptions {
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(run("cat concat cat.", "cat", word), vec![0..3, 11..14]);
    }

    #[test]
    fn regex_mode_and_errors() {
        let re = FindOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(run("a1 b22 c", r"\d+", re), vec![1..2, 4..6]);
        assert!(compile("(unclosed", re).is_err());
        assert!(compile("(unclosed", FindOptions::default()).is_ok());
    }

    #[test]
    fn replacements_expand_groups_only_in_regex_mode() {
        let re_opts = FindOptions {
            regex: true,
            ..Default::default()
        };
        let re = compile(r"(\w+)@(\w+)", re_opts).unwrap();
        let text = "me@home";
        let m = find_matches(text, &re);
        let edits = replacements(text, &re, &m, "$2 at $1", true);
        assert_eq!(edits, vec![(0..7, "home at me".to_string())]);
        let lit = replacements(text, &re, &m, "$2", false);
        assert_eq!(lit[0].1, "$2");
    }
}
