//! Window chrome: title bar content, tab strip, sidebar and status bar.
//! Icon-led and quiet: Iconoir glyphs, hairline borders, hover/active tints
//! from the active theme.

use std::path::PathBuf;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;

use crate::icons::{Ico, icon};
use crate::{MekuShell, Mode, NamingMode, theme};

type ClickFn = Box<dyn Fn(&mut Window, &mut App)>;

/// A welcome-screen row: label, icon, shortcut hint and what it does.
type WelcomeAction = (&'static str, Ico, &'static str, ClickFn);

/// A square icon-only button with a tooltip. `active` shows the pressed /
/// selected look.
pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    ico: Ico,
    active: bool,
    tip: &'static str,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let theme = cx.theme();
    let (fg, muted) = (theme.foreground, theme.muted_foreground);
    let hover_bg = theme.list_hover;
    let on_bg = theme.list_active;
    let on_click: ClickFn = Box::new(on_click);
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded_md()
        .cursor_pointer()
        .text_color(if active { fg } else { muted })
        .when_active(active, on_bg)
        .hover(move |style| style.bg(hover_bg).text_color(fg))
        .child(icon(ico, px(16.), if active { fg } else { muted }))
        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        .on_click(move |_, window, cx| on_click(window, cx))
}

trait WhenActive: Sized {
    fn when_active(self, active: bool, color: Hsla) -> Self;
}

impl<T: Styled> WhenActive for T {
    fn when_active(self, active: bool, color: Hsla) -> Self {
        if active { self.bg(color) } else { self }
    }
}

impl Mode {
    fn icon(self) -> Ico {
        match self {
            Mode::Edit => Ico::Source,
            Mode::Live => Ico::Live,
            Mode::Read => Ico::Read,
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Mode::Edit => "Source mode",
            Mode::Live => "Live preview",
            Mode::Read => "Reading view",
        }
    }
}

impl MekuShell {
    // -- title bar ----------------------------------------------------------

    /// Everything between the window edge and the window controls: sidebar
    /// toggle, breadcrumb, then the view-mode switch and quick actions.
    pub(crate) fn render_title_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.entity();
        let sidebar_shell = shell.clone();
        let palette_shell = shell.clone();
        let theme_shell = shell.clone();
        let search_shell = shell.clone();
        let dark = theme::active(cx).dark;

        // Breadcrumb: vault › folders › note.
        let theme_colors = cx.theme();
        let (fg, muted) = (theme_colors.foreground, theme_colors.muted_foreground);
        // Each crumb remembers the folder it stands for (revealed on click).
        let mut crumbs: Vec<(String, bool, Option<PathBuf>)> = Vec::new();
        if self.explorer.is_open() {
            crumbs.push((self.explorer.root_name(), false, Some(PathBuf::new())));
        }
        if let Some(rel) = self.doc_rel() {
            let comps: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            let last = comps.len().saturating_sub(1);
            let mut so_far = PathBuf::new();
            for (i, c) in comps.into_iter().enumerate() {
                so_far.push(&c);
                let name = if i == last {
                    c.trim_end_matches(".md")
                        .trim_end_matches(".markdown")
                        .to_string()
                } else {
                    c
                };
                crumbs.push((name, i == last, (i != last).then(|| so_far.clone())));
            }
        }
        if crumbs.is_empty() {
            crumbs.push(("Meku".to_string(), true, None));
        }
        let hover_bg = theme_colors.list_hover;
        let mut breadcrumb = h_flex().gap_1().items_center().min_w_0().text_sm();
        let total = crumbs.len();
        for (i, (name, current, folder)) in crumbs.into_iter().enumerate() {
            if i > 0 {
                breadcrumb = breadcrumb.child(icon(Ico::ChevronRight, px(12.), muted));
            }
            let mut crumb = div()
                .id(format!("crumb-{i}"))
                .test_support()
                .truncate()
                .max_w(px(if i + 1 == total { 280. } else { 140. }))
                .px_1()
                .rounded_sm()
                .text_color(if current { fg } else { muted })
                .when_weight(current)
                .child(name);
            if let Some(folder) = folder {
                let crumb_shell = shell.clone();
                crumb = crumb
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover_bg).text_color(fg))
                    .on_click(move |_, _, cx| {
                        crumb_shell.update(cx, |v, cx| {
                            // Show the folder in the sidebar tree.
                            v.show_sidebar = true;
                            v.explorer.reveal(&folder.join("x"));
                            cx.notify();
                        });
                    });
            }
            breadcrumb = breadcrumb.child(crumb);
        }

        let back_shell = shell.clone();
        let forward_shell = shell.clone();
        let (can_back, can_forward) = (self.can_nav(-1), self.can_nav(1));
        let nav_buttons = h_flex()
            .gap(px(1.))
            .child(
                div()
                    .opacity(if can_back { 1.0 } else { 0.35 })
                    .child(icon_button(
                        "nav-back",
                        Ico::Back,
                        false,
                        "Back (Alt+←)",
                        cx,
                        move |_, cx| {
                            back_shell.update(cx, |v, cx| v.nav_step(-1, cx));
                        },
                    )),
            )
            .child(
                div()
                    .opacity(if can_forward { 1.0 } else { 0.35 })
                    .child(icon_button(
                        "nav-forward",
                        Ico::Forward,
                        false,
                        "Forward (Alt+→)",
                        cx,
                        move |_, cx| {
                            forward_shell.update(cx, |v, cx| v.nav_step(1, cx));
                        },
                    )),
            );

        let mut right = h_flex().gap_1().items_center().flex_none();
        if self.doc_rel().is_some() {
            right = right.child(self.render_mode_switch_icons(cx));
        }
        right = right
            .child(icon_button(
                "title-search",
                Ico::Search,
                false,
                "Open note (Ctrl+P)",
                cx,
                move |_, cx| {
                    search_shell.update(cx, |v, cx| {
                        v.open_palette(crate::palette::PaletteKind::Files, cx)
                    });
                },
            ))
            .child(icon_button(
                "title-commands",
                Ico::Terminal,
                false,
                "Command palette (Ctrl+Shift+P)",
                cx,
                move |_, cx| {
                    palette_shell.update(cx, |v, cx| {
                        v.open_palette(crate::palette::PaletteKind::Commands, cx)
                    });
                },
            ))
            .child(icon_button(
                "title-theme",
                if dark { Ico::Moon } else { Ico::Sun },
                false,
                "Choose theme",
                cx,
                move |_, cx| {
                    theme_shell.update(cx, |v, cx| {
                        v.open_palette(crate::palette::PaletteKind::Themes, cx)
                    });
                },
            ));

        h_flex()
            .size_full()
            .items_center()
            .justify_between()
            .gap_3()
            .pl_1()
            .pr_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .min_w_0()
                    .child(icon_button(
                        "sidebar-toggle",
                        if self.show_sidebar {
                            Ico::SidebarCollapse
                        } else {
                            Ico::SidebarExpand
                        },
                        false,
                        "Toggle sidebar (Ctrl+\\)",
                        cx,
                        move |_, cx| {
                            sidebar_shell.update(cx, |v, cx| {
                                v.show_sidebar = !v.show_sidebar;
                                cx.notify();
                            });
                        },
                    ))
                    .child(nav_buttons)
                    .child(breadcrumb),
            )
            .child(right)
            .into_any_element()
    }

    /// Source / Live / Read as an icon segmented control.
    fn render_mode_switch_icons(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.tab_mode();
        let shell = cx.entity();
        let theme_colors = cx.theme();
        let track = theme_colors.secondary;
        let (fg, muted) = (theme_colors.foreground, theme_colors.muted_foreground);
        let raised = theme_colors.background;
        let border = theme_colors.border;
        let segments: Vec<AnyElement> = [Mode::Edit, Mode::Live, Mode::Read]
            .into_iter()
            .map(|m| {
                let click_shell = shell.clone();
                let on = m == current;
                let mut seg = div()
                    .id(format!("mode-{m:?}").to_lowercase())
                    .test_support()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded_md()
                    .cursor_pointer()
                    .child(icon(m.icon(), px(15.), if on { fg } else { muted }))
                    .tooltip(move |window, cx| Tooltip::new(m.tip()).build(window, cx))
                    .on_click(move |_, _, cx| {
                        click_shell.update(cx, |view, cx| view.set_tab_mode(m, cx));
                    });
                seg = if on {
                    seg.bg(raised).border_1().border_color(border).shadow_sm()
                } else {
                    seg.hover(move |s| s.text_color(fg))
                };
                seg.into_any_element()
            })
            .collect();
        h_flex()
            .id("mode-switch")
            .flex_none()
            .items_center()
            .gap(px(2.))
            .p(px(2.))
            .rounded_lg()
            .bg(track)
            .children(segments)
            .into_any_element()
    }

    // -- tab strip ------------------------------------------------------------

    pub(crate) fn render_tab_strip(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.entity();
        let theme_colors = cx.theme();
        let (fg, muted) = (theme_colors.foreground, theme_colors.muted_foreground);
        let bar_bg = theme_colors.tab_bar;
        let active_bg = theme_colors.background;
        let border = theme_colors.border;
        let hover_bg = theme_colors.list_hover;
        let accent = theme::accent_alpha(cx, 1.0);

        let tabs: Vec<AnyElement> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let active = index == self.active_tab;
                let dirty = tab.rel.as_ref().is_some_and(|rel| self.is_dirty(rel, cx));
                let select_shell = shell.clone();
                let close_shell = shell.clone();
                let mid_shell = shell.clone();
                let group = SharedString::from(format!("tab-{index}"));
                let title = tab.title.trim_end_matches(".md").to_string();
                let ico = if tab.rel.is_some() {
                    Ico::Note
                } else {
                    Ico::Journal
                };

                // Dirty dot sits where the close icon will appear on hover.
                let close = div()
                    .id(format!("close-tab-{index}"))
                    .test_support()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(18.))
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(muted)
                    .hover(move |s| s.bg(hover_bg).text_color(fg))
                    .child(if dirty {
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .bg(accent)
                            .into_any_element()
                    } else {
                        icon(Ico::Close, px(12.), muted).into_any_element()
                    })
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        close_shell.update(cx, |view, cx| view.close_tab(index, cx));
                    });
                let mut el = h_flex()
                    .id(("tab", index))
                    .test_support()
                    .group(group.clone())
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .h(px(32.))
                    .max_w(px(200.))
                    .pl(px(10.))
                    .pr(px(6.))
                    .rounded_t(px(8.))
                    .cursor_pointer()
                    .text_sm()
                    .on_mouse_down(MouseButton::Middle, move |_, _, cx| {
                        mid_shell.update(cx, |view, cx| view.close_tab(index, cx));
                    })
                    .on_click(move |_, _, cx| {
                        select_shell.update(cx, |view, cx| {
                            view.flush_title(cx);
                            view.active_tab = index;
                            view.save_session();
                            cx.notify();
                        });
                    })
                    .child(icon(ico, px(14.), if active { accent } else { muted }))
                    .child(
                        div()
                            .truncate()
                            .text_color(if active { fg } else { muted })
                            .child(title),
                    )
                    .child(close);
                el = if active {
                    el.bg(active_bg)
                        .border_1()
                        .border_b_0()
                        .border_color(border)
                } else {
                    el.hover(move |s| s.bg(hover_bg))
                };
                let menu_shell = shell.clone();
                let tab_rel = tab.rel.clone();
                el.context_menu(move |menu, _window, _cx| {
                    let mut menu = menu;
                    {
                        let s = menu_shell.clone();
                        menu = menu.item(PopupMenuItem::new("Close").on_click(move |_, _, cx| {
                            s.update(cx, |v, cx| v.close_tab(index, cx));
                        }));
                    }
                    {
                        let s = menu_shell.clone();
                        menu = menu.item(PopupMenuItem::new("Close others").on_click(
                            move |_, _, cx| {
                                s.update(cx, |v, cx| v.close_other_tabs(index, cx));
                            },
                        ));
                    }
                    {
                        let s = menu_shell.clone();
                        menu =
                            menu.item(PopupMenuItem::new("Close all").on_click(move |_, _, cx| {
                                s.update(cx, |v, cx| v.close_all_tabs(cx));
                            }));
                    }
                    if let Some(rel) = tab_rel.clone() {
                        menu = menu.item(PopupMenuItem::separator());
                        let copy_rel = rel.clone();
                        menu =
                            menu.item(PopupMenuItem::new("Copy path").on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    copy_rel.to_string_lossy().to_string(),
                                ));
                            }));
                        let s = menu_shell.clone();
                        menu = menu.item(PopupMenuItem::new("Reveal in sidebar").on_click(
                            move |_, _, cx| {
                                let rel = rel.clone();
                                s.update(cx, |v, cx| {
                                    v.show_sidebar = true;
                                    v.explorer.reveal(&rel);
                                    cx.notify();
                                });
                            },
                        ));
                    }
                    menu
                })
                .into_any_element()
            })
            .collect();

        let new_shell = shell.clone();
        let strip = h_flex()
            .id("tabs-scroll")
            .test_support()
            .flex_none()
            .w_full()
            .min_w_0()
            .items_end()
            .gap(px(2.))
            .pt(px(6.))
            .px(px(8.))
            .bg(bar_bg)
            .overflow_x_scroll()
            .track_scroll(&self.tabs_scroll)
            .children(tabs)
            .child(div().flex_none().pb(px(2.)).pl(px(4.)).child(icon_button(
                "new-tab",
                Ico::Plus,
                false,
                "New note (Ctrl+T)",
                cx,
                move |_, cx| {
                    new_shell.update(cx, |view, cx| view.new_tab_action(cx));
                },
            )));
        // Keep the active tab in view after switching.
        if self.tabs_scrolled_to != Some(self.active_tab) {
            self.tabs_scroll.scroll_to_item(self.active_tab);
            self.tabs_scrolled_to = Some(self.active_tab);
        }
        div()
            .flex_none()
            .w_full()
            .min_w_0()
            .border_b_1()
            .border_color(border)
            .child(strip)
            .into_any_element()
    }

    // -- sidebar ----------------------------------------------------------------

    pub(crate) fn render_sidebar_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme_colors = cx.theme();
        let (fg, muted) = (
            theme_colors.sidebar_foreground,
            theme_colors.muted_foreground,
        );
        let base = v_flex()
            .size_full()
            .bg(theme_colors.sidebar)
            .text_color(fg)
            .border_color(theme_colors.sidebar_border);
        if !self.explorer.is_open() {
            let open_shell = cx.entity();
            return base
                .items_center()
                .justify_center()
                .gap_3()
                .p_4()
                .child(icon(Ico::Folder, px(28.), muted))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("No folder open"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("A mekuto is any folder of markdown notes."),
                )
                .child(
                    gpui_kit::component::button::Button::new("sidebar-open-folder")
                        .label("Open folder")
                        .on_click(move |_, window, cx| {
                            open_shell.update(cx, |v, cx| {
                                v.on_open_folder(&crate::OpenFolder, window, cx)
                            });
                        }),
                )
                .into_any_element();
        }
        let shell = cx.entity();
        let new_note = shell.clone();
        let new_folder = shell.clone();
        let search = shell.clone();
        let hover_bg = cx.theme().list_hover;
        let field_bg = cx.theme().secondary;
        let border = cx.theme().sidebar_border;

        let header = h_flex()
            .px_3()
            .pt_3()
            .pb_1()
            .gap_2()
            .items_center()
            .justify_between()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .min_w_0()
                    .child(icon(Ico::Journal, px(17.), theme::accent_alpha(cx, 1.0)))
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.explorer.root_name()),
                    ),
            )
            .child(
                h_flex()
                    .gap(px(2.))
                    .child(icon_button(
                        "new-note",
                        Ico::NotePlus,
                        false,
                        "New note",
                        cx,
                        move |_, cx| {
                            new_note.update(cx, |view, cx| {
                                view.begin_naming(NamingMode::NewNote, PathBuf::new(), None, cx)
                            });
                        },
                    ))
                    .child(icon_button(
                        "new-folder",
                        Ico::FolderPlus,
                        false,
                        "New folder",
                        cx,
                        move |_, cx| {
                            new_folder.update(cx, |view, cx| {
                                view.begin_naming(NamingMode::NewFolder, PathBuf::new(), None, cx)
                            });
                        },
                    )),
            );

        // A search field that is really a button into the file finder.
        let search_field = div().px_3().py_1().child(
            h_flex()
                .id("sidebar-search")
                .test_support()
                .h(px(30.))
                .px_2()
                .gap_2()
                .items_center()
                .justify_between()
                .rounded_md()
                .bg(field_bg)
                .cursor_pointer()
                .text_sm()
                .text_color(muted)
                .hover(move |s| s.bg(hover_bg))
                .on_click(move |_, _, cx| {
                    search.update(cx, |v, cx| {
                        v.open_palette(crate::palette::PaletteKind::Files, cx)
                    });
                })
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(icon(Ico::Search, px(15.), muted))
                        .child("Search"),
                )
                .child(div().text_xs().child("Ctrl P")),
        );

        let footer_shell = cx.entity();
        let theme_shell = footer_shell.clone();
        let cmd_shell = footer_shell.clone();
        let dark = theme::active(cx).dark;
        let footer = h_flex()
            .flex_none()
            .px_2()
            .py_1p5()
            .gap_1()
            .items_center()
            .border_t_1()
            .border_color(border)
            .child(icon_button(
                "footer-theme",
                if dark { Ico::Moon } else { Ico::Sun },
                false,
                "Choose theme",
                cx,
                move |_, cx| {
                    theme_shell.update(cx, |v, cx| {
                        v.open_palette(crate::palette::PaletteKind::Themes, cx)
                    });
                },
            ))
            .child(icon_button(
                "footer-commands",
                Ico::Terminal,
                false,
                "Command palette",
                cx,
                move |_, cx| {
                    cmd_shell.update(cx, |v, cx| {
                        v.open_palette(crate::palette::PaletteKind::Commands, cx)
                    });
                },
            ))
            .child(div().flex_1())
            .child(div().text_xs().text_color(muted).child("Meku"));

        base.child(header)
            .child(search_field)
            .child(self.section_label("Notes", Ico::Notes, None, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .id("file-tree-scroll")
                    .test_support()
                    .overflow_y_scroll()
                    .px_1()
                    .child(self.render_tree_rows(cx)),
            )
            .child(self.render_outline_panel(cx))
            .child(footer)
            .into_any_element()
    }

    /// Small section heading with an icon; `toggle` makes it collapsible.
    fn section_label(
        &self,
        label: &'static str,
        ico: Ico,
        toggle: Option<(bool, Entity<MekuShell>)>,
        cx: &App,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let fg = cx.theme().foreground;
        let mut row = h_flex()
            .id(format!("section-{label}"))
            .test_support()
            .px_3()
            .pt_2()
            .pb_1()
            .gap_1p5()
            .items_center()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(muted);
        if let Some((open, shell)) = toggle {
            row = row
                .cursor_pointer()
                .hover(move |s| s.text_color(fg))
                .child(icon(
                    if open {
                        Ico::ChevronDown
                    } else {
                        Ico::ChevronRight
                    },
                    px(11.),
                    muted,
                ))
                .on_click(move |_, _, cx| {
                    shell.update(cx, |v, cx| {
                        v.outline_open = !v.outline_open;
                        cx.notify();
                    });
                });
        }
        row.child(icon(ico, px(13.), muted))
            .child(label)
            .into_any_element()
    }

    /// File tree rows: chevron + folder/note icon + name, hover/selected tints.
    fn render_tree_rows(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shell = cx.entity();
        let active_rel = self.tabs.get(self.active_tab).and_then(|t| t.rel.clone());
        let theme_colors = cx.theme();
        let (fg, muted) = (
            theme_colors.sidebar_foreground,
            theme_colors.muted_foreground,
        );
        let hover_bg = theme_colors.list_hover;
        let active_bg = theme_colors.list_active;
        let accent = theme::accent_alpha(cx, 1.0);
        let rows: Vec<AnyElement> = self
            .explorer
            .flat_rows()
            .into_iter()
            .map(|row| {
                let open_shell = shell.clone();
                let menu_shell = shell.clone();
                let open_rel = row.rel.clone();
                let menu_rel = row.rel.clone();
                let selected = active_rel.as_ref() == Some(&row.rel);
                let label = if row.is_dir {
                    row.label.clone()
                } else {
                    row.label
                        .trim_end_matches(".markdown")
                        .trim_end_matches(".md")
                        .to_string()
                };
                let lead = if row.is_dir {
                    icon(
                        if row.expanded {
                            Ico::ChevronDown
                        } else {
                            Ico::ChevronRight
                        },
                        px(12.),
                        muted,
                    )
                } else {
                    // Notes align with folder names: an empty chevron slot.
                    icon(Ico::ChevronRight, px(12.), gpui_kit::transparent_black())
                };
                let mut el = h_flex()
                    .id(format!("tree-row-{}", row.rel.display()))
                    .test_support()
                    .h(px(28.))
                    .pl(px(row.depth as f32 * 14.0 + 6.0))
                    .pr_2()
                    .gap_1p5()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(fg)
                    .child(lead)
                    .child(icon(
                        if row.is_dir { Ico::Folder } else { Ico::Note },
                        px(15.),
                        if selected { accent } else { muted },
                    ))
                    .child(div().truncate().child(label))
                    .on_click(move |_, _, cx| {
                        open_shell.update(cx, |view, cx| view.open_file(&open_rel, cx));
                    });
                el = if selected {
                    el.bg(active_bg)
                } else {
                    el.hover(move |s| s.bg(hover_bg))
                };
                el.context_menu(move |menu, _window, cx| {
                    let is_dir = menu_shell.read(cx).explorer.is_dir(&menu_rel);
                    let mut menu = menu;
                    if is_dir {
                        for (label, mode) in [
                            ("New note", NamingMode::NewNote),
                            ("New folder", NamingMode::NewFolder),
                        ] {
                            let s = menu_shell.clone();
                            let dir = menu_rel.clone();
                            menu =
                                menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                                    s.update(cx, |view, cx| {
                                        view.begin_naming(mode, dir.clone(), None, cx)
                                    });
                                }));
                        }
                        menu = menu.item(PopupMenuItem::separator());
                    }
                    {
                        let s = menu_shell.clone();
                        let target = menu_rel.clone();
                        menu = menu.item(PopupMenuItem::new("Rename").on_click(move |_, _, cx| {
                            s.update(cx, |view, cx| {
                                view.begin_naming(
                                    NamingMode::Rename,
                                    target.clone(),
                                    Some(target.clone()),
                                    cx,
                                )
                            });
                        }));
                    }
                    {
                        let s = menu_shell.clone();
                        let target = menu_rel.clone();
                        menu = menu.item(PopupMenuItem::new("Delete").on_click(move |_, _, cx| {
                            s.update(cx, |view, cx| view.ask_delete(target.clone(), cx));
                        }));
                    }
                    menu
                })
                .into_any_element()
            })
            .collect();
        v_flex().gap(px(1.)).children(rows).into_any_element()
    }

    /// Collapsible heading tree of the active note (follows the caret).
    fn render_outline_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let outline = self.doc_outline(cx);
        let border = cx.theme().sidebar_border;
        let open = self.outline_open;
        let header = self.section_label("Outline", Ico::Outline, Some((open, cx.entity())), cx);
        let mut panel = v_flex()
            .flex_none()
            .border_t_1()
            .border_color(border)
            .child(header);
        if open {
            let caret = self
                .doc_rel()
                .and_then(|rel| self.buffers.get(&rel))
                .map_or(0, |b| b.read(cx).caret());
            let active = outline.iter().rposition(|(_, _, at)| *at <= caret);
            let shell = cx.entity();
            let theme_colors = cx.theme();
            let (fg, muted) = (
                theme_colors.sidebar_foreground,
                theme_colors.muted_foreground,
            );
            let hover_bg = theme_colors.list_hover;
            let active_bg = theme_colors.list_active;
            let empty = outline.is_empty();
            let rows: Vec<AnyElement> = outline
                .into_iter()
                .enumerate()
                .map(|(i, (level, title, at))| {
                    let shell = shell.clone();
                    let on = active == Some(i);
                    let mut el = h_flex()
                        .id(format!("outline-{i}"))
                        .test_support()
                        .h(px(26.))
                        .pl(px(f32::from(level.saturating_sub(1)) * 12.0 + 22.0))
                        .pr_2()
                        .items_center()
                        .rounded_md()
                        .cursor_pointer()
                        .text_sm()
                        .text_color(if on { fg } else { muted })
                        .child(div().truncate().child(if title.is_empty() {
                            "(untitled)".to_string()
                        } else {
                            title
                        }))
                        .on_click(move |_, window, cx| {
                            shell.update(cx, |view, cx| {
                                view.doc_goto(at, cx);
                                window.focus(&view.focus_handle, cx);
                            });
                        });
                    el = if on {
                        el.bg(active_bg)
                    } else {
                        el.hover(move |s| s.bg(hover_bg).text_color(fg))
                    };
                    el.into_any_element()
                })
                .collect();
            panel = panel.child(
                div()
                    .id("outline-scroll")
                    .max_h(px(200.))
                    .overflow_y_scroll()
                    .px_1()
                    .pb_1()
                    .children(rows)
                    .when_empty(empty, muted),
            );
        }
        panel.into_any_element()
    }

    // -- empty state -----------------------------------------------------------

    /// What the editor area shows with no note open: the next useful actions.
    pub(crate) fn render_welcome(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme_colors = cx.theme();
        let (fg, muted) = (theme_colors.foreground, theme_colors.muted_foreground);
        let hover = theme_colors.list_hover;
        let open = self.explorer.is_open();
        let shell = cx.entity();
        let mut actions: Vec<WelcomeAction> = Vec::new();
        if open {
            let s = shell.clone();
            actions.push((
                "New note",
                Ico::NotePlus,
                "Ctrl T",
                Box::new(move |_, cx| {
                    s.update(cx, |v, cx| v.new_tab_action(cx));
                }),
            ));
            let s = shell.clone();
            actions.push((
                "Open a note",
                Ico::Search,
                "Ctrl P",
                Box::new(move |_, cx| {
                    s.update(cx, |v, cx| {
                        v.open_palette(crate::palette::PaletteKind::Files, cx)
                    });
                }),
            ));
        } else {
            let s = shell.clone();
            actions.push((
                "Open a folder",
                Ico::Folder,
                "Ctrl O",
                Box::new(move |window, cx| {
                    s.update(cx, |v, cx| v.on_open_folder(&crate::OpenFolder, window, cx));
                }),
            ));
        }
        let s = shell.clone();
        actions.push((
            "Command palette",
            Ico::Terminal,
            "Ctrl Shift P",
            Box::new(move |_, cx| {
                s.update(cx, |v, cx| {
                    v.open_palette(crate::palette::PaletteKind::Commands, cx)
                });
            }),
        ));
        let rows: Vec<AnyElement> = actions
            .into_iter()
            .enumerate()
            .map(|(i, (label, ico, hint, on))| {
                h_flex()
                    .id(("welcome-action", i))
                    .test_support()
                    .w(px(300.))
                    .h(px(38.))
                    .px_3()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .rounded_lg()
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(icon(ico, px(18.), muted))
                            .child(div().text_sm().text_color(fg).child(label)),
                    )
                    .child(div().text_xs().text_color(muted).child(hint))
                    .on_click(move |_, window, cx| on(window, cx))
                    .into_any_element()
            })
            .collect();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(icon(Ico::Journal, px(44.), theme::accent_alpha(cx, 0.9)))
            .child(
                v_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child("Meku"),
                    )
                    .child(div().text_sm().text_color(muted).child(if open {
                        "Pick a note from the sidebar, or start a new one."
                    } else {
                        "Notes are plain markdown files in a folder."
                    })),
            )
            .child(v_flex().gap_1().children(rows))
            .into_any_element()
    }

    // -- status bar --------------------------------------------------------------

    pub(crate) fn render_status(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme_colors = cx.theme();
        let (fg, muted) = (theme_colors.foreground, theme_colors.muted_foreground);
        let bar = theme_colors.status_bar;
        let border = theme_colors.status_bar_border;
        let success = theme_colors.success;

        let (words, line, col, dirty) = match self.doc_rel() {
            Some(rel) => {
                let words = self.doc_model(&rel, cx).map_or(0, |m| m.words);
                let (buffer, dirty) = self
                    .buffers
                    .get(&rel)
                    .map(|b| (b.clone(), b.read(cx).dirty))
                    .map_or((None, false), |(b, d)| (Some(b), d));
                let (line, col) = buffer.map_or((1, 1), |b| {
                    let b = b.read(cx);
                    b.line_col(b.caret())
                });
                (Some(words), line, col, dirty)
            }
            None => (None, 1, 1, false),
        };

        let mut left = h_flex().gap_3().items_center();
        left = if let Some(notice) = &self.notice {
            left.child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .text_color(fg)
                    .child(icon(Ico::Info, px(13.), muted))
                    .child(notice.clone()),
            )
        } else if words.is_some() {
            left.child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(if dirty {
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .bg(muted)
                            .into_any_element()
                    } else {
                        icon(Ico::CheckCircle, px(13.), success).into_any_element()
                    })
                    .child(if dirty { "Unsaved" } else { "Saved" }),
            )
        } else {
            left.child("Ready")
        };

        let mut right = h_flex().gap_4().items_center();
        if let Some(words) = words {
            let hover = cx.theme().list_hover;
            let goto_shell = cx.entity();
            let mode_shell = cx.entity();
            right = right
                .child(format!(
                    "{words} {}",
                    if words == 1 { "word" } else { "words" }
                ))
                .child(
                    div()
                        .id("status-position")
                        .test_support()
                        .px_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover).text_color(fg))
                        .child(format!("Ln {line}, Col {col}"))
                        .on_click(move |_, _, cx| {
                            goto_shell.update(cx, |v, cx| {
                                v.open_palette(crate::palette::PaletteKind::GotoLine, cx)
                            });
                        }),
                )
                .child(
                    h_flex()
                        .id("status-mode")
                        .test_support()
                        .gap_1p5()
                        .px_1()
                        .items_center()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover).text_color(fg))
                        .child(icon(self.tab_mode().icon(), px(13.), muted))
                        .child(self.tab_mode().label())
                        .on_click(move |_, _, cx| {
                            mode_shell.update(cx, |v, cx| {
                                if let Some(tab) = v.tabs.get_mut(v.active_tab) {
                                    tab.mode = tab.mode.next();
                                }
                                cx.notify();
                            });
                        }),
                );
        }
        h_flex()
            .id("status-bar")
            .test_support()
            .flex_none()
            .w_full()
            .h(px(26.))
            .px_3()
            .items_center()
            .justify_between()
            .text_xs()
            .text_color(muted)
            .bg(bar)
            .border_t_1()
            .border_color(border)
            .child(left)
            .child(right)
            .into_any_element()
    }
}

trait WhenWeight: Sized {
    fn when_weight(self, bold: bool) -> Self;
}

impl<T: Styled> WhenWeight for T {
    fn when_weight(self, bold: bool) -> Self {
        if bold {
            self.font_weight(FontWeight::MEDIUM)
        } else {
            self
        }
    }
}

trait WhenEmpty: Sized {
    fn when_empty(self, empty: bool, muted: Hsla) -> AnyElement;
}

impl WhenEmpty for Stateful<Div> {
    /// A hint line instead of nothing when the note has no headings.
    fn when_empty(self, empty: bool, muted: Hsla) -> AnyElement {
        if empty {
            self.child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(muted)
                    .child("Headings appear here"),
            )
            .into_any_element()
        } else {
            self.into_any_element()
        }
    }
}
