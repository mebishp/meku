//! Live-preview rendering: [`StyledDoc`](meku_markdown) blocks become GPUI
//! elements. Markup characters never reach the display: only recorded span
//! ranges are concatenated, so `#`, `**`, `](url)` and friends stay hidden.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    AbsoluteLength, AnyElement, App, FontStyle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, StatefulInteractiveElement, StrikethroughStyle, Styled, StyledText, TextStyle,
    UnderlineStyle, Window, div, px,
};
use meku_markdown::{BlockKind, InlineStyle, StyledBlock};

/// Rendered text of a block: the concatenation of its span slices.
/// Anything outside spans (markup, link targets, …) is display-excluded.
/// Used by UI tests to assert on visible content without pixels.
#[cfg(test)]
pub fn display_text(block: &StyledBlock, source: &str) -> String {
    let mut out = String::new();
    for span in &block.spans {
        if let Some(slice) = source.get(span.start..span.end) {
            out.push_str(slice);
        }
    }
    out
}

fn scale_font(style: &mut TextStyle, factor: f32) {
    style.font_size = match style.font_size {
        AbsoluteLength::Pixels(px) => AbsoluteLength::Pixels(px * factor),
        AbsoluteLength::Rems(r) => AbsoluteLength::Rems(r * factor),
    };
}

fn apply_inline(style: &mut TextStyle, bits: InlineStyle, cx: &App) {
    if bits.contains(InlineStyle::STRONG) {
        style.font_weight = FontWeight::BOLD;
    }
    if bits.contains(InlineStyle::EMPHASIS) {
        style.font_style = FontStyle::Italic;
    }
    if bits.contains(InlineStyle::STRIKE) {
        style.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.),
            color: None,
        });
    }
    if bits.contains(InlineStyle::CODE) {
        style.color = cx.theme().accent;
    }
    if bits.contains(InlineStyle::LINK) {
        style.color = cx.theme().link;
        style.underline = Some(UnderlineStyle {
            thickness: px(1.),
            color: None,
            wavy: false,
        });
    }
}

fn styled_text(block: &StyledBlock, source: &str, base: &TextStyle, cx: &App) -> StyledText {
    let mut text = String::new();
    let mut runs = Vec::with_capacity(block.spans.len());
    for span in &block.spans {
        let Some(slice) = source.get(span.start..span.end) else {
            continue;
        };
        if slice.is_empty() {
            continue;
        }
        let mut style = base.clone();
        apply_inline(&mut style, span.style, cx);
        text.push_str(slice);
        runs.push(style.to_run(slice.len()));
    }
    StyledText::new(text).with_runs(runs)
}

fn block_base_style(kind: &BlockKind, window: &mut Window) -> TextStyle {
    let mut base = window.text_style().clone();
    let (scale, bold) = match kind {
        BlockKind::Heading(1) => (1.6, true),
        BlockKind::Heading(2) => (1.4, true),
        BlockKind::Heading(3) => (1.2, true),
        BlockKind::Heading(_) => (1.05, true),
        _ => (1.0, false),
    };
    if scale != 1.0 {
        scale_font(&mut base, scale);
    }
    if bold {
        base.font_weight = FontWeight::BOLD;
    }
    base
}

/// Raw monospace-ish block (code fences, tables): the source slice verbatim,
/// since cell/item structure needs no span mapping to stay readable.
fn render_raw_block(
    block: &StyledBlock,
    source: &str,
    caption: Option<String>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let text = source
        .get(block.span.start..block.span.end)
        .unwrap_or_default();
    let mut column = div()
        .rounded_md()
        .bg(cx.theme().sidebar)
        .p_2()
        .text_xs()
        .child(text.to_string());
    if let Some(caption) = caption {
        column = column.child(
            div()
                .pb_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(caption),
        );
    }
    let _ = window;
    column.into_any_element()
}

/// Render one non-active block. `on_activate` fires on click; the shell
/// swaps the block for a raw editor (cursor starts at 0, a second click
/// positions natively — exact click-to-offset mapping is polish work).
pub fn render_styled_block(
    block: &StyledBlock,
    source: &str,
    index: usize,
    on_activate: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    if block.kind == BlockKind::Hr {
        return div()
            .id(format!("md-block-{index}"))
            .test_support()
            .on_click(on_activate)
            .py_2()
            .child(div().h(px(1.)).bg(cx.theme().border))
            .into_any_element();
    }
    if let BlockKind::CodeFence(lang) = &block.kind {
        let caption = lang.clone().filter(|l| !l.is_empty());
        return div()
            .id(format!("md-block-{index}"))
            .test_support()
            .on_click(on_activate)
            .mb_2()
            .child(render_raw_block(block, source, caption, window, cx))
            .into_any_element();
    }
    if block.kind == BlockKind::Table {
        return div()
            .id(format!("md-block-{index}"))
            .test_support()
            .on_click(on_activate)
            .mb_2()
            .child(render_raw_block(block, source, None, window, cx))
            .into_any_element();
    }

    let base = block_base_style(&block.kind, window);
    let content = styled_text(block, source, &base, cx);
    let content: AnyElement = if block.kind == BlockKind::Quote {
        div()
            .border_l_2()
            .border_color(cx.theme().accent)
            .pl_3()
            .child(content)
            .into_any_element()
    } else {
        content.into_any_element()
    };
    div()
        .id(format!("md-block-{index}"))
        .test_support()
        .on_click(on_activate)
        .mb_2()
        .child(content)
        .into_any_element()
}
