//! Theme presets and their persistence.
//!
//! A [`Palette`] is the whole look of a preset: surface/ink/accent colors plus
//! the code-block syntax colors. [`apply`] turns it into a kit `ThemeConfig`
//! (so every kit component follows) and publishes it as the [`Active`] global
//! (so the editor's own syntax colors follow too).

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeRegistry};
use gpui_kit::{App, Global, WindowAppearance};
use serde::{Deserialize, Serialize};

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

pub fn preset(name: &str) -> Option<Palette> {
    PRESETS.iter().copied().find(|p| p.name == name)
}

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
fn config_json(p: &Palette) -> serde_json::Value {
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
    theme.insert("name".into(), p.name.into());
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

/// Make `palette` the app's look. Safe to call repeatedly.
pub fn apply(palette: Palette, cx: &mut App) {
    let json = config_json(&palette).to_string();
    let registry = ThemeRegistry::global_mut(cx);
    if registry.themes().get(palette.name).is_none() {
        let _ = registry.load_themes_from_str(&json);
    }
    let config: Option<Rc<ThemeConfig>> = ThemeRegistry::global(cx)
        .themes()
        .get(palette.name)
        .cloned();
    if let Some(config) = config {
        Theme::update(cx, |theme| theme.apply_config(&config));
    }
    cx.set_global(Active(palette));
    cx.refresh_windows();
}

// -- settings -----------------------------------------------------------------

/// User preferences kept across runs in `~/.config/meku/settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Preset name; ignored while `follow_system` is on.
    pub theme: String,
    /// Pick Meku Light / Meku Dark from the OS appearance.
    pub follow_system: bool,
    /// Disable every non-essential animation.
    pub reduce_motion: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: MEKU_LIGHT.name.to_string(),
            follow_system: true,
            reduce_motion: false,
        }
    }
}

impl Global for Settings {}

fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("meku").join("settings.json"))
}

impl Settings {
    /// Read the saved settings; a missing or unreadable file yields defaults
    /// (never an error: a bad settings file must not stop the app starting).
    pub fn load() -> Self {
        settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    /// The palette these settings select under the given OS appearance.
    pub fn resolve(&self, appearance: WindowAppearance) -> Palette {
        if self.follow_system {
            match appearance {
                WindowAppearance::Dark | WindowAppearance::VibrantDark => MEKU_DARK,
                WindowAppearance::Light | WindowAppearance::VibrantLight => MEKU_LIGHT,
            }
        } else {
            preset(&self.theme).unwrap_or(MEKU_LIGHT)
        }
    }
}

/// Apply the saved settings (with `MEKU_THEME` overriding, for screenshots).
pub fn init(cx: &mut App) {
    let mut settings = Settings::load();
    if let Ok(name) = std::env::var("MEKU_THEME")
        && let Some(p) = PRESETS.iter().find(|p| {
            p.name.eq_ignore_ascii_case(&name)
                || p.name.to_lowercase().replace(' ', "-") == name.to_lowercase()
        })
    {
        settings.theme = p.name.to_string();
        settings.follow_system = false;
    }
    let palette = settings.resolve(cx.window_appearance());
    cx.set_global(settings);
    apply(palette, cx);
}

/// Change preset from the UI and remember it.
pub fn choose(name: &str, cx: &mut App) {
    if let Some(palette) = preset(name) {
        let mut settings = cx.global::<Settings>().clone();
        settings.theme = palette.name.to_string();
        settings.follow_system = false;
        settings.save();
        cx.set_global(settings);
        apply(palette, cx);
    }
}

/// Switch back to following the OS appearance.
#[allow(dead_code)] // wired to the command palette
pub fn follow_system(cx: &mut App) {
    let mut settings = cx.global::<Settings>().clone();
    settings.follow_system = true;
    settings.save();
    let palette = settings.resolve(cx.window_appearance());
    cx.set_global(settings);
    apply(palette, cx);
}

/// Re-resolve after the OS flipped between light and dark.
pub fn on_system_appearance_changed(cx: &mut App) {
    let settings = cx.global::<Settings>().clone();
    if settings.follow_system {
        apply(settings.resolve(cx.window_appearance()), cx);
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
    fn settings_roundtrip_and_resolution() {
        let mut s = Settings::default();
        assert_eq!(s.resolve(WindowAppearance::Dark).name, "Meku Dark");
        assert_eq!(s.resolve(WindowAppearance::Light).name, "Meku Light");
        s.follow_system = false;
        s.theme = "Nord".into();
        assert_eq!(s.resolve(WindowAppearance::Light).name, "Nord");
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
        assert_eq!(
            serde_json::from_str::<Settings>("{}").unwrap(),
            Settings::default()
        );
        s.theme = "Nope".into();
        assert_eq!(s.resolve(WindowAppearance::Dark).name, "Meku Light");
    }
}
