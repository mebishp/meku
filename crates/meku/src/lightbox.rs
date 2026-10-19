//! Full-window zoom view for images: click a picture in a note to open it,
//! click it again (or use the buttons) to zoom, click outside or press Esc to
//! close.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme, h_flex};
use gpui_kit::*;

use crate::MekuShell;
use crate::document::{ImageInfo, picture};
use crate::icons::{Ico, icon};

type Action = Box<dyn Fn(&mut Window, &mut App)>;

/// Zoom steps relative to the image's real pixel size.
const STEPS: [f32; 7] = [0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0];

impl MekuShell {
    pub(crate) fn open_lightbox(&mut self, info: ImageInfo, cx: &mut Context<Self>) {
        self.lightbox = Some(info);
        self.lightbox_zoom = None;
        cx.notify();
    }

    pub(crate) fn close_lightbox(&mut self, cx: &mut Context<Self>) -> bool {
        let was_open = self.lightbox.take().is_some();
        if was_open {
            cx.notify();
        }
        was_open
    }

    /// Scale that makes the image fit the window area (never past 3x).
    fn lightbox_fit(&self, window: &Window) -> Option<f32> {
        let (w, h) = self.lightbox.as_ref()?.dims?;
        let view = window.viewport_size();
        let aw = (f32::from(view.width) - 96.0).max(100.0);
        let ah = (f32::from(view.height) - 120.0).max(100.0);
        Some((aw / w as f32).min(ah / h as f32).min(3.0))
    }

    /// Step the zoom up (`true`) or down through [`STEPS`]; the Fit button
    /// returns to fit-to-window.
    pub(crate) fn lightbox_step(&mut self, up: bool, window: &Window, cx: &mut Context<Self>) {
        let Some(fit) = self.lightbox_fit(window) else {
            return;
        };
        let current = self.lightbox_zoom.unwrap_or(fit);
        self.lightbox_zoom = if up {
            STEPS
                .iter()
                .copied()
                .find(|s| *s > current + 0.001)
                .or(STEPS.last().copied())
        } else {
            STEPS
                .iter()
                .rev()
                .copied()
                .find(|s| *s < current - 0.001)
                .or(STEPS.first().copied())
        };
        cx.notify();
    }

    /// Click on the picture: fit ⇄ actual size (or 2x for small images).
    fn lightbox_toggle(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(fit) = self.lightbox_fit(window) else {
            return;
        };
        self.lightbox_zoom = match self.lightbox_zoom {
            Some(_) => None,
            None if fit < 0.99 => Some(1.0),
            None => Some((fit * 2.0).min(4.0)),
        };
        cx.notify();
    }

    pub(crate) fn render_lightbox(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let info = self.lightbox.clone()?;
        let shell = cx.entity();
        let fit = self.lightbox_fit(window);
        let zoom = self.lightbox_zoom;
        let shown_scale = zoom.or(fit);
        let view = window.viewport_size();

        // Picture size: scaled real size when known, else the whole area.
        let (pw, ph) = match (info.dims, shown_scale) {
            (Some((w, h)), Some(sc)) => (px(w as f32 * sc), px(h as f32 * sc)),
            _ => (
                px((f32::from(view.width) - 96.0).max(100.0)),
                px((f32::from(view.height) - 120.0).max(100.0)),
            ),
        };

        let light = gpui_kit::hsla(0., 0., 1., 0.92);
        let dim = gpui_kit::hsla(0., 0., 1., 0.6);
        let button = |id: &'static str, ico: Ico, tip: &'static str, on: Action| {
            div()
                .id(id)
                .test_support()
                .flex()
                .items_center()
                .justify_center()
                .size(px(32.))
                .rounded_md()
                .cursor_pointer()
                .text_color(dim)
                .hover(move |s| s.bg(gpui_kit::hsla(0., 0., 1., 0.14)).text_color(light))
                .child(icon(ico, px(18.), dim))
                .tooltip(move |w, cx| gpui_kit::component::tooltip::Tooltip::new(tip).build(w, cx))
                .on_click(move |_, window, cx| on(window, cx))
        };
        let (zi, zo, zf, zc) = (shell.clone(), shell.clone(), shell.clone(), shell.clone());
        let percent = shown_scale.map(|s| format!("{}%", (s * 100.0).round() as u32));
        let mut controls = h_flex().gap_1().items_center();
        if let Some(p) = percent {
            controls = controls.child(div().text_xs().text_color(dim).px_2().child(p));
        }
        let bar =
            h_flex()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(52.))
                .px_4()
                .gap_2()
                .items_center()
                .justify_between()
                .child(div().truncate().text_sm().text_color(light).child(
                    if info.caption.is_empty() {
                        match &info.src {
                            crate::document::ImageSrc::Path(p) => p
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                            crate::document::ImageSrc::Url(u) => u.clone(),
                        }
                    } else {
                        info.caption.clone()
                    },
                ))
                .child(
                    controls
                        .child(button(
                            "lightbox-out",
                            Ico::ZoomOut,
                            "Zoom out",
                            Box::new(move |w, cx| {
                                zo.update(cx, |v, cx| v.lightbox_step(false, w, cx));
                            }),
                        ))
                        .child(button(
                            "lightbox-in",
                            Ico::ZoomIn,
                            "Zoom in",
                            Box::new(move |w, cx| {
                                zi.update(cx, |v, cx| v.lightbox_step(true, w, cx));
                            }),
                        ))
                        .child(button(
                            "lightbox-fit",
                            Ico::Maximize,
                            "Fit to window",
                            Box::new(move |_, cx| {
                                zf.update(cx, |v, cx| {
                                    v.lightbox_zoom = None;
                                    cx.notify();
                                });
                            }),
                        ))
                        .child(button(
                            "lightbox-close",
                            Ico::Close,
                            "Close (Esc)",
                            Box::new(move |_, cx| {
                                zc.update(cx, |v, cx| {
                                    v.close_lightbox(cx);
                                });
                            }),
                        )),
                );

        let toggle_shell = shell.clone();
        let close_shell = shell.clone();
        let stage = div()
            .id("lightbox-scroll")
            .absolute()
            .top(px(52.))
            .bottom(px(16.))
            .left(px(16.))
            .right(px(16.))
            .overflow_scroll()
            // Anywhere outside the picture closes the view.
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                close_shell.update(cx, |v, cx| {
                    v.close_lightbox(cx);
                });
            })
            .child(
                div()
                    .min_w_full()
                    .min_h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .id("lightbox-image")
                            .test_support()
                            .w(pw)
                            .h(ph)
                            .flex_none()
                            .rounded_md()
                            .overflow_hidden()
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                cx.stop_propagation();
                                toggle_shell.update(cx, |v, cx| v.lightbox_toggle(window, cx));
                            })
                            .child(picture(&info.src, cx)),
                    ),
            );
        let root = div()
            .id("lightbox")
            .test_support()
            .absolute()
            .inset_0()
            .bg(gpui_kit::hsla(0., 0., 0.03, 0.9))
            // Block everything beneath the view.
            .occlude()
            .child(stage)
            .child(bar);
        let _ = cx.theme();
        Some(crate::theme::fade_in(root, "lightbox-in", 160, 0.0, cx))
    }
}
