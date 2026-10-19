//! Settings: one registry of every configurable thing, two JSON layers.
//!
//! * **Global** — `~/.config/meku/settings.json`, applies to every mekuto.
//! * **Mekuto** — `<mekuto>/.meku/settings.json`, overrides the global layer
//!   for that folder only.
//!
//! Both files hold only what the user changed (plus `themes`, custom theme
//! definitions). A file with broken JSON, or a key with a bad value, never
//! stops the app: valid keys still apply, the problems are reported as
//! [`Issue`]s, and a broken file is never overwritten until the user chooses
//! to back it up and start fresh.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value};

/// Where a value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Mekuto,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Global => "global settings",
            Scope::Mekuto => "this mekuto's settings",
        }
    }
}

/// Which page section a setting belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Layout,
    Editor,
    Markdown,
    Files,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Appearance,
        Section::Layout,
        Section::Editor,
        Section::Markdown,
        Section::Files,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Layout => "Layout",
            Section::Editor => "Editor",
            Section::Markdown => "Markdown",
            Section::Files => "Files",
        }
    }
}

/// What a setting accepts.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Bool,
    /// One of a fixed set of strings.
    Choice(&'static [&'static str]),
    Number {
        min: f64,
        max: f64,
        step: f64,
    },
    Text,
    /// The name of a theme (built-in or custom); the options are dynamic.
    Theme,
}

/// A default value.
#[derive(Debug, Clone, Copy)]
pub enum Dv {
    B(bool),
    N(f64),
    S(&'static str),
}

impl Dv {
    fn to_value(self) -> Value {
        match self {
            Dv::B(b) => Value::Bool(b),
            Dv::N(n) => serde_json::json!(n),
            Dv::S(s) => Value::String(s.to_string()),
        }
    }
}

/// One entry in the registry.
#[derive(Debug, Clone, Copy)]
pub struct Def {
    pub key: &'static str,
    pub section: Section,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    pub default: Dv,
}

const fn def(
    key: &'static str,
    section: Section,
    label: &'static str,
    help: &'static str,
    kind: Kind,
    default: Dv,
) -> Def {
    Def {
        key,
        section,
        label,
        help,
        kind,
        default,
    }
}

const MODES: &[&str] = &["edit", "live", "read"];
const WIDTHS: &[&str] = &["narrow", "comfortable", "wide", "full"];
const REVEAL: &[&str] = &["selection", "caret-line", "never"];
const LINKS: &[&str] = &["click", "ctrl-click"];

/// Every setting. The settings page, the validator and the defaults all come
/// from this table, so a new option is one line here plus its use.
pub const DEFS: &[Def] = &[
    // -- Appearance
    def(
        "theme",
        Section::Appearance,
        "Theme",
        "Picking one stops following the system.",
        Kind::Theme,
        Dv::S("Meku Light"),
    ),
    def(
        "follow_system",
        Section::Appearance,
        "Follow system appearance",
        "Use the light or dark theme to match your system.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "theme_light",
        Section::Appearance,
        "Light theme",
        "Used when the system is light.",
        Kind::Theme,
        Dv::S("Meku Light"),
    ),
    def(
        "theme_dark",
        Section::Appearance,
        "Dark theme",
        "Used when the system is dark.",
        Kind::Theme,
        Dv::S("Meku Dark"),
    ),
    def(
        "zoom",
        Section::Appearance,
        "Document zoom",
        "Scale of the note text (Ctrl +, Ctrl -).",
        Kind::Number {
            min: 0.7,
            max: 2.0,
            step: 0.1,
        },
        Dv::N(1.0),
    ),
    def(
        "ui_font",
        Section::Appearance,
        "Interface and body font",
        "Font family. Empty uses Inter.",
        Kind::Text,
        Dv::S(""),
    ),
    def(
        "code_font",
        Section::Appearance,
        "Code font",
        "Monospace family. Empty uses JetBrains Mono.",
        Kind::Text,
        Dv::S(""),
    ),
    def(
        "column_width",
        Section::Appearance,
        "Reading column width",
        "How wide the note text column is.",
        Kind::Choice(WIDTHS),
        Dv::S("comfortable"),
    ),
    def(
        "reduce_motion",
        Section::Appearance,
        "Reduce motion",
        "Turn off fades and slides.",
        Kind::Bool,
        Dv::B(false),
    ),
    // -- Layout
    def(
        "sidebar_visible",
        Section::Layout,
        "Show sidebar",
        "The file tree and outline panel.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "outline_visible",
        Section::Layout,
        "Show outline",
        "The heading list under the file tree.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "tab_bar_visible",
        Section::Layout,
        "Show tab bar",
        "The open notes strip.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "status_bar_visible",
        Section::Layout,
        "Show status bar",
        "Saved state, word count and position.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "narrow_breakpoint",
        Section::Layout,
        "Fold sidebar below (px)",
        "Hide the sidebar in windows narrower than this. 0 never hides it.",
        Kind::Number {
            min: 0.0,
            max: 2000.0,
            step: 20.0,
        },
        Dv::N(820.0),
    ),
    // -- Editor
    def(
        "default_mode",
        Section::Editor,
        "Default view",
        "How notes open: Source, Live preview or Reading.",
        Kind::Choice(MODES),
        Dv::S("live"),
    ),
    def(
        "reveal_markdown",
        Section::Editor,
        "Show markdown source",
        "When live view shows raw markdown.",
        Kind::Choice(REVEAL),
        Dv::S("selection"),
    ),
    def(
        "auto_close_pairs",
        Section::Editor,
        "Auto-close brackets and quotes",
        "Typing ( [ { \" ` * adds the closing half.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "smart_lists",
        Section::Editor,
        "Smart lists",
        "Enter continues bullets, numbers and tasks.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "slash_menu",
        Section::Editor,
        "Slash menu",
        "Type / at the start of a line to insert blocks.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "caret_blink",
        Section::Editor,
        "Blinking caret",
        "",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "caret_blink_ms",
        Section::Editor,
        "Caret blink interval (ms)",
        "How long the caret stays on or off.",
        Kind::Number {
            min: 200.0,
            max: 2000.0,
            step: 50.0,
        },
        Dv::N(530.0),
    ),
    def(
        "link_click",
        Section::Editor,
        "Follow links with",
        "A plain click, or Ctrl+click only.",
        Kind::Choice(LINKS),
        Dv::S("click"),
    ),
    // -- Markdown
    def(
        "render_tables",
        Section::Markdown,
        "Render tables as grids",
        "Off shows pipe tables as plain text.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "render_callouts",
        Section::Markdown,
        "Render callouts",
        "Off shows > [!note] blocks as plain quotes.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "render_images",
        Section::Markdown,
        "Show images",
        "Off shows image lines as text.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "image_max_height",
        Section::Markdown,
        "Image max height (px)",
        "Taller pictures are scaled down to this.",
        Kind::Number {
            min: 120.0,
            max: 1600.0,
            step: 20.0,
        },
        Dv::N(460.0),
    ),
    def(
        "image_click_zoom",
        Section::Markdown,
        "Click image to zoom",
        "Open the zoom view when an image is clicked.",
        Kind::Bool,
        Dv::B(true),
    ),
    // -- Files
    def(
        "autosave",
        Section::Files,
        "Autosave",
        "Save notes shortly after you stop typing.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "autosave_delay_ms",
        Section::Files,
        "Autosave delay (ms)",
        "How long to wait after the last keystroke.",
        Kind::Number {
            min: 200.0,
            max: 10000.0,
            step: 100.0,
        },
        Dv::N(800.0),
    ),
    def(
        "new_note_folder",
        Section::Files,
        "New notes go in",
        "A folder in the mekuto. Empty means the top level.",
        Kind::Text,
        Dv::S(""),
    ),
    def(
        "attachments_folder",
        Section::Files,
        "Attachments folder",
        "Pasted and dropped images are saved here, beside the note.",
        Kind::Text,
        Dv::S("attachments"),
    ),
    def(
        "confirm_delete",
        Section::Files,
        "Confirm before deleting",
        "Ask before deleting a note or folder.",
        Kind::Bool,
        Dv::B(true),
    ),
    def(
        "restore_session",
        Section::Files,
        "Reopen last tabs",
        "Restore the open notes when a mekuto opens.",
        Kind::Bool,
        Dv::B(true),
    ),
];

/// Keys that are valid in a settings file without being registry entries.
const EXTRA_KEYS: &[&str] = &["themes", "$schema"];

pub fn def_for(key: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.key == key)
}

/// A problem found while loading settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub scope: Scope,
    /// The key involved, when the problem is about one setting.
    pub key: Option<String>,
    pub message: String,
    /// The whole file could not be read (broken JSON); nothing from it applies.
    pub fatal: bool,
}

/// A JSON syntax error with a position and, when we can guess one, a hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub hint: Option<&'static str>,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )?;
        if let Some(h) = self.hint {
            write!(f, " ({h})")?;
        }
        Ok(())
    }
}

/// Parse a settings file's text into its top-level object.
pub fn parse_layer_text(text: &str) -> Result<Map<String, Value>, ParseError> {
    if text.trim().is_empty() {
        return Ok(Map::new()); // an empty file is fine: nothing set
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(ParseError {
            message: "the file must contain one JSON object ({ … })".into(),
            line: 1,
            column: 1,
            hint: None,
        }),
        Err(e) => {
            let msg = e.to_string();
            let line_text = text.lines().nth(e.line().saturating_sub(1)).unwrap_or("");
            let hint = if msg.contains("trailing comma") {
                Some("remove the comma after the last item")
            } else if line_text.trim_start().starts_with("//")
                || line_text.trim_start().starts_with('#')
            {
                Some("JSON does not allow comments")
            } else if line_text.contains('\'') && !line_text.contains('"') {
                Some("JSON uses double quotes, not single quotes")
            } else if msg.contains("key must be a string") {
                Some("keys need double quotes, like \"zoom\"")
            } else if msg.contains("expected `,` or `}`") || msg.contains("expected `,` or `]`") {
                Some("a comma may be missing between items")
            } else if msg.contains("EOF while parsing") {
                Some("a closing bracket or quote is missing")
            } else {
                None
            };
            // serde's message ends with " at line X column Y"; keep the front.
            let message = msg.split(" at line ").next().unwrap_or(&msg).to_string();
            Err(ParseError {
                message,
                line: e.line().max(1),
                column: e.column().max(1),
                hint,
            })
        }
    }
}

/// Check `value` against `def`. Out-of-range numbers are clamped and
/// reported; everything else wrong is an error (the default stays in force).
fn validate(def: &Def, value: &Value) -> Result<(Value, Option<String>), String> {
    match (def.kind, value) {
        (Kind::Bool, Value::Bool(_)) => Ok((value.clone(), None)),
        (Kind::Bool, _) => Err("expected true or false".into()),
        (Kind::Choice(options), Value::String(s)) => {
            if options.contains(&s.as_str()) {
                Ok((value.clone(), None))
            } else {
                Err(format!("\"{s}\" is not one of {}", options.join(", ")))
            }
        }
        (Kind::Choice(_), _) => Err("expected a text value".into()),
        (Kind::Number { min, max, .. }, Value::Number(n)) => {
            let x = n.as_f64().unwrap_or(f64::NAN);
            if !x.is_finite() {
                return Err("expected a finite number".into());
            }
            let c = x.clamp(min, max);
            let note = (c != x).then(|| format!("{x} is outside {min}–{max}; using {c}"));
            Ok((serde_json::json!(c), note))
        }
        (Kind::Number { .. }, _) => Err("expected a number".into()),
        (Kind::Text | Kind::Theme, Value::String(_)) => Ok((value.clone(), None)),
        (Kind::Text | Kind::Theme, _) => Err("expected a text value".into()),
    }
}

/// One settings file, parsed (or not).
#[derive(Debug, Clone)]
pub struct Layer {
    pub path: Option<PathBuf>,
    /// The whole top-level object, unknown keys and `themes` included, so a
    /// save keeps everything the user wrote.
    pub values: Map<String, Value>,
    /// Set when the file exists but is not valid JSON.
    pub error: Option<ParseError>,
    mtime: Option<SystemTime>,
}

impl Layer {
    fn empty(path: Option<PathBuf>) -> Self {
        Self {
            path,
            values: Map::new(),
            error: None,
            mtime: None,
        }
    }

    fn load(path: PathBuf) -> Self {
        let mut layer = Self::empty(Some(path.clone()));
        layer.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        match std::fs::read_to_string(&path) {
            Ok(text) => match parse_layer_text(&text) {
                Ok(map) => layer.values = map,
                Err(e) => layer.error = Some(e),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                layer.error = Some(ParseError {
                    message: format!("could not read the file: {e}"),
                    line: 1,
                    column: 1,
                    hint: None,
                })
            }
        }
        layer
    }

    fn disk_mtime(&self) -> Option<SystemTime> {
        self.path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
    }
}

/// The effective values (defaults ← global ← mekuto).
#[derive(Debug, Clone)]
pub struct Config {
    values: HashMap<&'static str, Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            values: DEFS.iter().map(|d| (d.key, d.default.to_value())).collect(),
        }
    }
}

impl Config {
    pub fn bool(&self, key: &str) -> bool {
        self.values
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or_else(|| match def_for(key).map(|d| d.default) {
                Some(Dv::B(b)) => b,
                _ => false,
            })
    }

    pub fn f64(&self, key: &str) -> f64 {
        self.values
            .get(key)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| match def_for(key).map(|d| d.default) {
                Some(Dv::N(n)) => n,
                _ => 0.0,
            })
    }

    pub fn str(&self, key: &str) -> &str {
        self.values
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_else(|| match def_for(key).map(|d| d.default) {
                Some(Dv::S(s)) => s,
                _ => "",
            })
    }
}

/// A custom theme as written in a settings file, before it is turned into a
/// palette (that needs the built-in themes to inherit from).
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeSpec {
    pub name: String,
    pub dark: Option<bool>,
    pub base: Option<String>,
    /// Colour fields by name (`bg`, `accent`, …) as `0xRRGGBB`.
    pub colors: Vec<(String, u32)>,
    pub scope: Scope,
}

/// The colour fields a theme may set.
pub const THEME_COLOR_KEYS: &[&str] = &[
    "bg", "surface", "border", "fg", "muted", "accent", "keyword", "string", "number", "func",
    "ty", "constant", "prop", "comment", "code",
];

/// Parse `#rgb` / `#rrggbb` (with or without `#`).
pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    match h.len() {
        6 => u32::from_str_radix(h, 16).ok(),
        3 => {
            let v = u32::from_str_radix(h, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            Some(((r * 17) << 16) | ((g * 17) << 8) | (b * 17))
        }
        _ => None,
    }
}

fn parse_themes(scope: Scope, value: &Value, issues: &mut Vec<Issue>) -> Vec<ThemeSpec> {
    let issue = |key: &str, message: String| Issue {
        scope,
        key: Some(key.to_string()),
        message,
        fatal: false,
    };
    let Value::Array(items) = value else {
        issues.push(issue(
            "themes",
            "expected a list of themes: [ { \"name\": … }, … ]".into(),
        ));
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let at = format!("themes[{i}]");
        let Value::Object(obj) = item else {
            issues.push(issue(&at, "expected an object".into()));
            continue;
        };
        let Some(name) = obj
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            issues.push(issue(&at, "every theme needs a \"name\"".into()));
            continue;
        };
        let dark = match obj.get("dark") {
            None => None,
            Some(Value::Bool(b)) => Some(*b),
            Some(_) => {
                issues.push(issue(
                    &format!("themes[{i}].dark"),
                    "expected true or false".into(),
                ));
                None
            }
        };
        let base = obj.get("base").and_then(Value::as_str).map(str::to_string);
        let mut colors = Vec::new();
        for (k, v) in obj {
            if matches!(k.as_str(), "name" | "dark" | "base") {
                continue;
            }
            if !THEME_COLOR_KEYS.contains(&k.as_str()) {
                issues.push(issue(
                    &format!("themes[{i}].{k}"),
                    format!(
                        "unknown theme field (known: {})",
                        THEME_COLOR_KEYS.join(", ")
                    ),
                ));
                continue;
            }
            match v.as_str().and_then(parse_hex) {
                Some(c) => colors.push((k.clone(), c)),
                None => issues.push(issue(
                    &format!("themes[{i}].{k}"),
                    "expected a colour like \"#1e1e2e\"; using the base theme's".into(),
                )),
            }
        }
        out.push(ThemeSpec {
            name: name.to_string(),
            dark,
            base,
            colors,
            scope,
        });
    }
    out
}

/// Both layers, the merged result, and everything that went wrong.
#[derive(Debug, Clone)]
pub struct ConfigState {
    pub global: Layer,
    pub mekuto: Option<Layer>,
    pub effective: Config,
    pub issues: Vec<Issue>,
    /// Custom themes: mekuto themes override global ones of the same name.
    pub themes: Vec<ThemeSpec>,
    /// `MEKU_THEME` for this run only (screenshots); never saved.
    pub env_theme: Option<String>,
    /// Bumped on every rebuild; views compare it to notice changes.
    pub generation: u64,
}

impl Default for ConfigState {
    fn default() -> Self {
        Self::in_memory()
    }
}

/// `~/.config/meku/settings.json` (or `$XDG_CONFIG_HOME`).
pub fn global_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("meku").join("settings.json"))
}

pub fn mekuto_path(root: &Path) -> PathBuf {
    root.join(".meku").join("settings.json")
}

impl ConfigState {
    /// No files at all: defaults only, nothing is ever written. Used by
    /// tests and before anything is loaded.
    pub fn in_memory() -> Self {
        let mut s = Self {
            global: Layer::empty(None),
            mekuto: None,
            effective: Config::default(),
            issues: Vec::new(),
            themes: Vec::new(),
            env_theme: None,
            generation: 0,
        };
        s.rebuild();
        s
    }

    /// Read the global file and, when a mekuto is open, its `.meku` file.
    pub fn load(root: Option<&Path>) -> Self {
        let mut s = Self {
            global: global_path().map_or_else(|| Layer::empty(None), Layer::load),
            mekuto: root.map(|r| Layer::load(mekuto_path(r))),
            effective: Config::default(),
            issues: Vec::new(),
            themes: Vec::new(),
            env_theme: None,
            generation: 0,
        };
        s.rebuild();
        s
    }

    pub fn layer(&self, scope: Scope) -> Option<&Layer> {
        match scope {
            Scope::Global => Some(&self.global),
            Scope::Mekuto => self.mekuto.as_ref(),
        }
    }

    fn layer_mut(&mut self, scope: Scope) -> Option<&mut Layer> {
        match scope {
            Scope::Global => Some(&mut self.global),
            Scope::Mekuto => self.mekuto.as_mut(),
        }
    }

    /// Switch to another mekuto (or none) and re-merge.
    pub fn set_mekuto(&mut self, root: Option<&Path>) {
        self.mekuto = root.map(|r| Layer::load(mekuto_path(r)));
        self.rebuild();
    }

    /// Recompute `effective`, `issues` and `themes` from the two layers.
    pub fn rebuild(&mut self) {
        let mut effective = Config::default();
        let mut issues = Vec::new();
        let mut themes: Vec<ThemeSpec> = Vec::new();
        for scope in [Scope::Global, Scope::Mekuto] {
            let Some(layer) = self.layer(scope) else {
                continue;
            };
            if let Some(err) = &layer.error {
                issues.push(Issue {
                    scope,
                    key: None,
                    message: format!("not valid JSON — {err}"),
                    fatal: true,
                });
                continue;
            }
            for (key, value) in &layer.values {
                if key == "themes" {
                    for t in parse_themes(scope, value, &mut issues) {
                        // A later (mekuto) theme replaces an earlier one by name.
                        themes.retain(|e| !e.name.eq_ignore_ascii_case(&t.name));
                        themes.push(t);
                    }
                    continue;
                }
                if EXTRA_KEYS.contains(&key.as_str()) {
                    continue;
                }
                let Some(def) = def_for(key) else {
                    issues.push(Issue {
                        scope,
                        key: Some(key.clone()),
                        message: "unknown setting (ignored)".into(),
                        fatal: false,
                    });
                    continue;
                };
                match validate(def, value) {
                    Ok((v, note)) => {
                        effective.values.insert(def.key, v);
                        if let Some(message) = note {
                            issues.push(Issue {
                                scope,
                                key: Some(key.clone()),
                                message,
                                fatal: false,
                            });
                        }
                    }
                    Err(message) => issues.push(Issue {
                        scope,
                        key: Some(key.clone()),
                        message: format!("{message}; using the default"),
                        fatal: false,
                    }),
                }
            }
        }
        self.generation += 1;
        self.effective = effective;
        self.issues = issues;
        self.themes = themes;
    }

    /// Whether `scope`'s file sets `key`.
    pub fn is_set(&self, scope: Scope, key: &str) -> bool {
        self.layer(scope)
            .is_some_and(|l| l.error.is_none() && l.values.contains_key(key))
    }

    /// Issues about one key (any scope).
    pub fn issues_for(&self, key: &str) -> Vec<&Issue> {
        self.issues
            .iter()
            .filter(|i| i.key.as_deref() == Some(key))
            .collect()
    }

    fn can_write(&self, scope: Scope) -> Result<(), String> {
        match self.layer(scope) {
            None => Err("no mekuto is open".into()),
            Some(l) if l.error.is_some() => Err(format!(
                "the {} file has a JSON error — fix it or back it up and reset it first",
                scope.label()
            )),
            Some(_) => Ok(()),
        }
    }

    /// Set `key` in `scope` and save. Validates first; the file keeps every
    /// other key (and its order) exactly as it was.
    pub fn write(&mut self, scope: Scope, key: &str, value: Value) -> Result<(), String> {
        self.can_write(scope)?;
        if let Some(def) = def_for(key) {
            validate(def, &value).map_err(|m| format!("{}: {m}", def.label))?;
        }
        if let Some(layer) = self.layer_mut(scope) {
            layer.values.insert(key.to_string(), value);
        }
        self.save(scope)
    }

    /// Remove `key` from `scope` (back to the inherited value) and save.
    pub fn clear(&mut self, scope: Scope, key: &str) -> Result<(), String> {
        self.can_write(scope)?;
        if let Some(layer) = self.layer_mut(scope) {
            layer.values.shift_remove(key);
        }
        self.save(scope)
    }

    /// Replace `themes` in `scope` (or remove it when empty) and save.
    pub fn write_themes(&mut self, scope: Scope, themes: Vec<Value>) -> Result<(), String> {
        self.can_write(scope)?;
        if let Some(layer) = self.layer_mut(scope) {
            if themes.is_empty() {
                layer.values.shift_remove("themes");
            } else {
                layer.values.insert("themes".into(), Value::Array(themes));
            }
        }
        self.save(scope)
    }

    /// Write the layer atomically (temp file + rename) and re-merge.
    fn save(&mut self, scope: Scope) -> Result<(), String> {
        let result = match self
            .layer(scope)
            .and_then(|l| l.path.clone().map(|p| (p, l)))
        {
            // In-memory layer: nothing to write (tests, no config dir).
            None => Ok(()),
            Some((path, layer)) => {
                let json = serde_json::to_string_pretty(&Value::Object(layer.values.clone()))
                    .map_err(|e| e.to_string())
                    .map(|j| j + "\n");
                json.and_then(|json| write_atomic(&path, &json))
            }
        };
        if result.is_ok()
            && let Some(layer) = self.layer_mut(scope)
        {
            layer.mtime = layer.disk_mtime();
        }
        self.rebuild();
        result
    }

    /// Move a broken file aside (`settings.json.bak-<time>`) and start a
    /// fresh empty one. Returns the backup's path.
    pub fn backup_and_reset(&mut self, scope: Scope) -> Result<PathBuf, String> {
        let path = self
            .layer(scope)
            .and_then(|l| l.path.clone())
            .ok_or("there is no settings file to reset")?;
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let backup = path.with_extension(format!("json.bak-{stamp}"));
        if path.exists() {
            std::fs::rename(&path, &backup)
                .map_err(|e| format!("could not back up the file: {e}"))?;
        }
        if let Some(layer) = self.layer_mut(scope) {
            *layer = Layer::empty(Some(path));
        }
        self.rebuild();
        Ok(backup)
    }

    /// The values as `scope` sees them: the global file alone for
    /// [`Scope::Global`], global plus the mekuto's overrides for
    /// [`Scope::Mekuto`]. The settings page shows these.
    pub fn effective_for(&self, scope: Scope) -> Config {
        match scope {
            Scope::Mekuto => self.effective.clone(),
            Scope::Global => {
                let mut alone = self.clone();
                alone.mekuto = None;
                alone.rebuild();
                alone.effective
            }
        }
    }

    /// Make sure `scope`'s file exists (an empty object) and return its path,
    /// so it can be opened in an editor.
    pub fn ensure_file(&self, scope: Scope) -> Option<PathBuf> {
        let path = self.layer(scope)?.path.clone()?;
        if !path.exists() {
            write_atomic(&path, "{}\n").ok()?;
        }
        Some(path)
    }

    /// Whether a settings file changed on disk since it was last read.
    pub fn changed_on_disk(&self) -> bool {
        [Scope::Global, Scope::Mekuto]
            .iter()
            .filter_map(|s| self.layer(*s))
            .any(|l| l.path.is_some() && l.disk_mtime() != l.mtime)
    }

    /// Re-read a file that changed on disk (edited by hand, say). Returns
    /// whether anything was reloaded.
    pub fn reload_if_changed(&mut self) -> bool {
        let mut changed = false;
        for scope in [Scope::Global, Scope::Mekuto] {
            let stale = self
                .layer(scope)
                .is_some_and(|l| l.path.is_some() && l.disk_mtime() != l.mtime);
            if stale && let Some(path) = self.layer(scope).and_then(|l| l.path.clone()) {
                if let Some(slot) = self.layer_mut(scope) {
                    *slot = Layer::load(path);
                }
                changed = true;
            }
        }
        if changed {
            self.rebuild();
        }
        changed
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not save {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state_with(global: &str, mekuto: Option<&str>) -> ConfigState {
        let mut s = ConfigState::in_memory();
        s.global.values = parse_layer_text(global).unwrap();
        if let Some(m) = mekuto {
            let mut layer = Layer::empty(None);
            layer.values = parse_layer_text(m).unwrap();
            s.mekuto = Some(layer);
        }
        s.rebuild();
        s
    }

    #[test]
    fn defaults_cover_every_registry_entry_with_valid_values() {
        let c = Config::default();
        for d in DEFS {
            let (v, note) =
                validate(d, &d.default.to_value()).unwrap_or_else(|e| panic!("{}: {e}", d.key));
            assert!(note.is_none(), "{} default is out of range", d.key);
            assert_eq!(c.values[d.key], v, "{}", d.key);
        }
        let mut keys: Vec<_> = DEFS.iter().map(|d| d.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), DEFS.len(), "keys are unique");
        assert!(
            c.bool("auto_close_pairs") && c.f64("zoom") == 1.0 && c.str("default_mode") == "live"
        );
    }

    #[test]
    fn mekuto_overrides_global_overrides_default() {
        let s = state_with(
            r#"{"zoom": 1.2, "autosave": false}"#,
            Some(r#"{"zoom": 1.5, "default_mode": "read"}"#),
        );
        assert_eq!(s.effective.f64("zoom"), 1.5, "mekuto wins");
        assert!(
            !s.effective.bool("autosave"),
            "global applies where the mekuto is silent"
        );
        assert_eq!(s.effective.str("default_mode"), "read");
        assert!(
            s.effective.bool("smart_lists"),
            "untouched keys keep their default"
        );
        assert!(s.issues.is_empty());
        assert!(s.is_set(Scope::Global, "autosave") && !s.is_set(Scope::Mekuto, "autosave"));
    }

    #[test]
    fn global_view_ignores_the_mekuto_layer() {
        let s = state_with(r#"{"zoom": 1.2}"#, Some(r#"{"zoom": 1.5}"#));
        assert_eq!(s.effective_for(Scope::Global).f64("zoom"), 1.2);
        assert_eq!(s.effective_for(Scope::Mekuto).f64("zoom"), 1.5);
    }

    #[test]
    fn bad_values_are_reported_and_the_rest_still_apply() {
        let s = state_with(
            r#"{"zoom": "big", "autosave": "yes", "default_mode": "zen", "caret_blink_ms": 99999, "nope": 1, "smart_lists": false}"#,
            None,
        );
        assert_eq!(s.effective.f64("zoom"), 1.0, "wrong type → default");
        assert!(s.effective.bool("autosave"));
        assert_eq!(
            s.effective.str("default_mode"),
            "live",
            "unknown choice → default"
        );
        assert_eq!(
            s.effective.f64("caret_blink_ms"),
            2000.0,
            "out of range → clamped"
        );
        assert!(
            !s.effective.bool("smart_lists"),
            "a good key beside bad ones still applies"
        );
        let for_key = |k: &str| {
            s.issues_for(k)
                .first()
                .map(|i| i.message.clone())
                .unwrap_or_default()
        };
        assert!(for_key("zoom").contains("expected a number"));
        assert!(for_key("default_mode").contains("edit, live, read"));
        assert!(for_key("caret_blink_ms").contains("using 2000"));
        assert!(for_key("nope").contains("unknown setting"));
        assert!(s.issues.iter().all(|i| !i.fatal));
    }

    #[test]
    fn broken_json_reports_position_and_hint() {
        let cases = [
            (r#"{"zoom": 1.2,}"#, "remove the comma"),
            (r#"{zoom: 1}"#, "double quotes"),
            ("{\n  'zoom': 1\n}", "single quotes"),
            ("{\n  // note\n  \"zoom\": 1\n}", "comments"),
            (r#"{"zoom": 1 "autosave": true}"#, "comma may be missing"),
            (r#"{"zoom": 1"#, "closing bracket"),
        ];
        for (text, hint) in cases {
            let e = parse_layer_text(text).unwrap_err();
            assert!(e.hint.is_some_and(|h| h.contains(hint)), "{text:?} → {e}");
            assert!(e.line >= 1 && e.column >= 1);
        }
        let e = parse_layer_text("{\n  \"a\": 1,\n  \"b\": ,\n}").unwrap_err();
        assert_eq!(e.line, 3, "points at the offending line");
        assert!(parse_layer_text("[1, 2]").is_err(), "must be an object");
        assert!(
            parse_layer_text("   ").unwrap().is_empty(),
            "empty file = nothing set"
        );
    }

    #[test]
    fn a_broken_layer_is_fatal_for_that_layer_only() {
        let mut s = ConfigState::in_memory();
        s.global.values = parse_layer_text(r#"{"zoom": 1.3}"#).unwrap();
        let mut broken = Layer::empty(None);
        broken.error = parse_layer_text("{oops").err();
        s.mekuto = Some(broken);
        s.rebuild();
        assert_eq!(s.effective.f64("zoom"), 1.3, "global still applies");
        let fatal: Vec<_> = s.issues.iter().filter(|i| i.fatal).collect();
        assert_eq!(fatal.len(), 1);
        assert_eq!(fatal[0].scope, Scope::Mekuto);
        assert!(
            s.write(Scope::Mekuto, "zoom", json!(1.4))
                .unwrap_err()
                .contains("JSON error"),
            "never writes over a broken file"
        );
    }

    #[test]
    fn custom_themes_parse_with_per_field_errors() {
        let s = state_with(
            r##"{"themes": [
                {"name": "Ink", "base": "Nord", "dark": true, "bg": "#101010", "accent": "#f80", "string": "nope", "wat": "#fff"},
                {"base": "Nord"},
                "not an object"
            ]}"##,
            Some(
                r##"{"themes": [{"name": "ink", "bg": "#000000"}, {"name": "Paper", "dark": false, "bg": "ffffff"}]}"##,
            ),
        );
        let names: Vec<_> = s.themes.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            ["ink", "Paper"],
            "the mekuto's Ink replaces the global one (case-insensitive)"
        );
        let paper = &s.themes[1];
        assert_eq!(
            paper.colors,
            vec![("bg".to_string(), 0xffffff)],
            "hex without # works"
        );
        let msgs: Vec<_> = s.issues.iter().filter_map(|i| i.key.clone()).collect();
        assert!(msgs.contains(&"themes[0].string".to_string()), "{msgs:?}");
        assert!(msgs.contains(&"themes[0].wat".to_string()));
        assert!(msgs.contains(&"themes[1]".to_string()), "nameless theme");
        assert!(msgs.contains(&"themes[2]".to_string()), "non-object entry");
        assert_eq!(parse_hex("#f80"), Some(0xff8800));
        assert_eq!(parse_hex("#12345"), None);
    }

    #[test]
    fn writes_go_to_disk_keep_other_keys_and_survive_reload() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("g/settings.json");
        let mut s = ConfigState::in_memory();
        s.global = Layer::empty(Some(global.clone()));
        s.write(Scope::Global, "zoom", json!(1.3)).unwrap();
        // The user adds a key by hand; a later save must keep it.
        let mut text = std::fs::read_to_string(&global).unwrap();
        text = text.replace('}', ",\n  \"my_note\": \"keep me\"\n}");
        std::fs::write(&global, text).unwrap();
        assert!(s.reload_if_changed(), "hand edits are picked up");
        assert_eq!(s.effective.f64("zoom"), 1.3);
        s.write(Scope::Global, "autosave", json!(false)).unwrap();
        let saved = std::fs::read_to_string(&global).unwrap();
        assert!(
            saved.contains("my_note") && saved.contains("keep me"),
            "{saved}"
        );
        assert!(
            saved.find("zoom").unwrap() < saved.find("autosave").unwrap(),
            "order kept: {saved}"
        );
        s.clear(Scope::Global, "zoom").unwrap();
        assert_eq!(s.effective.f64("zoom"), 1.0);
        assert!(!std::fs::read_to_string(&global).unwrap().contains("zoom"));
        assert!(
            s.write(Scope::Global, "zoom", json!("x")).is_err(),
            "invalid values are refused"
        );
        assert!(
            s.write(Scope::Mekuto, "zoom", json!(1.1))
                .unwrap_err()
                .contains("no mekuto")
        );
    }

    #[test]
    fn broken_files_can_be_backed_up_and_reset() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".meku")).unwrap();
        let file = mekuto_path(root);
        std::fs::write(&file, "{ \"zoom\": 1.2, }").unwrap();
        let mut s = ConfigState::in_memory();
        s.set_mekuto(Some(root));
        assert!(s.issues.iter().any(|i| i.fatal && i.scope == Scope::Mekuto));
        let backup = s.backup_and_reset(Scope::Mekuto).unwrap();
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "{ \"zoom\": 1.2, }",
            "the broken text is preserved"
        );
        assert!(s.issues.is_empty());
        s.write(Scope::Mekuto, "zoom", json!(1.5)).unwrap();
        assert_eq!(s.effective.f64("zoom"), 1.5);
        assert!(std::fs::read_to_string(&file).unwrap().contains("1.5"));
    }
}
