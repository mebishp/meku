use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::sidebar::{
    Sidebar, SidebarFooter, SidebarHeader, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::TitleBar;
use gpui_kit::component::*;
use gpui_kit::*;

actions!(meku, [ToggleSidebar, NewTab, CloseTab, NextTab]);

pub struct MekuShell {
    focus_handle: FocusHandle,
    show_sidebar: bool,
    tabs: Vec<String>,
    active_tab: usize,
}

impl MekuShell {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            show_sidebar: true,
            tabs: vec!["Welcome".to_string()],
            active_tab: 0,
        }
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.show_sidebar = !self.show_sidebar;
        cx.notify();
    }

    fn new_tab(&mut self, _: &NewTab, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len() + 1;
        self.tabs.push(format!("Untitled-{n}"));
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
    }

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.tabs.push("Welcome".to_string());
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        cx.notify();
    }

    fn on_close_tab(&mut self, _: &CloseTab, _: &mut Window, cx: &mut Context<Self>) {
        self.close_tab(self.active_tab, cx);
    }

    fn on_next_tab(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
            cx.notify();
        }
    }

    fn render_sidebar(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
        Sidebar::new("meku-sidebar")
            .header(SidebarHeader::new().child("No mekuto open"))
            .child(
                SidebarMenu::new()
                    .child(SidebarMenuItem::new("Explorer"))
                    .child(SidebarMenuItem::new("Search"))
                    .child(SidebarMenuItem::new("Outline")),
            )
            .footer(SidebarFooter::new().child("Meku v0.1"))
    }

    fn render_center(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let bar_entity = entity.clone();
        let tab_bar = TabBar::new("meku-tabs")
            .selected_index(self.active_tab)
            .on_click(move |index: &usize, _window, cx| {
                let index = *index;
                bar_entity.update(cx, |view, cx| {
                    view.active_tab = index;
                    cx.notify();
                });
            })
            .children(self.tabs.iter().enumerate().map(|(index, name)| {
                let entity = entity.clone();
                Tab::new().label(name.clone()).suffix(
                    Button::new(format!("close-tab-{index}"))
                        .label("×")
                        .ghost()
                        .xsmall()
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |view, cx| {
                                view.close_tab(index, cx);
                            });
                        }),
                )
            }));

        let active = self.tabs.get(self.active_tab).cloned().unwrap_or_default();
        let empty_state = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(cx.theme().muted_foreground)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(active),
            )
            .child("No file open — file tree and editor land in Build 2/3.");

        v_flex()
            .size_full()
            .child(tab_bar)
            .child(div().flex_1().child(empty_state).into_any_element())
    }

    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .h(px(28.))
            .px_2()
            .gap_3()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("○ No mekuto open")
            .child(h_flex().gap_3().child("Markdown").child("Ln 1, Col 1"))
    }
}

impl Render for MekuShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.focus_handle.clone();
        div()
            .size_full()
            .track_focus(&focus)
            .key_context("Meku")
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .child(TitleBar::new().child("Meku — no mekuto open"))
            .child(
                div().flex_1().child(
                    h_resizable("meku-shell")
                        .child(
                            resizable_panel()
                                .visible(self.show_sidebar)
                                .size(px(240.))
                                .size_range(px(200.)..px(400.))
                                .child(self.render_sidebar(cx).into_any_element()),
                        )
                        .child(self.render_center(cx).into_any_element()),
                ),
            )
            .child(self.render_status_bar(cx))
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(move |cx| {
        // Required: initializes gpui-component theming/global state.
        gpui_kit::init(cx);

        cx.bind_keys([
            KeyBinding::new("ctrl-b", ToggleSidebar, Some("Meku")),
            KeyBinding::new("ctrl-t", NewTab, Some("Meku")),
            KeyBinding::new("ctrl-w", CloseTab, Some("Meku")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Meku")),
        ]);

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        cx.spawn(async move |cx| {
            let mut options = TitleBar::window_options();
            options.window_bounds = Some(WindowBounds::Windowed(bounds));
            options.app_id = Some("meku".to_string());
            cx.open_window(options, |window, cx| {
                let view = cx.new(MekuShell::new);
                // First level on the window must be a Root.
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open meku window");
        })
        .detach();
    });
}
