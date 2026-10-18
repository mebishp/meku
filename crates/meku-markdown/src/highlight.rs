//! Fenced-code syntax highlighting. Pure text logic, no GPUI.
//!
//! Code fences render plain first (the editor never blocks on grammars);
//! [`highlight`] then refines the text of a fence into [`Tok`] spans when the
//! fence declares one of the supported languages (rust, python, js/ts, bash,
//! markdown). Grammars ship their own official highlight queries, compiled
//! once behind [`OnceLock`]; an unknown language (or a query that fails to
//! load) returns `None` and callers keep the plain rendering.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;

use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

/// Highlight names (in index order) that map onto [`Tok`] classes.
const NAMES: [&str; 10] = [
    "comment", "string", "escape", "number", "constant", "property", "function", "type",
    "operator", "keyword",
];

/// Semantic token classes the editor colors. Untokenized bytes stay plain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Comment,
    Str,
    Num,
    Const,
    Prop,
    Func,
    Type,
    Op,
    Keyword,
    Other,
}

fn index_to_tok(index: usize) -> Tok {
    match NAMES.get(index).copied() {
        Some("comment") => Tok::Comment,
        Some("string") | Some("escape") => Tok::Str,
        Some("number") => Tok::Num,
        Some("constant") => Tok::Const,
        Some("property") => Tok::Prop,
        Some("function") => Tok::Func,
        Some("type") => Tok::Type,
        Some("operator") => Tok::Op,
        Some("keyword") => Tok::Keyword,
        _ => Tok::Other,
    }
}

/// Fence language tag (plus common aliases) to a canonical grammar key.
fn canonical(lang: &str) -> Option<&'static str> {
    let l = lang.trim().to_ascii_lowercase();
    let l = l.strip_prefix("rust-").unwrap_or(&l);
    Some(match l {
        "rs" | "rust" => "rust",
        "py" | "python" => "python",
        "js" | "jsx" | "javascript" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" | "typescript" => "javascript",
        "sh" | "bash" | "zsh" | "shell" => "bash",
        "md" | "markdown" => "markdown",
        _ => return None,
    })
}

fn configs() -> &'static HashMap<&'static str, HighlightConfiguration> {
    static CONFIGS: OnceLock<HashMap<&'static str, HighlightConfiguration>> = OnceLock::new();
    CONFIGS.get_or_init(|| {
        let langs: [(
            &'static str,
            tree_sitter::Language,
            &'static str,
            &'static str,
        ); 6] = [
            (
                "rust",
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY,
                tree_sitter_rust::INJECTIONS_QUERY,
            ),
            (
                "python",
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY,
                "",
            ),
            (
                "javascript",
                tree_sitter_javascript::LANGUAGE.into(),
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::INJECTIONS_QUERY,
            ),
            (
                "bash",
                tree_sitter_bash::LANGUAGE.into(),
                tree_sitter_bash::HIGHLIGHT_QUERY,
                "",
            ),
            (
                "markdown",
                tree_sitter_md_025::LANGUAGE.into(),
                tree_sitter_md_025::HIGHLIGHT_QUERY_BLOCK,
                tree_sitter_md_025::INJECTION_QUERY_BLOCK,
            ),
            (
                "markdown_inline",
                tree_sitter_md_025::INLINE_LANGUAGE.into(),
                tree_sitter_md_025::HIGHLIGHT_QUERY_INLINE,
                "",
            ),
        ];
        let mut map = HashMap::new();
        for (name, language, highlights, injections) in langs {
            let mut config =
                match HighlightConfiguration::new(language, name, highlights, injections, "") {
                    Ok(config) => config,
                    Err(_) => continue,
                };
            config.configure(&NAMES);
            map.insert(name, config);
        }
        map
    })
}

/// Token spans of one fence, shared between renders and the cache.
pub type Spans = Vec<(Range<usize>, Tok)>;
/// Shareable [`Spans`].
pub type SharedSpans = Arc<Spans>;

/// Highlight a fenced code block once and cache the spans (the same fences
/// repaint every frame). Returns `None` for unknown languages, loads that do
/// not compile, or internal errors; callers keep the plain rendering.
pub fn highlight(lang: &str, code: &str) -> Option<SharedSpans> {
    let canonical = canonical(lang)?;
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache
        .lock()
        .unwrap()
        .get(&(canonical.to_string(), code.to_string()))
    {
        return Some(hit.clone());
    }
    let config = configs().get(canonical)?;
    let spans = Arc::new(run(config, code)?);
    let mut guard = cache.lock().unwrap();
    if guard.len() >= 256 {
        guard.clear();
    }
    guard.insert((canonical.to_string(), code.to_string()), spans.clone());
    Some(spans)
}

type SpanCache = Mutex<HashMap<(String, String), SharedSpans>>;

static CACHE: OnceLock<SpanCache> = OnceLock::new();

fn run(config: &HighlightConfiguration, code: &str) -> Option<Spans> {
    let map = configs();
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(config, code.as_bytes(), None, |injected| match injected {
            "markdown_inline" => map.get("markdown_inline"),
            other => canonical(other).and_then(|key| map.get(key)),
        })
        .ok()?;
    let mut out: Vec<(Range<usize>, Tok)> = Vec::new();
    let mut active: Option<Tok> = None;
    for event in events {
        let event = event.ok()?;
        match event {
            HighlightEvent::HighlightStart(Highlight(index)) => {
                active = Some(index_to_tok(index));
            }
            HighlightEvent::HighlightEnd => active = None,
            HighlightEvent::Source { start, end } => {
                if let (Some(token), true) = (active, start < end && end <= code.len()) {
                    if let Some(last) = out.last_mut()
                        && last.0.end == start
                        && last.1 == token
                    {
                        last.0.end = end;
                    } else {
                        out.push((start..end, token));
                    }
                }
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(lang: &str, code: &str) -> Vec<(String, Tok)> {
        highlight(lang, code)
            .expect("supported language")
            .iter()
            .map(|(r, t)| (code[r.clone()].to_string(), *t))
            .collect()
    }

    #[test]
    fn rust_classifies_common_tokens() {
        let code = "let x: u32 = 42; // set\nfn main() { \"hi\" }\n";
        let got = tokens("rust", code);
        assert!(got.iter().any(|(t, k)| t == "let" && *k == Tok::Keyword));
        assert!(got.iter().any(|(t, k)| t == "u32" && *k == Tok::Type));
        assert!(got.iter().any(|(t, k)| t == "42" && *k == Tok::Const));
        assert!(got.iter().any(|(t, k)| t == "// set" && *k == Tok::Comment));
        assert!(got.iter().any(|(t, k)| t == "\"hi\"" && *k == Tok::Str));
    }

    #[test]
    fn wellformed_spans_do_not_overlap() {
        let code = "struct S { a: u8 } // x\nlet s = \"\"; let b = 1.5;\n";
        let spans = highlight("rust", code).expect("rust config");
        for span in spans.iter() {
            assert!(span.0.end <= code.len());
            assert!(code.is_char_boundary(span.0.start));
            assert!(code.is_char_boundary(span.0.end));
        }
        for pair in spans.windows(2) {
            assert!(pair[0].0.end <= pair[1].0.start, "overlap: {pair:?}");
        }
    }

    #[test]
    fn malformed_code_never_panics() {
        for code in [
            "",
            "\"",
            "let x = (",
            "// unterminated\nmatc",
            "fn 🦀() { let s = 'a; }\0",
        ] {
            let _ = highlight("rust", code).expect("rust config");
        }
    }

    #[test]
    fn aliases_map_to_canonical_grammars() {
        assert!(highlight("rs", "fn f() {}").is_some());
        assert!(highlight("PYTHON", "if True:\n    pass\n").is_some());
        assert!(highlight("ts", "const a = 1;").is_some());
        assert!(highlight("sh", "echo hi").is_some());
    }

    #[test]
    fn unknown_lang_yields_none() {
        assert_eq!(highlight("cobol", "MOVE 1 TO X"), None);
        assert_eq!(highlight("", "text"), None);
    }

    #[test]
    fn python_js_bash_classify() {
        let py = "def hi(x):\n    return x + 1.5 # done\n";
        let got = tokens("python", py);
        assert!(got.iter().any(|(t, k)| t == "hi" && *k == Tok::Func));
        assert!(got.iter().any(|(t, k)| t == "# done" && *k == Tok::Comment));

        let js = "function go() { const n = 1.5; }\n";
        let got = tokens("javascript", js);
        assert!(got.iter().any(|(t, k)| t == "go" && *k == Tok::Func));

        let sh = "# run\nif [ -n \"$x\" ]; then echo ok; fi\n";
        let got = tokens("bash", sh);
        assert!(got.iter().any(|(t, k)| t == "# run" && *k == Tok::Comment));
        assert!(got.iter().any(|(_, k)| *k == Tok::Keyword));
    }

    #[test]
    fn cache_returns_same_spans() {
        let a = highlight("rust", "let q = 9;");
        let b = highlight("rust", "let q = 9;");
        assert!(a.is_some() && b.is_some() && a.is_some() && a == b);
    }

    #[test]
    fn markdown_loads_with_inline_injections() {
        let code = "# Title\n\n```rs\nlet x = 1;\n```\n";
        let got = highlight("markdown", code).expect("markdown config");
        assert!(!got.is_empty());
        // The injected rust fence is highlighted by its info string.
        assert!(
            got.iter()
                .any(|(r, t)| t == &Tok::Keyword && &code[r.clone()] == "let"),
            "injected rust should highlight: {got:?}"
        );
    }
}
