//! Theme presets and their persistence.
//!
//! A [`Palette`] is the whole look of a preset: surface/ink/accent colors plus
//! the code-block syntax colors. [`apply`] turns it into a kit `ThemeConfig`
//! (so every kit component follows) and publishes it as the [`Active`] global
//! (so the editor's own syntax colors follow too).

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use crate::config::{Config, ConfigState, Scope, ThemeSpec};
use gpui_kit::component::{Theme, ThemeConfig, ThemeRegistry};
use gpui_kit::{App, Global, WindowAppearance};

/// One complete preset. Colors are `0xRRGGBB`.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub name: &'static str,
    pub dark: bool,
    pub bg: u32,
    pub surface: u32,
    pub border: u32,
    pub fg: u32,
    pub muted: u32,
    pub accent: u32,
    pub keyword: u32,
    pub string: u32,
    pub number: u32,
    pub func: u32,
    pub ty: u32,
    pub constant: u32,
    pub prop: u32,
    pub comment: u32,
    /// Inline `code` text.
    pub code: u32,
}

pub const MEKU_LIGHT: Palette = Palette {
    name: "Meku Light",
    dark: false,
    bg: 0xfcfcfd,
    surface: 0xf3f4f6,
    border: 0xe2e4e9,
    fg: 0x1c1f26,
    muted: 0x666d78,
    accent: 0x3d63dd,
    keyword: 0x8250df,
    string: 0x1a7f37,
    number: 0xb35900,
    func: 0x0969da,
    ty: 0x953800,
    constant: 0xbf3989,
    prop: 0x0a7c8c,
    comment: 0x6a737d,
    code: 0x9a3412,
};

pub const MEKU_DARK: Palette = Palette {
    name: "Meku Dark",
    dark: true,
    bg: 0x16181d,
    surface: 0x1c1f25,
    border: 0x2b2f37,
    fg: 0xe4e6ea,
    muted: 0x9298a3,
    accent: 0x7c9cff,
    keyword: 0xc792ea,
    string: 0xa5d68b,
    number: 0xf2a65a,
    func: 0x7cb7ff,
    ty: 0xf0cf8c,
    constant: 0xf28fad,
    prop: 0x7fd6cb,
    comment: 0x7a8190,
    code: 0xf2a88a,
};

pub const SOLARIZED_LIGHT: Palette = Palette {
    name: "Solarized Light",
    dark: false,
    bg: 0xfdf6e3,
    surface: 0xeee8d5,
    border: 0xddd6c1,
    fg: 0x073642,
    muted: 0x586e75,
    accent: 0x268bd2,
    keyword: 0x6b7a00,
    string: 0x1f8a82,
    number: 0xb5400f,
    func: 0x1f77b4,
    ty: 0xb58900,
    constant: 0xd33682,
    prop: 0x6c71c4,
    comment: 0x7a8b8f,
    code: 0xa83a0c,
};

pub const SOLARIZED_DARK: Palette = Palette {
    name: "Solarized Dark",
    dark: true,
    bg: 0x002b36,
    surface: 0x073642,
    border: 0x16505f,
    fg: 0xa9b6b6,
    muted: 0x839496,
    accent: 0x2f9fe0,
    keyword: 0xa4b800,
    string: 0x35b8ae,
    number: 0xdb6a3a,
    func: 0x2f9fe0,
    ty: 0xd4a017,
    constant: 0xe05a9c,
    prop: 0x8a90dc,
    comment: 0x6b8890,
    code: 0xe2784a,
};

pub const NORD: Palette = Palette {
    name: "Nord",
    dark: true,
    bg: 0x2e3440,
    surface: 0x3b4252,
    border: 0x4c566a,
    fg: 0xeceff4,
    muted: 0xa7b1c2,
    accent: 0x88c0d0,
    keyword: 0x81a1c1,
    string: 0xa3be8c,
    number: 0xb48ead,
    func: 0x88c0d0,
    ty: 0xebcb8b,
    constant: 0xd08770,
    prop: 0x8fbcbb,
    comment: 0x7d889b,
    code: 0xebcb8b,
};

pub const GRUVBOX_DARK: Palette = Palette {
    name: "Gruvbox Dark",
    dark: true,
    bg: 0x282828,
    surface: 0x32302f,
    border: 0x4a4540,
    fg: 0xebdbb2,
    muted: 0xa89984,
    accent: 0xfabd2f,
    keyword: 0xfb4934,
    string: 0xb8bb26,
    number: 0xd3869b,
    func: 0x83a598,
    ty: 0xfabd2f,
    constant: 0xd3869b,
    prop: 0x8ec07c,
    comment: 0x928374,
    code: 0xfe8019,
};

pub const DRACULA: Palette = Palette {
    name: "Dracula",
    dark: true,
    bg: 0x282a36,
    surface: 0x21222c,
    border: 0x44475a,
    fg: 0xf8f8f2,
    muted: 0x9aa0c0,
    accent: 0xbd93f9,
    keyword: 0xff79c6,
    string: 0xf1fa8c,
    number: 0xbd93f9,
    func: 0x50fa7b,
    ty: 0x8be9fd,
    constant: 0xffb86c,
    prop: 0x8be9fd,
    comment: 0x7a84b0,
    code: 0xffb86c,
};

pub const PRESETS: [Palette; 7] = [
    MEKU_LIGHT,
    MEKU_DARK,
    SOLARIZED_LIGHT,
    SOLARIZED_DARK,
    NORD,
    GRUVBOX_DARK,
    DRACULA,
];

/// The palette currently in force; read by the editor for syntax colors.
#[derive(Clone, Copy)]
pub struct Active(pub Palette);

impl Global for Active {}

/// The active palette, or the default light one before any theme was applied
/// (headless tests).
pub fn active(cx: &App) -> Palette {
    cx.try_global::<Active>().map_or(MEKU_LIGHT, |a| a.0)
}

/// Backdrop behind dialogs and the palette: always darkens, more so on dark
/// themes where a light scrim would wash the window out.
pub fn scrim(cx: &App) -> gpui_kit::Hsla {
    gpui_kit::hsla(0.0, 0.0, 0.0, if active(cx).dark { 0.5 } else { 0.3 })
}

/// The active accent color at the given opacity (find highlights etc.).
pub fn accent_alpha(cx: &App, alpha: f32) -> gpui_kit::Hsla {
    let mut c: gpui_kit::Hsla = gpui_kit::rgb(active(cx).accent).into();
    c.a = alpha;
    c
}

// -- color math -------------------------------------------------------------

fn channels(hex: u32) -> [f32; 3] {
    [
        ((hex >> 16) & 0xff) as f32,
        ((hex >> 8) & 0xff) as f32,
        (hex & 0xff) as f32,
    ]
}

/// `a` blended toward `b` by `t` (0..1), as `#rrggbb`.
fn mix(a: u32, b: u32, t: f32) -> String {
    let (a, b) = (channels(a), channels(b));
    let c = |i: usize| (a[i] + (b[i] - a[i]) * t).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", c(0), c(1), c(2))
}

fn hex(c: u32) -> String {
    format!("#{c:06x}")
}

fn alpha(c: u32, a: f32) -> String {
    format!("#{c:06x}{:02x}", (a * 255.0).round() as u8)
}

/// Relative luminance, for picking readable text on a colored fill.
fn luminance(c: u32) -> f32 {
    let lin = |v: f32| {
        let v = v / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = channels(c);
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

fn on(fill: u32) -> String {
    if luminance(fill) > 0.45 {
        "#0f1115".into()
    } else {
        "#ffffff".into()
    }
}

/// The kit `ThemeConfig` JSON for a palette: every surface the editor shell
/// draws, derived from the handful of palette colors.
fn config_json(p: &Palette, reg_name: &str) -> serde_json::Value {
    let hover = mix(p.bg, p.fg, 0.06);
    let active_bg = mix(p.bg, p.accent, 0.16);
    let surface_hover = mix(p.surface, p.fg, 0.07);
    let surface2 = mix(p.surface, p.fg, 0.05);
    let (danger, warning, success, info) = if p.dark {
        (0xf0707a, 0xe8b25a, 0x7cc98a, 0x6cb6e8)
    } else {
        (0xcf222e, 0x9a6700, 0x1a7f37, 0x0969da)
    };
    let colors: Vec<(&str, String)> = vec![
        ("background", hex(p.bg)),
        ("foreground", hex(p.fg)),
        ("border", hex(p.border)),
        ("input.border", hex(p.border)),
        ("caret", hex(p.accent)),
        ("ring", hex(p.accent)),
        ("selection.background", alpha(p.accent, 0.30)),
        ("overlay", "#00000066".to_string()),
        ("accent.background", hover.clone()),
        ("accent.foreground", hex(p.fg)),
        ("muted.background", hex(p.surface)),
        ("muted.foreground", hex(p.muted)),
        ("primary.background", hex(p.accent)),
        ("primary.hover.background", mix(p.accent, p.fg, 0.12)),
        ("primary.active.background", mix(p.accent, p.fg, 0.22)),
        ("primary.foreground", on(p.accent)),
        ("secondary.background", surface2.clone()),
        ("secondary.hover.background", surface_hover.clone()),
        ("secondary.active.background", mix(p.surface, p.fg, 0.10)),
        ("secondary.foreground", hex(p.fg)),
        ("popover.background", hex(p.surface)),
        ("popover.foreground", hex(p.fg)),
        ("link.foreground", hex(p.accent)),
        ("link.hover.foreground", mix(p.accent, p.fg, 0.25)),
        ("link.active.foreground", mix(p.accent, p.fg, 0.40)),
        ("list.background", hex(p.bg)),
        ("list.even.background", hex(p.bg)),
        ("list.head.background", hex(p.surface)),
        ("list.hover.background", hover.clone()),
        ("list.active.background", active_bg.clone()),
        ("list.active.border", hex(p.accent)),
        ("sidebar.background", hex(p.surface)),
        ("sidebar.foreground", hex(p.fg)),
        ("sidebar.border", hex(p.border)),
        ("sidebar.accent.background", active_bg.clone()),
        ("sidebar.accent.foreground", hex(p.fg)),
        ("sidebar.primary.background", hex(p.accent)),
        ("sidebar.primary.foreground", on(p.accent)),
        ("tab_bar.background", hex(p.surface)),
        ("tab_bar.segmented.background", surface2.clone()),
        ("tab.background", hex(p.surface)),
        ("tab.foreground", hex(p.muted)),
        ("tab.active.background", hex(p.bg)),
        ("tab.active.foreground", hex(p.fg)),
        ("title_bar.background", hex(p.surface)),
        ("title_bar.border", hex(p.border)),
        ("status_bar.background", hex(p.surface)),
        ("status_bar.border", hex(p.border)),
        ("window.border", hex(p.border)),
        ("scrollbar.background", "#00000000".to_string()),
        ("scrollbar.thumb.background", alpha(p.muted, 0.45)),
        ("scrollbar.thumb.hover.background", alpha(p.muted, 0.7)),
        ("danger.background", hex(danger)),
        ("danger.foreground", on(danger)),
        ("warning.background", hex(warning)),
        ("warning.foreground", on(warning)),
        ("success.background", hex(success)),
        ("success.foreground", on(success)),
        ("info.background", hex(info)),
        ("info.foreground", on(info)),
    ];
    let colors: serde_json::Map<String, serde_json::Value> = colors
        .into_iter()
        .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
        .collect();
    let mut theme = serde_json::Map::new();
    theme.insert("name".into(), reg_name.into());
    theme.insert("mode".into(), if p.dark { "dark" } else { "light" }.into());
    theme.insert("colors".into(), colors.into());
    let mut set = serde_json::Map::new();
    set.insert("name".into(), "Meku".into());
    set.insert(
        "themes".into(),
        vec![serde_json::Value::Object(theme)].into(),
    );
    serde_json::Value::Object(set)
}

/// The name a palette is registered under in the kit's theme registry. It
/// includes a hash of the colours so an edited custom theme replaces the
/// stale registration instead of being ignored.
fn registry_name(p: &Palette) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (p.dark, p.bg, p.surface, p.border, p.fg, p.muted, p.accent).hash(&mut h);
    (
        p.keyword, p.string, p.number, p.func, p.ty, p.constant, p.prop, p.comment, p.code,
    )
        .hash(&mut h);
    format!("{}:{:x}", p.name, h.finish())
}

/// Make `palette` the app's look. Safe to call repeatedly.
pub fn apply(palette: Palette, cx: &mut App) {
    let reg_name = registry_name(&palette);
    let registry = ThemeRegistry::global_mut(cx);
    if registry.themes().get(reg_name.as_str()).is_none() {
        let _ = registry.load_themes_from_str(&config_json(&palette, &reg_name).to_string());
    }
    let config: Option<Rc<ThemeConfig>> = ThemeRegistry::global(cx)
        .themes()
        .get(reg_name.as_str())
        .cloned();
    if let Some(config) = config {
        Theme::update(cx, |theme| theme.apply_config(&config));
    }
    cx.set_global(Active(palette));
    cx.refresh_windows();
}

// -- motion ------------------------------------------------------------------

/// Whether animations run: the user has not asked to reduce motion. With no
/// settings loaded (headless tests) motion is off, because animations only
/// finish when real time passes.
pub fn motion(cx: &App) -> bool {
    cx.try_global::<ConfigState>()
        .is_some_and(|s| !s.effective.bool("reduce_motion"))
}

/// Fade `el` in over `ms` (ease-out) when it first appears, sliding it
/// `lift` pixels into place from below. A plain element with reduced motion.
pub fn fade_in<E>(
    el: E,
    id: impl Into<gpui_kit::ElementId>,
    ms: u64,
    lift: f32,
    cx: &App,
) -> gpui_kit::AnyElement
where
    E: gpui_kit::IntoElement + gpui_kit::Styled + 'static,
{
    fade_from(el, id, ms, 0.0, lift, cx)
}

/// Like [`fade_in`], starting at `from` opacity instead of invisible.
pub fn fade_from<E>(
    el: E,
    id: impl Into<gpui_kit::ElementId>,
    ms: u64,
    from: f32,
    lift: f32,
    cx: &App,
) -> gpui_kit::AnyElement
where
    E: gpui_kit::IntoElement + gpui_kit::Styled + 'static,
{
    use gpui_kit::{Animation, AnimationExt as _, IntoElement as _};
    if !motion(cx) {
        return el.into_any_element();
    }
    el.with_animation(
        id,
        Animation::new(std::time::Duration::from_millis(ms))
            .with_easing(gpui_kit::ease_out_quint()),
        move |el, t| {
            el.opacity(from + (1.0 - from) * t)
                .mt(gpui_kit::px(lift * (1.0 - t)))
        },
    )
    .into_any_element()
}

/// Smallest / largest document zoom and the step between presets.
pub const ZOOM_MIN: f32 = 0.7;
pub const ZOOM_MAX: f32 = 2.0;
const ZOOM_STEP: f32 = 0.1;

/// Current document zoom (1.0 with no settings, e.g. in tests).
pub fn zoom(cx: &App) -> f32 {
    (cfg(cx).f64("zoom") as f32).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Step the zoom by `steps` tenths (negative = out), or reset with `None`.
/// Saved immediately.
pub fn set_zoom(change: Option<i32>, cx: &mut App) {
    let next = match change {
        None => 1.0,
        Some(steps) => {
            ((zoom(cx) + steps as f32 * ZOOM_STEP) * 10.0)
                .round()
                .clamp(ZOOM_MIN * 10.0, ZOOM_MAX * 10.0)
                / 10.0
        }
    };
    let _ = set_setting("zoom", serde_json::json!(next), cx);
}

// -- settings access ---------------------------------------------------------

impl Global for ConfigState {}

/// The loaded settings (defaults before anything is loaded: headless tests).
pub fn state(cx: &App) -> &ConfigState {
    static DEFAULT: OnceLock<ConfigState> = OnceLock::new();
    cx.try_global::<ConfigState>()
        .unwrap_or_else(|| DEFAULT.get_or_init(ConfigState::in_memory))
}

/// The effective setting values.
pub fn cfg(cx: &App) -> &Config {
    &state(cx).effective
}

/// Change the settings, then re-resolve the theme and repaint. Never fails
/// the caller: a refused write (broken file) is returned for the UI to show.
pub fn update_config<R>(cx: &mut App, f: impl FnOnce(&mut ConfigState) -> R) -> R {
    if cx.try_global::<ConfigState>().is_none() {
        cx.set_global(ConfigState::in_memory());
    }
    let out = f(cx.global_mut::<ConfigState>());
    reapply(cx);
    out
}

/// The layer a quick change (zoom keys, theme picker) should write to: the
/// mekuto's when it already overrides the key, else the global one.
fn scope_for(state: &ConfigState, key: &str) -> Scope {
    if state.is_set(Scope::Mekuto, key) {
        Scope::Mekuto
    } else {
        Scope::Global
    }
}

/// Set one setting where it already lives (see [`scope_for`]) and save.
pub fn set_setting(key: &str, value: serde_json::Value, cx: &mut App) -> Result<(), String> {
    update_config(cx, |s| {
        let scope = scope_for(s, key);
        s.write(scope, key, value)
    })
}

// -- palettes ----------------------------------------------------------------

/// Leak each distinct theme name once so a [`Palette`] can keep a
/// `&'static str`; custom themes are few and names repeat across reloads.
fn intern(name: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut set = NAMES.get_or_init(Default::default).lock().unwrap();
    if let Some(found) = set.get(name) {
        return found;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    set.insert(leaked);
    leaked
}

fn build_custom(spec: &ThemeSpec, known: &[Palette]) -> Palette {
    let by_name = |n: &str| {
        known
            .iter()
            .copied()
            .find(|p| p.name.eq_ignore_ascii_case(n))
    };
    let base = spec
        .base
        .as_deref()
        .and_then(by_name)
        .unwrap_or(if spec.dark == Some(true) {
            MEKU_DARK
        } else {
            MEKU_LIGHT
        });
    let mut p = Palette {
        name: intern(&spec.name),
        dark: spec.dark.unwrap_or(base.dark),
        ..base
    };
    for (key, c) in &spec.colors {
        let c = *c;
        match key.as_str() {
            "bg" => p.bg = c,
            "surface" => p.surface = c,
            "border" => p.border = c,
            "fg" => p.fg = c,
            "muted" => p.muted = c,
            "accent" => p.accent = c,
            "keyword" => p.keyword = c,
            "string" => p.string = c,
            "number" => p.number = c,
            "func" => p.func = c,
            "ty" => p.ty = c,
            "constant" => p.constant = c,
            "prop" => p.prop = c,
            "comment" => p.comment = c,
            "code" => p.code = c,
            _ => {}
        }
    }
    p
}

/// Built-in presets followed by the custom themes from settings. A custom
/// theme named like a preset replaces it.
pub fn palettes_of(state: &ConfigState) -> Vec<Palette> {
    let mut all: Vec<Palette> = PRESETS.to_vec();
    for spec in &state.themes {
        let p = build_custom(spec, &all);
        match all.iter().position(|e| e.name.eq_ignore_ascii_case(p.name)) {
            Some(i) => all[i] = p,
            None => all.push(p),
        }
    }
    all
}

pub fn palettes(cx: &App) -> Vec<Palette> {
    palettes_of(state(cx))
}

/// A palette by (case-insensitive) name, custom themes included.
pub fn find(name: &str, cx: &App) -> Option<Palette> {
    palettes(cx)
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// The palette the settings select under the given OS appearance.
pub fn resolve(state: &ConfigState, appearance: WindowAppearance) -> Palette {
    let all = palettes_of(state);
    let pick = |name: &str, fallback: Palette| {
        all.iter()
            .copied()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .unwrap_or(fallback)
    };
    let c = &state.effective;
    if let Some(name) = &state.env_theme {
        return pick(name, MEKU_LIGHT);
    }
    if c.bool("follow_system") {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => {
                pick(c.str("theme_dark"), MEKU_DARK)
            }
            WindowAppearance::Light | WindowAppearance::VibrantLight => {
                pick(c.str("theme_light"), MEKU_LIGHT)
            }
        }
    } else {
        pick(c.str("theme"), MEKU_LIGHT)
    }
}

/// Re-resolve the palette from the current settings and apply it.
pub fn reapply(cx: &mut App) {
    let palette = resolve(state(cx), cx.window_appearance());
    apply(palette, cx);
}

/// Load the settings (global file, plus the mekuto's when `root` is known),
/// with `MEKU_THEME` overriding the theme for screenshots, and apply them.
pub fn init(root: Option<&std::path::Path>, cx: &mut App) {
    let mut state = ConfigState::load(root);
    if let Ok(name) = std::env::var("MEKU_THEME") {
        let slug = name.to_lowercase();
        state.env_theme = palettes_of(&state)
            .iter()
            .find(|p| {
                p.name.eq_ignore_ascii_case(&name)
                    || p.name.to_lowercase().replace(' ', "-") == slug
            })
            .map(|p| p.name.to_string());
    }
    cx.set_global(state);
    reapply(cx);
}

/// Change theme from the UI and remember it.
pub fn choose(name: &str, cx: &mut App) {
    if let Some(palette) = find(name, cx) {
        update_config(cx, |s| {
            s.env_theme = None;
            let scope = scope_for(s, "theme");
            let _ = s.write(scope, "theme", palette.name.into());
            let scope = scope_for(s, "follow_system");
            let _ = s.write(scope, "follow_system", false.into());
        });
    }
}

/// Next theme in the list (presets, then custom), wrapping.
pub fn cycle(cx: &mut App) {
    let all = palettes(cx);
    let current = active(cx).name;
    let at = all.iter().position(|p| p.name == current).unwrap_or(0);
    choose(all[(at + 1) % all.len()].name, cx);
}

/// Switch back to following the OS appearance.
#[allow(dead_code)] // wired to the command palette
pub fn follow_system(cx: &mut App) {
    update_config(cx, |s| {
        s.env_theme = None;
        let scope = scope_for(s, "follow_system");
        let _ = s.write(scope, "follow_system", true.into());
    });
}

/// Re-resolve after the OS flipped between light and dark.
pub fn on_system_appearance_changed(cx: &mut App) {
    if cfg(cx).bool("follow_system") {
        reapply(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_unique_names_and_readable_text() {
        let mut names: Vec<_> = PRESETS.iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), PRESETS.len());
        for p in PRESETS {
            let ratio = |a: u32, b: u32| {
                let (la, lb) = (luminance(a) + 0.05, luminance(b) + 0.05);
                la.max(lb) / la.min(lb)
            };
            assert!(ratio(p.fg, p.bg) >= 7.0, "{} body text contrast", p.name);
            assert!(
                ratio(p.muted, p.bg) >= 4.5,
                "{} muted text contrast",
                p.name
            );
            assert!(
                ratio(p.muted, p.surface) >= 4.0,
                "{} muted on surface",
                p.name
            );
            assert!(
                ratio(p.code, p.bg) >= 4.5,
                "{} inline code contrast",
                p.name
            );
            for (what, c) in [
                ("string", p.string),
                ("keyword", p.keyword),
                ("func", p.func),
                ("number", p.number),
            ] {
                assert!(ratio(c, p.bg) >= 3.0, "{} {what} contrast", p.name);
            }
        }
    }

    #[test]
    fn mix_and_text_helpers() {
        assert_eq!(mix(0x000000, 0xffffff, 0.5), "#808080");
        assert_eq!(on(0xffffff), "#0f1115");
        assert_eq!(on(0x000000), "#ffffff");
        assert_eq!(alpha(0xff0000, 0.5), "#ff000080");
    }

    #[test]
    fn custom_themes_inherit_from_a_base_and_replace_same_named_presets() {
        let mut state = ConfigState::in_memory();
        state.global.values = crate::config::parse_layer_text(
            r##"{"themes": [
                {"name": "Ink", "base": "nord", "bg": "#101010"},
                {"name": "Paper", "dark": false, "accent": "#ff0000"},
                {"name": "Dracula", "bg": "#000000"}
            ]}"##,
        )
        .unwrap();
        state.rebuild();
        let all = palettes_of(&state);
        assert_eq!(all.len(), PRESETS.len() + 2, "Dracula replaced, two added");
        let ink = all.iter().find(|p| p.name == "Ink").unwrap();
        assert_eq!(
            (ink.bg, ink.fg, ink.dark),
            (0x101010, NORD.fg, true),
            "inherits the rest from Nord"
        );
        let paper = all.iter().find(|p| p.name == "Paper").unwrap();
        assert_eq!((paper.accent, paper.bg), (0xff0000, MEKU_LIGHT.bg));
        assert_eq!(
            all.iter().find(|p| p.name == "Dracula").unwrap().bg,
            0x000000
        );
        assert!(
            std::ptr::eq(intern("Ink"), ink.name),
            "names are interned once"
        );
    }

    #[test]
    fn resolution_follows_system_or_the_chosen_theme() {
        let mut state = ConfigState::in_memory();
        assert_eq!(resolve(&state, WindowAppearance::Dark).name, "Meku Dark");
        assert_eq!(resolve(&state, WindowAppearance::Light).name, "Meku Light");
        state.global.values = crate::config::parse_layer_text(
            r#"{"follow_system": false, "theme": "Nord", "theme_dark": "Dracula"}"#,
        )
        .unwrap();
        state.rebuild();
        assert_eq!(resolve(&state, WindowAppearance::Light).name, "Nord");
        state.global.values =
            crate::config::parse_layer_text(r#"{"theme_dark": "Dracula"}"#).unwrap();
        state.rebuild();
        assert_eq!(resolve(&state, WindowAppearance::Dark).name, "Dracula");
        state.global.values =
            crate::config::parse_layer_text(r#"{"theme_dark": "Missing"}"#).unwrap();
        state.rebuild();
        assert_eq!(
            resolve(&state, WindowAppearance::Dark).name,
            "Meku Dark",
            "unknown name falls back"
        );
    }
}
