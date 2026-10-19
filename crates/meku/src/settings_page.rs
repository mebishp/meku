//! The settings page: every registry setting, global or per mekuto, plus
//! themes and custom themes. Opened with Ctrl+, or the sidebar's gear.
//!
//! Edits write straight to `settings.json` (global) or
//! `<mekuto>/.meku/settings.json`; a file with broken JSON is shown as a
//! banner and never written over until the user backs it up.

use std::collections::HashMap;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;
use serde_json::{Value, json};

use crate::MekuShell;
use crate::config::{DEFS, Def, Kind, Scope, Section};
use crate::icons::{Ico, icon};
use crate::theme::{self, Palette};

actions!(meku_settings, [OpenSettings]);

/// Which page of the settings is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Section(Section),
}

impl Tab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Tab::Section(s) => s.label(),
        }
    }

    fn ico(self) -> Ico {
        match self {
            Tab::Section(Section::Appearance) => Ico::Sun,
            Tab::Section(Section::Layout) => Ico::SidebarExpand,
            Tab::Section(Section::Editor) => Ico::Live,
            Tab::Section(Section::Markdown) => Ico::Notes,
            Tab::Section(Section::Files) => Ico::Folder,
        }
    }

    pub(crate) fn all() -> Vec<Tab> {
        Section::ALL.iter().map(|s| Tab::Section(*s)).collect()
    }
}

/// State of the open settings page.
pub(crate) struct SettingsPage {
    pub scope: Scope,
    pub tab: Tab,
    search: Entity<InputState>,
    texts: HashMap<&'static str, Entity<InputState>>,
    _subs: Vec<Subscription>,
    /// Result of the last action that needs telling (a refused write, a
    /// backup path).
    pub note: Option<String>,
}

type Click = Box<dyn Fn(&mut Window, &mut App)>;

impl MekuShell {
    pub(crate) fn on_open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(None, window, cx);
    }

    /// Show the page (on `tab`, or where it was). A second call just
    /// re-focuses the search box.
    pub(crate) fn open_settings(
        &mut self,
        tab: Option<Tab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(page) = self.settings.as_mut() {
            if let Some(tab) = tab {
                page.tab = tab;
            }
            window.focus(&page.search.read(cx).focus_handle(cx), cx);
            cx.notify();
            return;
        }
        // The command palette and other overlays step aside.
        self.close_palette(true, cx);
        self.close_find(cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings…"));
        let mut subs = vec![cx.subscribe(&search, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })];
        let mut texts = HashMap::new();
        for def in DEFS.iter().filter(|d| matches!(d.kind, Kind::Text)) {
            let key = def.key;
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(def.label));
            subs.push(cx.subscribe(&input, move |this, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.settings_commit_text(key, cx);
                }
            }));
            texts.insert(key, input);
        }
        self.settings = Some(SettingsPage {
            scope: Scope::Global,
            tab: tab.unwrap_or(Tab::Section(Section::Appearance)),
            search,
            texts,
            _subs: subs,
            note: None,
        });
        self.settings_sync(window, cx);
        if let Some(page) = &self.settings {
            window.focus(&page.search.read(cx).focus_handle(cx), cx);
        }
        cx.notify();
    }

    /// Close the page; `true` when it was open.
    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) -> bool {
        let was_open = self.settings.take().is_some();
        if was_open {
            self.focus_doc = true;
            cx.notify();
        }
        was_open
    }

    /// Put the current values into the text inputs.
    fn settings_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.settings.as_ref() else {
            return;
        };
        let values = theme::state(cx).effective_for(page.scope);
        let inputs: Vec<_> = page.texts.iter().map(|(k, i)| (*k, i.clone())).collect();
        for (key, input) in inputs {
            let text = values.str(key).to_string();
            input.update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }

    fn settings_commit_text(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let Some(page) = self.settings.as_ref() else {
            return;
        };
        let Some(input) = page.texts.get(key) else {
            return;
        };
        let text = input.read(cx).value().trim().to_string();
        let scope = page.scope;
        let shown = theme::state(cx).effective_for(scope);
        if shown.str(key) == text {
            return; // nothing changed: do not pin the default into the file
        }
        self.settings_write(key, Value::String(text), cx);
    }

    /// Write `key` in the page's scope; a refusal lands in the page's note.
    pub(crate) fn settings_write(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        let Some(scope) = self.settings.as_ref().map(|p| p.scope) else {
            return;
        };
        let result = theme::update_config(cx, |s| s.write(scope, key, value));
        if let Some(page) = self.settings.as_mut() {
            page.note = result.err();
        }
        cx.notify();
    }

    fn settings_reset(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = self.settings.as_ref().map(|p| p.scope) else {
            return;
        };
        let result = theme::update_config(cx, |s| s.clear(scope, key));
        if let Some(page) = self.settings.as_mut() {
            page.note = result.err();
        }
        self.settings_sync(window, cx);
        cx.notify();
    }

    fn settings_set_scope(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        if scope == Scope::Mekuto && theme::state(cx).mekuto.is_none() {
            return;
        }
        if let Some(page) = self.settings.as_mut() {
            page.scope = scope;
            page.note = None;
        }
        self.settings_sync(window, cx);
        cx.notify();
    }

    fn settings_open_file(&mut self, scope: Scope, cx: &mut Context<Self>) {
        let path = theme::state(cx).ensure_file(scope);
        let note = match path {
            Some(p) => match std::process::Command::new("xdg-open").arg(&p).spawn() {
                Ok(_) => None,
                Err(e) => Some(format!("Could not open {}: {e}", p.display())),
            },
            None => Some("There is no settings file for that scope".to_string()),
        };
        if let Some(page) = self.settings.as_mut() {
            page.note = note;
        }
        cx.notify();
    }

    fn settings_backup(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        let result = theme::update_config(cx, |s| s.backup_and_reset(scope));
        if let Some(page) = self.settings.as_mut() {
            page.note = Some(match result {
                Ok(path) => format!("Saved the broken file as {}", path.display()),
                Err(e) => e,
            });
        }
        self.settings_sync(window, cx);
        cx.notify();
    }

    /// Add a starter custom theme (based on the current one) to the scope's
    /// `themes` list.
    fn settings_new_theme(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.settings.as_ref().map(|p| p.scope) else {
            return;
        };
        let current = theme::active(cx);
        let taken: Vec<String> = theme::palettes(cx)
            .iter()
            .map(|p| p.name.to_lowercase())
            .collect();
        let name = (1..)
            .map(|n| {
                if n == 1 {
                    "My theme".to_string()
                } else {
                    format!("My theme {n}")
                }
            })
            .find(|n| !taken.contains(&n.to_lowercase()))
            .unwrap_or_else(|| "My theme".into());
        let mut list: Vec<Value> = theme::state(cx)
            .layer(scope)
            .and_then(|l| l.values.get("themes"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        list.push(json!({
            "name": name,
            "base": current.name,
            "dark": current.dark,
            "accent": format!("#{:06x}", current.accent),
        }));
        let result = theme::update_config(cx, |s| s.write_themes(scope, list));
        if let Some(page) = self.settings.as_mut() {
            page.note = Some(match result {
                Ok(()) => format!("Added “{name}”. Change its colours in settings.json."),
                Err(e) => e,
            });
        }
        cx.notify();
    }

    fn settings_delete_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(scope) = self.settings.as_ref().map(|p| p.scope) else {
            return;
        };
        let mut list: Vec<Value> = theme::state(cx)
            .layer(scope)
            .and_then(|l| l.values.get("themes"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        list.retain(|t| {
            t.get("name")
                .and_then(Value::as_str)
                .is_none_or(|n| !n.eq_ignore_ascii_case(name))
        });
        let result = theme::update_config(cx, |s| s.write_themes(scope, list));
        if let Some(page) = self.settings.as_mut() {
            page.note = result.err();
        }
        cx.notify();
    }

    pub(crate) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (scope, tab, search, note) = {
            let page = self.settings.as_ref()?;
            (page.scope, page.tab, page.search.clone(), page.note.clone())
        };
        let shell = cx.entity();
        let colors = cx.theme();
        let (border, muted, fg) = (colors.border, colors.muted_foreground, colors.foreground);
        let state = theme::state(cx).clone();
        let query = search.read(cx).value().trim().to_lowercase();
        let has_mekuto = state.mekuto.is_some();
        let view = viewport_panel(window);

        // -- header: title, scope switch, close
        let scope_chip = |id: &'static str, label: &'static str, target: Scope, enabled: bool| {
            let shell = shell.clone();
            chip(
                id,
                label,
                scope == target,
                enabled,
                cx,
                move |window, cx| {
                    shell.update(cx, |v, cx| v.settings_set_scope(target, window, cx));
                },
            )
        };
        let close_shell = shell.clone();
        let header = h_flex()
            .flex_none()
            .px_4()
            .py_3()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(border)
            .child(icon(Ico::Settings, px(18.), fg))
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .child(
                h_flex()
                    .gap_1()
                    .ml_3()
                    .child(scope_chip(
                        "settings-scope-global",
                        "Global",
                        Scope::Global,
                        true,
                    ))
                    .child(scope_chip(
                        "settings-scope-mekuto",
                        "This mekuto",
                        Scope::Mekuto,
                        has_mekuto,
                    )),
            )
            .child(div().flex_1())
            .child(
                div()
                    .w(px(220.))
                    .child(Input::new(&search).appearance(true).cleanable(true)),
            )
            .child(small_button(
                "settings-close",
                Ico::Close,
                "Close (Esc)",
                cx,
                move |_, cx| {
                    close_shell.update(cx, |v, cx| {
                        v.close_settings(cx);
                    });
                },
            ));

        // -- left nav
        let mut nav = v_flex()
            .flex_none()
            .w(px(170.))
            .p_2()
            .gap_0p5()
            .border_r_1()
            .border_color(border);
        for t in Tab::all() {
            let shell = shell.clone();
            let on = query.is_empty() && t == tab;
            let id = SharedString::from(format!("settings-tab-{}", t.label().to_lowercase()));
            let mut row = h_flex()
                .id(id)
                .test_support()
                .px_2()
                .py_1p5()
                .gap_2()
                .items_center()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                .text_color(if on { fg } else { muted })
                .hover(|s| s.bg(cx.theme().list_hover))
                .child(icon(t.ico(), px(15.), if on { fg } else { muted }))
                .child(t.label())
                .on_click(move |_, _, cx| {
                    shell.update(cx, |v, cx| {
                        if let Some(p) = v.settings.as_mut() {
                            p.tab = t;
                        }
                        cx.notify();
                    });
                });
            if on {
                row = row.bg(cx.theme().list_active);
            }
            nav = nav.child(row);
        }

        // -- body
        let mut body = v_flex().gap_3().p_4();
        // Fatal JSON errors first, whichever scope is showing.
        for issue in state.issues.iter().filter(|i| i.fatal) {
            body = body.child(self.json_banner(issue.scope, &issue.message, cx));
        }
        if let Some(note) = note {
            body = body.child(
                div()
                    .id("settings-note")
                    .test_support()
                    .text_xs()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(cx.theme().list_active)
                    .text_color(fg)
                    .child(note),
            );
        }
        if !query.is_empty() {
            let hits: Vec<&'static Def> = DEFS
                .iter()
                .filter(|d| {
                    d.label.to_lowercase().contains(&query)
                        || d.key.contains(&query)
                        || d.help.to_lowercase().contains(&query)
                })
                .collect();
            if hits.is_empty() {
                body = body.child(
                    div()
                        .id("settings-empty")
                        .test_support()
                        .text_sm()
                        .text_color(muted)
                        .child(format!("No settings match “{query}”.")),
                );
            }
            for def in hits {
                body = body.child(self.setting_row(def, scope, &state, cx));
            }
        } else {
            let Tab::Section(section) = tab;
            body = body.child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(section.label()),
            );
            if section == Section::Appearance {
                // Themes live here: pickers, gallery, custom themes.
                body = body.child(self.themes_page(scope, &state, cx));
            }
            let themed = ["follow_system", "theme_light", "theme_dark", "theme"];
            for def in DEFS
                .iter()
                .filter(|d| d.section == section && !themed.contains(&d.key))
            {
                body = body.child(self.setting_row(def, scope, &state, cx));
            }
        }
        // Problems that belong to no visible row.
        let stray: Vec<_> = state
            .issues
            .iter()
            .filter(|i| {
                !i.fatal
                    && i.key.as_deref().is_none_or(|k| {
                        crate::config::def_for(k).is_none() && !k.starts_with("themes")
                    })
            })
            .collect();
        if !stray.is_empty() {
            let mut list = v_flex()
                .id("settings-problems")
                .test_support()
                .gap_0p5()
                .text_xs()
                .text_color(muted);
            for i in stray {
                list = list.child(format!(
                    "{}: {}{}",
                    i.scope.label(),
                    i.key
                        .as_deref()
                        .map(|k| format!("“{k}” — "))
                        .unwrap_or_default(),
                    i.message
                ));
            }
            body = body.child(list);
        }

        let content = div()
            .id("settings-scroll")
            .test_support()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(body);

        let footer_path = state
            .layer(scope)
            .and_then(|l| l.path.clone())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(not saved: no folder is open)".into());
        let open_shell = shell.clone();
        let footer = h_flex()
            .flex_none()
            .px_4()
            .py_2()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(border)
            .text_xs()
            .text_color(muted)
            .child(div().flex_1().truncate().child(footer_path))
            .child(text_button(
                "settings-open-file",
                "Open settings.json",
                cx,
                move |_, cx| {
                    open_shell.update(cx, |v, cx| v.settings_open_file(scope, cx));
                },
            ));

        let panel = v_flex()
            .id("settings-page")
            .test_support()
            .w(view.0)
            .h(view.1)
            .rounded_xl()
            .border_1()
            .border_color(border)
            .bg(cx.theme().background)
            .text_color(fg)
            .shadow_lg()
            .overflow_hidden()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .overflow_hidden()
                    .child(nav)
                    .child(content),
            )
            .child(footer);

        let backdrop_shell = shell.clone();
        let root = div()
            .id("settings-backdrop")
            .test_support()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim(cx))
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                backdrop_shell.update(cx, |v, cx| {
                    v.close_settings(cx);
                });
            })
            .child(panel);
        Some(theme::fade_in(root, "settings-in", 140, 0.0, cx))
    }

    /// Red-ish banner for a settings file that is not valid JSON.
    fn json_banner(&self, scope: Scope, message: &str, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.entity();
        let (open, reset) = (shell.clone(), shell);
        let danger = cx.theme().danger;
        let id = SharedString::from(format!(
            "settings-banner-{}",
            if scope == Scope::Global {
                "global"
            } else {
                "mekuto"
            }
        ));
        v_flex()
            .id(id)
            .test_support()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(danger)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(icon(Ico::Warning, px(16.), danger))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} can't be read", capitalize(scope.label()))),
                    ),
            )
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!(
                "{message}. Everything else keeps its default (or the other file's value), and edits are paused for this file so nothing is lost."
            )))
            .child(
                h_flex()
                    .gap_2()
                    .child(text_button(
                        if scope == Scope::Global { "banner-open-global" } else { "banner-open-mekuto" },
                        "Open file to fix",
                        cx,
                        move |_, cx| open.update(cx, |v, cx| v.settings_open_file(scope, cx)),
                    ))
                    .child(text_button(
                        if scope == Scope::Global { "banner-reset-global" } else { "banner-reset-mekuto" },
                        "Back up and start fresh",
                        cx,
                        move |window, cx| {
                            reset.update(cx, |v, cx| v.settings_backup(scope, window, cx));
                        },
                    )),
            )
            .into_any_element()
    }

    /// One setting: label, help, control, modified dot and reset.
    fn setting_row(
        &self,
        def: &'static Def,
        scope: Scope,
        state: &crate::config::ConfigState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shell = cx.entity();
        let colors = cx.theme();
        let (border, muted, accent) = (colors.border, colors.muted_foreground, colors.primary);
        let values = state.effective_for(scope);
        let is_set = state.is_set(scope, def.key);
        let key = def.key;

        let control: AnyElement = match def.kind {
            Kind::Bool => {
                let shell = shell.clone();
                Switch::new(SharedString::from(format!("set-{key}")))
                    .checked(values.bool(key))
                    .on_click(move |on, _, cx| {
                        let on = *on;
                        shell.update(cx, |v, cx| v.settings_write(key, Value::Bool(on), cx));
                    })
                    .into_any_element()
            }
            Kind::Choice(options) => {
                let current = values.str(key).to_string();
                let mut row = h_flex().gap_1().flex_wrap().justify_end();
                for opt in options {
                    let shell = shell.clone();
                    let id = SharedString::from(format!("set-{key}-{opt}"));
                    row = row.child(chip_dyn(
                        id,
                        label_of(opt),
                        current == *opt,
                        true,
                        cx,
                        move |_, cx| {
                            shell.update(cx, |v, cx| {
                                v.settings_write(key, Value::String((*opt).into()), cx)
                            });
                        },
                    ));
                }
                row.into_any_element()
            }
            Kind::Number { min, max, step } => {
                let value = values.f64(key);
                let (down, up) = (shell.clone(), shell.clone());
                let decimals = usize::from(step < 1.0);
                let apply = move |delta: f64| {
                    let next = ((value + delta) / step).round() * step;
                    let next = next.clamp(min, max);
                    // Keep one decimal clean (1.2, not 1.2000000000000002).
                    if decimals == 1 {
                        (next * 10.0).round() / 10.0
                    } else {
                        next.round()
                    }
                };
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(small_button(
                        SharedString::from(format!("set-{key}-down")),
                        Ico::Minus,
                        "Decrease",
                        cx,
                        move |_, cx| {
                            let v = apply(-step);
                            down.update(cx, |s, cx| s.settings_write(key, json!(v), cx));
                        },
                    ))
                    .child(
                        div()
                            .id(SharedString::from(format!("set-{key}-value")))
                            .test_support()
                            .min_w(px(56.))
                            .text_center()
                            .text_sm()
                            .child(format!("{value:.decimals$}")),
                    )
                    .child(small_button(
                        SharedString::from(format!("set-{key}-up")),
                        Ico::Plus,
                        "Increase",
                        cx,
                        move |_, cx| {
                            let v = apply(step);
                            up.update(cx, |s, cx| s.settings_write(key, json!(v), cx));
                        },
                    ))
                    .into_any_element()
            }
            Kind::Text => {
                let input = self
                    .settings
                    .as_ref()
                    .and_then(|p| p.texts.get(key))
                    .cloned();
                match input {
                    Some(input) => div()
                        .w(px(220.))
                        .child(Input::new(&input))
                        .into_any_element(),
                    None => div().into_any_element(),
                }
            }
            Kind::Theme => {
                let current = values.str(key).to_string();
                let mut row = h_flex().gap_1().flex_wrap();
                for p in theme::palettes_of(state) {
                    let shell = shell.clone();
                    let name = p.name;
                    let on = current.eq_ignore_ascii_case(name);
                    let id = SharedString::from(format!(
                        "set-{key}-{}",
                        name.to_lowercase().replace(' ', "-")
                    ));
                    row = row.child(swatch_chip(id, p, on, cx, move |_, cx| {
                        shell.update(cx, |v, cx| {
                            v.settings_write(key, Value::String(name.into()), cx)
                        });
                    }));
                }
                row.into_any_element()
            }
        };

        let mut title = h_flex().gap_2().items_center().child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(def.label),
        );
        if is_set {
            title = title.child(
                div()
                    .id(SharedString::from(format!("set-{key}-dot")))
                    .test_support()
                    .size(px(6.))
                    .rounded_full()
                    .bg(accent),
            );
        }
        let mut info = v_flex().flex_1().min_w_0().gap_0p5().child(title);
        if !def.help.is_empty() {
            info = info.child(div().text_xs().text_color(muted).child(def.help));
        }
        if scope == Scope::Global && state.is_set(Scope::Mekuto, key) {
            info = info.child(
                div()
                    .text_xs()
                    .text_color(accent)
                    .child("This mekuto overrides it."),
            );
        }
        for issue in state
            .issues_for(key)
            .into_iter()
            .filter(|i| i.scope == scope)
        {
            info = info.child(
                div()
                    .id(SharedString::from(format!("set-{key}-issue")))
                    .test_support()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(issue.message.clone()),
            );
        }
        let reset: Option<AnyElement> = is_set.then(|| {
            let shell = shell.clone();
            small_button(
                SharedString::from(format!("set-{key}-reset")),
                Ico::Undo,
                if scope == Scope::Mekuto {
                    "Use the global value"
                } else {
                    "Reset to default"
                },
                cx,
                move |window, cx| {
                    shell.update(cx, |v, cx| v.settings_reset(key, window, cx));
                },
            )
        });
        let row_id = SharedString::from(format!("setting-{key}"));
        if matches!(def.kind, Kind::Theme) {
            // Theme pickers are wide: the chips get a line of their own.
            return v_flex()
                .id(row_id)
                .test_support()
                .gap_2()
                .py_3()
                .border_b_1()
                .border_color(border)
                .child(
                    h_flex()
                        .gap_4()
                        .items_start()
                        .justify_between()
                        .child(info)
                        .children(reset),
                )
                .child(control)
                .into_any_element();
        }
        h_flex()
            .id(row_id)
            .test_support()
            .gap_4()
            .py_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(border)
            .child(info)
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .items_center()
                    .child(control)
                    .children(reset),
            )
            .into_any_element()
    }

    fn themes_page(
        &self,
        scope: Scope,
        state: &crate::config::ConfigState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shell = cx.entity();
        let colors = cx.theme();
        let (border, muted) = (colors.border, colors.muted_foreground);
        let active = theme::active(cx).name;
        let mut page = v_flex().gap_3();
        for key in ["follow_system", "theme_light", "theme_dark", "theme"] {
            if let Some(def) = crate::config::def_for(key) {
                page = page.child(self.setting_row(def, scope, state, cx));
            }
        }
        // Gallery of every theme.
        let mut grid = h_flex()
            .gap_3()
            .flex_wrap()
            .id("settings-theme-gallery")
            .test_support();
        let custom: Vec<String> = state.themes.iter().map(|t| t.name.to_lowercase()).collect();
        for p in theme::palettes_of(state) {
            let shell = shell.clone();
            let on = p.name == active;
            let is_custom = custom.contains(&p.name.to_lowercase());
            grid = grid.child(theme_card(p, on, is_custom, cx, move |_, cx| {
                theme::choose(p.name, cx);
                shell.update(cx, |_, cx| cx.notify());
            }));
        }
        page = page.child(grid);

        // Custom themes in the scope being edited.
        let mine: Vec<_> = state.themes.iter().filter(|t| t.scope == scope).collect();
        let mut custom_box = v_flex()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(border)
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Custom themes"))
            .child(div().text_xs().text_color(muted).child(
                "Add themes to the \"themes\" list in settings.json. Each needs a \"name\"; \"base\" names a theme to start from, \"dark\" picks light or dark, and any of bg, surface, border, fg, muted, accent, keyword, string, number, func, ty, constant, prop, comment, code can be set as \"#rrggbb\".",
            ));
        for t in &mine {
            let shell = shell.clone();
            let name = t.name.clone();
            let del_name = name.clone();
            custom_box = custom_box.child(
                h_flex()
                    .id(SharedString::from(format!(
                        "custom-theme-{}",
                        name.to_lowercase().replace(' ', "-")
                    )))
                    .test_support()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().text_sm().child(name))
                    .child(small_button(
                        SharedString::from(format!(
                            "custom-theme-del-{}",
                            del_name.to_lowercase().replace(' ', "-")
                        )),
                        Ico::Trash,
                        "Remove theme",
                        cx,
                        move |_, cx| {
                            let n = del_name.clone();
                            shell.update(cx, |v, cx| v.settings_delete_theme(&n, cx));
                        },
                    )),
            );
        }
        for issue in state.issues.iter().filter(|i| {
            i.scope == scope && i.key.as_deref().is_some_and(|k| k.starts_with("themes"))
        }) {
            custom_box =
                custom_box.child(div().text_xs().text_color(cx.theme().danger).child(format!(
                    "{}: {}",
                    issue.key.as_deref().unwrap_or("themes"),
                    issue.message
                )));
        }
        let (add, open) = (shell.clone(), shell);
        custom_box = custom_box.child(
            h_flex()
                .gap_2()
                .child(text_button(
                    "settings-new-theme",
                    "New custom theme",
                    cx,
                    move |_, cx| {
                        add.update(cx, |v, cx| v.settings_new_theme(cx));
                    },
                ))
                .child(text_button(
                    "settings-theme-file",
                    "Open settings.json",
                    cx,
                    move |_, cx| {
                        open.update(cx, |v, cx| v.settings_open_file(scope, cx));
                    },
                )),
        );
        page.child(custom_box).into_any_element()
    }
}

/// Panel size: most of the window, capped.
fn viewport_panel(window: &Window) -> (Pixels, Pixels) {
    let v = window.viewport_size();
    (
        (v.width * 0.92).min(px(940.)).max(px(320.)),
        (v.height * 0.88).min(px(680.)).max(px(280.)),
    )
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn label_of(opt: &str) -> String {
    let mut s = opt.replace('-', " ");
    if let Some(first) = s.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    s
}

fn chip(
    id: &'static str,
    label: &'static str,
    on: bool,
    enabled: bool,
    cx: &App,
    click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    chip_dyn(
        SharedString::from(id),
        label.to_string(),
        on,
        enabled,
        cx,
        click,
    )
}

/// A pill button; `on` marks the selected one of a group.
fn chip_dyn(
    id: SharedString,
    label: String,
    on: bool,
    enabled: bool,
    cx: &App,
    click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let colors = cx.theme();
    let (fg, muted, border) = (colors.foreground, colors.muted_foreground, colors.border);
    let (active, hover) = (colors.list_active, colors.list_hover);
    let mut el = div()
        .id(id)
        .test_support()
        .px_2p5()
        .py_1()
        .rounded_md()
        .border_1()
        .text_xs()
        .child(label);
    if on {
        el = el.bg(active).border_color(colors.primary).text_color(fg);
    } else {
        el = el.border_color(border).text_color(muted);
    }
    if enabled {
        el = el
            .cursor_pointer()
            .hover(move |s| s.bg(hover).text_color(fg))
            .on_click(move |_, window, cx| click(window, cx));
    } else {
        el = el.opacity(0.4);
    }
    el.into_any_element()
}

/// A theme-name chip led by a swatch of its colours.
fn swatch_chip(
    id: SharedString,
    p: Palette,
    on: bool,
    cx: &App,
    click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let colors = cx.theme();
    let (fg, muted, border) = (colors.foreground, colors.muted_foreground, colors.border);
    let (active, hover) = (colors.list_active, colors.list_hover);
    let dot = div()
        .size(px(12.))
        .rounded_full()
        .border_1()
        .border_color(rgb(p.border))
        .bg(rgb(p.bg))
        .child(
            div()
                .size(px(5.))
                .mt(px(3.))
                .ml(px(3.))
                .rounded_full()
                .bg(rgb(p.accent)),
        );
    let mut el = h_flex()
        .id(id)
        .test_support()
        .gap_1p5()
        .items_center()
        .px_2()
        .py_1()
        .rounded_md()
        .border_1()
        .text_xs()
        .cursor_pointer()
        .child(dot)
        .child(p.name);
    if on {
        el = el.bg(active).border_color(colors.primary).text_color(fg);
    } else {
        el = el
            .border_color(border)
            .text_color(muted)
            .hover(move |s| s.bg(hover).text_color(fg));
    }
    el.on_click(move |_, window, cx| click(window, cx))
        .into_any_element()
}

/// Gallery card: a miniature of the theme and its name.
fn theme_card(
    p: Palette,
    on: bool,
    custom: bool,
    cx: &App,
    click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let colors = cx.theme();
    let ring = if on { colors.primary } else { colors.border };
    let id = SharedString::from(format!(
        "theme-card-{}",
        p.name.to_lowercase().replace(' ', "-")
    ));
    let preview = v_flex()
        .h(px(64.))
        .p_2()
        .gap_1()
        .bg(rgb(p.bg))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.fg))
                .child("Aa heading"),
        )
        .child(
            h_flex()
                .gap_1()
                .child(div().h(px(4.)).w(px(22.)).rounded_full().bg(rgb(p.accent)))
                .child(div().h(px(4.)).w(px(14.)).rounded_full().bg(rgb(p.muted)))
                .child(div().h(px(4.)).w(px(10.)).rounded_full().bg(rgb(p.string)))
                .child(div().h(px(4.)).w(px(8.)).rounded_full().bg(rgb(p.keyword))),
        );
    let label = h_flex()
        .px_2()
        .py_1p5()
        .gap_1()
        .items_center()
        .bg(rgb(p.surface))
        .text_xs()
        .text_color(rgb(p.fg))
        .child(div().flex_1().truncate().child(p.name))
        .children(custom.then(|| div().text_color(rgb(p.muted)).child("custom")))
        .children(on.then(|| icon(Ico::Check, px(12.), rgb(p.accent).into())));
    v_flex()
        .id(id)
        .test_support()
        .w(px(148.))
        .rounded_lg()
        .border_1()
        .border_color(ring)
        .overflow_hidden()
        .cursor_pointer()
        .child(preview)
        .child(label)
        .on_click(move |_, window, cx| click(window, cx))
        .into_any_element()
}

fn small_button(
    id: impl Into<SharedString>,
    ico: Ico,
    tip: &'static str,
    cx: &App,
    click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let colors = cx.theme();
    let (fg, muted, hover) = (
        colors.foreground,
        colors.muted_foreground,
        colors.list_hover,
    );
    let click: Click = Box::new(click);
    div()
        .id(id.into())
        .test_support()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded_md()
        .cursor_pointer()
        .text_color(muted)
        .hover(move |s| s.bg(hover).text_color(fg))
        .child(icon(ico, px(15.), muted))
        .tooltip(move |w, cx| gpui_kit::component::tooltip::Tooltip::new(tip).build(w, cx))
        .on_click(move |_, window, cx| click(window, cx))
        .into_any_element()
}

fn text_button(
    id: &'static str,
    label: &'static str,
    cx: &App,
    click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let colors = cx.theme();
    let (fg, border, hover) = (colors.foreground, colors.border, colors.list_hover);
    div()
        .id(id)
        .test_support()
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(border)
        .text_xs()
        .text_color(fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label)
        .on_click(move |_, window, cx| click(window, cx))
        .into_any_element()
}
