//! The settings page: every registry setting, global or per mekuto. Opened
//! with Ctrl+, or the sidebar's gear.
//!
//! Edits write straight to `settings.json` (global) or
//! `<mekuto>/.meku/settings.json`; a file with broken JSON is shown as a
//! banner and never written over until the user backs it up.

use std::collections::HashMap;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;
use serde_json::{Value, json};

use crate::MekuShell;
use crate::config::{ConfigState, DEFS, Def, Kind, Scope, Section};
use crate::icons::{Ico, icon};
use crate::theme;

actions!(meku_settings, [OpenSettings]);

/// Which section of the settings is showing.
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

    pub(crate) fn all() -> Vec<Tab> {
        Section::ALL.iter().map(|s| Tab::Section(*s)).collect()
    }
}

/// Theme settings; shown in a fixed order at the top of Appearance.
const THEME_KEYS: [&str; 4] = ["theme", "follow_system", "theme_light", "theme_dark"];

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
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
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
        let shown = theme::state(cx).effective_for(page.scope);
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
        self.settings_note(result.err());
        cx.notify();
    }

    /// Pick a theme: it also turns off following the system, so the choice
    /// takes effect right away.
    fn settings_pick_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(scope) = self.settings.as_ref().map(|p| p.scope) else {
            return;
        };
        theme::remember(cx);
        let result = theme::update_config(cx, |s| {
            s.env_theme = None;
            s.write(scope, "follow_system", Value::Bool(false))?;
            s.write(scope, "theme", Value::String(name.into()))
        });
        self.settings_note(result.err());
        cx.notify();
    }

    fn settings_note(&mut self, note: Option<String>) {
        if let Some(page) = self.settings.as_mut() {
            page.note = note;
        }
    }

    fn settings_reset(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = self.settings.as_ref().map(|p| p.scope) else {
            return;
        };
        let result = theme::update_config(cx, |s| s.clear(scope, key));
        self.settings_note(result.err());
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
        let note = match theme::state(cx).ensure_file(scope) {
            Some(p) => match std::process::Command::new("xdg-open").arg(&p).spawn() {
                Ok(_) => None,
                Err(e) => Some(format!("Could not open {}: {e}", p.display())),
            },
            None => Some("There is no settings file for that scope".to_string()),
        };
        self.settings_note(note);
        cx.notify();
    }

    fn settings_backup(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        let result = theme::update_config(cx, |s| s.backup_and_reset(scope));
        self.settings_note(Some(match result {
            Ok(path) => format!("Saved the broken file as {}", path.display()),
            Err(e) => e,
        }));
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
        self.settings_note(Some(match result {
            Ok(()) => format!("Added “{name}” to your themes. Edit its colours in settings.json."),
            Err(e) => e,
        }));
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
        let (panel_w, panel_h) = panel_size(window);

        // -- header: title, scope, search, close
        let scope_options = [
            ("settings-scope-global", "Global", Scope::Global, true),
            (
                "settings-scope-mekuto",
                "This mekuto",
                Scope::Mekuto,
                has_mekuto,
            ),
        ];
        let scope_switch = segmented(
            scope_options
                .iter()
                .map(|(id, label, target, enabled)| {
                    let shell = shell.clone();
                    let target = *target;
                    Segment {
                        id: SharedString::from(*id),
                        label: (*label).to_string(),
                        on: scope == target,
                        enabled: *enabled,
                        click: Box::new(move |window, cx| {
                            shell.update(cx, |v, cx| v.settings_set_scope(target, window, cx));
                        }),
                    }
                })
                .collect(),
            cx,
        );
        let close_shell = shell.clone();
        let header = h_flex()
            .flex_none()
            .h(px(52.))
            .px_4()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .child(scope_switch)
            .child(div().flex_1())
            .child(
                div()
                    .w(px(200.))
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

        // -- left nav: plain text, the current section tinted
        let mut nav = v_flex()
            .flex_none()
            .h_full()
            .w(px(148.))
            .p_2()
            .gap_0p5()
            .border_r_1()
            .border_color(border);
        let (hover, active_bg) = (cx.theme().list_hover, cx.theme().list_active);
        for t in Tab::all() {
            let shell = shell.clone();
            let on = query.is_empty() && t == tab;
            let id = SharedString::from(format!("settings-tab-{}", t.label().to_lowercase()));
            let mut row = div()
                .id(id)
                .test_support()
                .px_2p5()
                .py_1p5()
                .rounded_md()
                .cursor_pointer()
                .tab_stop(true)
                .focus_visible(move |s| s.bg(hover).text_color(fg))
                .text_sm()
                .text_color(if on { fg } else { muted })
                .hover(move |s| s.bg(hover).text_color(fg))
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
                row = row.bg(active_bg).font_weight(FontWeight::MEDIUM);
            }
            nav = nav.child(row);
        }

        // -- body
        let mut body = v_flex().w_full().max_w(px(560.)).gap_1().px_6().py_4();
        body = body.child(
            div()
                .id("settings-scope-note")
                .test_support()
                .pb_2()
                .text_xs()
                .text_color(muted)
                .child(match scope {
                    Scope::Global => "Applies to every mekuto, unless a mekuto overrides it.",
                    Scope::Mekuto => "Applies to this mekuto only and overrides Global.",
                }),
        );
        for issue in state.issues.iter().filter(|i| i.fatal) {
            body = body.child(self.json_banner(issue.scope, &issue.message, cx));
        }
        if let Some(note) = note {
            body = body.child(
                div()
                    .id("settings-note")
                    .test_support()
                    .py_2()
                    .text_xs()
                    .text_color(muted)
                    .child(note),
            );
        }
        let rows: Vec<&'static Def> = if query.is_empty() {
            let Tab::Section(section) = tab;
            let mut rows = ordered_defs(section, state.effective_for(scope).bool("follow_system"));
            // Keep each subhead's rows together (stable within a group).
            let rank = |k: &str| match group_of(k) {
                "View" | "Saving" => 1,
                "Typing" | "Folders" => 2,
                "Caret" | "Behaviour" => 3,
                _ => 0,
            };
            rows.sort_by_key(|d| rank(d.key));
            rows
        } else {
            DEFS.iter()
                .filter(|d| {
                    d.label.to_lowercase().contains(&query)
                        || d.key.contains(&query)
                        || d.help.to_lowercase().contains(&query)
                })
                .collect()
        };
        if rows.is_empty() {
            body = body.child(
                div()
                    .id("settings-empty")
                    .test_support()
                    .py_4()
                    .text_sm()
                    .text_color(muted)
                    .child(format!("No settings match “{query}”.")),
            );
        }
        let mut group = "";
        for def in rows {
            // Subheads inside a section (not while searching).
            let g = if query.is_empty() {
                group_of(def.key)
            } else {
                ""
            };
            if g != group && !g.is_empty() {
                body = body.child(
                    div()
                        .pt_4()
                        .pb_1()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(muted)
                        .child(g),
                );
            }
            group = g;
            body = body.child(self.setting_row(def, scope, &state, cx));
        }
        if query.is_empty() && tab == Tab::Section(Section::Appearance) {
            body = body.child(self.custom_themes_row(scope, &state, cx));
        }
        // Problems that belong to no visible row (unknown keys).
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
                .pt_3()
                .gap_0p5()
                .text_xs()
                .text_color(muted);
            for i in stray {
                list = list.child(format!(
                    "{}: {}{}",
                    capitalize(i.scope.label()),
                    i.key
                        .as_deref()
                        .map(|k| format!("“{k}” — "))
                        .unwrap_or_default(),
                    i.message
                ));
            }
            body = body.child(list);
        }

        let content = crate::smooth::frame(
            "settings-scrollbar",
            div().flex_1().min_w_0().h_full(),
            div().id("settings-scroll").test_support().child(body),
            &self.smooth.settings_h,
            &self.smooth.settings,
        );

        let path = state
            .layer(scope)
            .and_then(|l| l.path.clone())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Not saved: no folder is open".into());
        let open_shell = shell.clone();
        let footer = h_flex()
            .flex_none()
            .h(px(36.))
            .px_4()
            .gap_3()
            .items_center()
            .border_t_1()
            .border_color(border)
            .text_xs()
            .text_color(muted)
            .child(div().flex_1().truncate().child(path))
            .child(link_button(
                "settings-open-file",
                "Open file",
                cx,
                move |_, cx| {
                    open_shell.update(cx, |v, cx| v.settings_open_file(scope, cx));
                },
            ));

        let panel = v_flex()
            .id("settings-page")
            .test_support()
            .w(panel_w)
            .h(panel_h)
            .rounded_lg()
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

    /// A settings file that is not valid JSON: what, where, and the way out.
    fn json_banner(&self, scope: Scope, message: &str, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.entity();
        let (open, reset) = (shell.clone(), shell);
        let which = if scope == Scope::Global {
            "global"
        } else {
            "mekuto"
        };
        let mut tint = cx.theme().danger;
        tint.a = 0.12;
        v_flex()
            .id(SharedString::from(format!("settings-banner-{which}")))
            .test_support()
            .gap_2()
            .p_3()
            .mb_2()
            .rounded_md()
            .bg(tint)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("{} can't be read", capitalize(scope.label()))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{message}. Defaults are in use and edits to this file are paused."
                    )),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(link_button(
                        SharedString::from(format!("banner-open-{which}")),
                        "Open file",
                        cx,
                        move |_, cx| open.update(cx, |v, cx| v.settings_open_file(scope, cx)),
                    ))
                    .child(link_button(
                        SharedString::from(format!("banner-reset-{which}")),
                        "Back up and start fresh",
                        cx,
                        move |window, cx| {
                            reset.update(cx, |v, cx| v.settings_backup(scope, window, cx));
                        },
                    )),
            )
            .into_any_element()
    }

    /// One setting: label and one line of help on the left, its control on
    /// the right, and a reset when it differs from the inherited value.
    fn setting_row(
        &self,
        def: &'static Def,
        scope: Scope,
        state: &ConfigState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shell = cx.entity();
        let muted = cx.theme().muted_foreground;
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
                segmented(
                    options
                        .iter()
                        .map(|opt| {
                            let shell = shell.clone();
                            Segment {
                                id: SharedString::from(format!("set-{key}-{opt}")),
                                label: label_of(opt),
                                on: current == *opt,
                                enabled: true,
                                click: Box::new(move |_, cx| {
                                    shell.update(cx, |v, cx| {
                                        v.settings_write(key, Value::String((*opt).into()), cx)
                                    });
                                }),
                            }
                        })
                        .collect(),
                    cx,
                )
            }
            Kind::Number { min, max, step } => {
                let value = values.f64(key);
                let (down, up) = (shell.clone(), shell.clone());
                let decimals = usize::from(step < 1.0);
                let apply = move |delta: f64| {
                    let next = (((value + delta) / step).round() * step).clamp(min, max);
                    // Keep one decimal clean (1.2, not 1.2000000000000002).
                    if decimals == 1 {
                        (next * 10.0).round() / 10.0
                    } else {
                        next.round()
                    }
                };
                h_flex()
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
                            .min_w(px(52.))
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
            Kind::Text => match self.settings.as_ref().and_then(|p| p.texts.get(key)) {
                Some(input) => div()
                    .w(px(200.))
                    .child(Input::new(input))
                    .into_any_element(),
                None => div().into_any_element(),
            },
            Kind::Theme => {
                let current = values.str(key).to_string();
                let names: Vec<&'static str> =
                    theme::palettes_of(state).iter().map(|p| p.name).collect();
                let shown = names
                    .iter()
                    .find(|n| n.eq_ignore_ascii_case(&current))
                    .copied()
                    .unwrap_or("Meku Light");
                let picker = shell.clone();
                Button::new(SharedString::from(format!("set-{key}")))
                    .label(shown)
                    .outline()
                    .small()
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for name in &names {
                            let (name, picker) = (*name, picker.clone());
                            menu = menu.item(
                                PopupMenuItem::new(name)
                                    .checked(name.eq_ignore_ascii_case(&current))
                                    .on_click(move |_, _, cx| {
                                        picker.update(cx, |v, cx| {
                                            if key == "theme" {
                                                v.settings_pick_theme(name, cx);
                                            } else {
                                                v.settings_write(
                                                    key,
                                                    Value::String(name.into()),
                                                    cx,
                                                );
                                            }
                                        });
                                    }),
                            );
                        }
                        menu
                    })
                    .into_any_element()
            }
        };

        let mut info = v_flex()
            .flex_1()
            .min_w_0()
            .gap_0p5()
            .child(div().text_sm().child(def.label));
        if !def.help.is_empty() {
            info = info.child(div().text_xs().text_color(muted).child(def.help));
        }
        if scope == Scope::Global && state.is_set(Scope::Mekuto, key) {
            info = info.child(
                div()
                    .text_xs()
                    .text_color(muted)
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
        let reset = is_set.then(|| {
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
        h_flex()
            .id(SharedString::from(format!("setting-{key}")))
            .test_support()
            .gap_4()
            .py_2p5()
            .items_center()
            .justify_between()
            .child(info)
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .items_center()
                    .child(control)
                    // Keeps controls aligned whether or not a reset shows.
                    .child(div().w(px(26.)).children(reset)),
            )
            .into_any_element()
    }

    /// Custom themes live in settings.json; this row adds a starter entry
    /// and reports problems with the ones already there.
    fn custom_themes_row(
        &self,
        scope: Scope,
        state: &ConfigState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shell = cx.entity();
        let muted = cx.theme().muted_foreground;
        let mut info = v_flex()
            .flex_1()
            .gap_0p5()
            .child(div().text_sm().child("Custom themes"))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Add them under \"themes\" in settings.json."),
            );
        for issue in state.issues.iter().filter(|i| {
            i.scope == scope && i.key.as_deref().is_some_and(|k| k.starts_with("themes"))
        }) {
            info = info.child(div().text_xs().text_color(cx.theme().danger).child(format!(
                "{}: {}",
                issue.key.as_deref().unwrap_or("themes"),
                issue.message
            )));
        }
        h_flex()
            .id("settings-custom-themes")
            .test_support()
            .gap_4()
            .py_2p5()
            .items_center()
            .justify_between()
            .child(info)
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new("settings-new-theme")
                            .label("Add theme")
                            .outline()
                            .small()
                            .on_click(move |_, _, cx| {
                                shell.update(cx, |v, cx| v.settings_new_theme(cx));
                            }),
                    )
                    .child(div().w(px(26.))),
            )
            .into_any_element()
    }
}

/// The settings of `section` in display order. Appearance leads with the
/// theme, then whether to follow the system; the light/dark pair only
/// matters (and only shows) while following it.
fn ordered_defs(section: Section, follow: bool) -> Vec<&'static Def> {
    let mut rows: Vec<&'static Def> = Vec::new();
    if section == Section::Appearance {
        for key in THEME_KEYS {
            let pair = key == "theme_light" || key == "theme_dark";
            if pair && !follow {
                continue;
            }
            rows.extend(crate::config::def_for(key));
        }
    }
    rows.extend(
        DEFS.iter()
            .filter(|d| d.section == section && !THEME_KEYS.contains(&d.key)),
    );
    rows
}

/// Subhead a setting sits under within its section; empty for none.
fn group_of(key: &str) -> &'static str {
    match key {
        "default_mode" | "reveal_markdown" | "link_click" => "View",
        "auto_close_pairs" | "smart_lists" | "slash_menu" => "Typing",
        "caret_blink" | "caret_blink_ms" => "Caret",
        "autosave" | "autosave_delay_ms" => "Saving",
        "new_note_folder" | "attachments_folder" => "Folders",
        "confirm_delete" | "reopen_last_mekuto" | "restore_session" => "Behaviour",
        _ => "",
    }
}

/// Panel size: most of the window, capped.
fn panel_size(window: &Window) -> (Pixels, Pixels) {
    let v = window.viewport_size();
    (
        (v.width * 0.92).clamp(px(320.), px(820.)),
        (v.height * 0.86).clamp(px(280.), px(600.)),
    )
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn label_of(opt: &str) -> String {
    capitalize(&opt.replace('-', " "))
}

/// One option of a [`segmented`] control.
struct Segment {
    id: SharedString,
    label: String,
    on: bool,
    enabled: bool,
    click: Click,
}

/// Joined options in one recessed track; the selected one is raised.
fn segmented(segments: Vec<Segment>, cx: &App) -> AnyElement {
    let colors = cx.theme();
    let (fg, muted) = (colors.foreground, colors.muted_foreground);
    let (track, raised, focus) = (colors.secondary, colors.background, colors.list_hover);
    let mut row = h_flex()
        .flex_none()
        .p_0p5()
        .gap_0p5()
        .rounded_md()
        .bg(track);
    for seg in segments {
        let click = seg.click;
        let mut el = div()
            .id(seg.id)
            .test_support()
            .px_2p5()
            .py_1()
            .rounded(px(5.))
            .text_xs()
            .child(seg.label);
        if seg.on {
            el = el.bg(raised).text_color(fg).font_weight(FontWeight::MEDIUM);
        } else {
            el = el.text_color(muted);
        }
        if seg.enabled {
            el = el
                .cursor_pointer()
                .tab_stop(true)
                .focus_visible(move |s| s.text_color(fg).bg(focus))
                .hover(move |s| s.text_color(fg))
                .on_click(move |_, window, cx| click(window, cx));
        } else {
            el = el.opacity(0.4);
        }
        row = row.child(el);
    }
    row.into_any_element()
}

/// Square icon-only button with a tooltip.
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
        .tab_stop(true)
        .focus_visible(move |s| s.bg(hover).text_color(fg))
        .text_color(muted)
        .hover(move |s| s.bg(hover).text_color(fg))
        .child(icon(ico, px(15.), muted))
        .tooltip(move |w, cx| gpui_kit::component::tooltip::Tooltip::new(tip).build(w, cx))
        .on_click(move |_, window, cx| click(window, cx))
        .into_any_element()
}

/// Quiet text action: accent-coloured, foreground on hover.
fn link_button(
    id: impl Into<SharedString>,
    label: &'static str,
    cx: &App,
    click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let colors = cx.theme();
    let (accent, fg) = (colors.primary, colors.foreground);
    div()
        .id(id.into())
        .test_support()
        .text_xs()
        .text_color(accent)
        .cursor_pointer()
        .tab_stop(true)
        .focus_visible(move |s| s.text_color(fg).underline())
        .hover(move |s| s.text_color(fg))
        .child(label)
        .on_click(move |_, window, cx| click(window, cx))
        .into_any_element()
}
