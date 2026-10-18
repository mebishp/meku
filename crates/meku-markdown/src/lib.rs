//! `meku-markdown`: pure Markdown → styled-block pipeline.
//!
//! [`pulldown_cmark::Parser`] events plus [`into_offset_iter`](pulldown_cmark::Parser::into_offset_iter)
//! byte ranges become [`StyledDoc`]: a flat block list the GPUI editor maps
//! cursors against. No filesystem, no GPUI — fully unit-testable.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Byte span into the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    Para,
    Heading(u8),
    CodeFence(Option<String>),
    List(bool),
    Quote,
    Hr,
    Table,
}

/// Bit flags so a span can carry combined styles (e.g. strong link).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InlineStyle {
    bits: u8,
}

impl InlineStyle {
    pub const STRONG: u8 = 0b0000_0001;
    pub const EMPHASIS: u8 = 0b0000_0010;
    pub const STRIKE: u8 = 0b0000_0100;
    pub const CODE: u8 = 0b0000_1000;
    pub const LINK: u8 = 0b0001_0000;

    pub fn empty() -> Self {
        Self { bits: 0 }
    }

    pub fn with(mut self, flag: u8) -> Self {
        self.bits |= flag;
        self
    }

    pub fn contains(self, flag: u8) -> bool {
        self.bits & flag != 0
    }

    pub fn is_plain(self) -> bool {
        self.bits == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledSpan {
    pub start: usize,
    pub end: usize,
    pub style: InlineStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledBlock {
    pub kind: BlockKind,
    pub span: SourceSpan,
    pub spans: Vec<StyledSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub offset: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyledDoc {
    pub version: u64,
    pub blocks: Vec<StyledBlock>,
    pub headings: Vec<Heading>,
}

/// Single-surface live-preview model (rows, roles, hidden/reveal projection).
pub mod live;

/// Fenced-code syntax highlighting into [`crate::highlight::Tok`] spans.
pub mod highlight;

pub fn parser_options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

struct OpenBlock {
    kind: BlockKind,
    tag: BlockTag,
    start: usize,
    spans: Vec<StyledSpan>,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockTag {
    Para,
    Heading,
    Code,
    List,
    Quote,
    Table,
    Html,
}

/// Classify a start-tag: typed block opener, or `None` for inline/structure
/// tags that only contribute spans to the enclosing block.
fn block_tag(tag: &Tag<'_>) -> Option<(BlockTag, BlockKind)> {
    match tag {
        Tag::Paragraph => Some((BlockTag::Para, BlockKind::Para)),
        Tag::Heading { level, .. } => Some((BlockTag::Heading, BlockKind::Heading(*level as u8))),
        Tag::CodeBlock(kind) => {
            let lang = match kind {
                pulldown_cmark::CodeBlockKind::Fenced(info) => {
                    let lang = info.split_whitespace().next().unwrap_or("");
                    (!lang.is_empty()).then(|| lang.to_string())
                }
                pulldown_cmark::CodeBlockKind::Indented => None,
            };
            Some((BlockTag::Code, BlockKind::CodeFence(lang)))
        }
        Tag::List(ordered) => Some((BlockTag::List, BlockKind::List(ordered.is_some()))),
        Tag::BlockQuote(_) => Some((BlockTag::Quote, BlockKind::Quote)),
        Tag::Table(_) => Some((BlockTag::Table, BlockKind::Table)),
        Tag::HtmlBlock => Some((BlockTag::Html, BlockKind::Para)),
        _ => None,
    }
}

fn end_tag_matches(tag: BlockTag, end: &TagEnd) -> bool {
    matches!(
        (tag, end),
        (BlockTag::Para, TagEnd::Paragraph)
            | (BlockTag::Heading, TagEnd::Heading(_))
            | (BlockTag::Code, TagEnd::CodeBlock)
            | (BlockTag::List, TagEnd::List(_))
            | (BlockTag::Quote, TagEnd::BlockQuote(_))
            | (BlockTag::Table, TagEnd::Table)
            | (BlockTag::Html, TagEnd::HtmlBlock)
    )
}

/// Push a styled span for `range`, first emitting a bullet span when a list
/// item just opened (so `- `/`1. ` markers survive in rendered output).
/// Out-of-range inputs are skipped; parsing never panics.
fn push_span(
    current: &mut OpenBlock,
    text: &str,
    pending_item: &mut Option<usize>,
    range: std::ops::Range<usize>,
    style: InlineStyle,
    content: &str,
) {
    if range.start > range.end || range.end > text.len() {
        pending_item.take();
        return;
    }
    if let Some(item_start) = pending_item.take()
        && item_start <= range.start
        && range.start - item_start <= 12
        && let Some(bullet) = text.get(item_start..range.start)
        && is_bullet(bullet)
    {
        current.spans.push(StyledSpan {
            start: item_start,
            end: range.start,
            style: InlineStyle::empty(),
        });
    }
    if text.get(range.clone()).is_none() {
        return;
    }
    current.spans.push(StyledSpan {
        start: range.start,
        end: range.end,
        style,
    });
    current.text.push_str(content);
}

/// True for list-item markers (`- `, `* `, `+ `, `1. `, `2) `), including
/// leading indentation so nested items keep their offset.
fn is_bullet(slice: &str) -> bool {
    let trimmed = slice.trim_start_matches([' ', '\t']);
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return true;
    }
    let digits = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 && digits < 10 {
        let rest = &trimmed[digits..];
        return rest.starts_with(". ") || rest.starts_with(") ");
    }
    false
}

/// Parse Markdown into styled blocks. Never panics on hostile input;
/// unclosed constructs simply run to end of input.
pub fn parse(text: &str, version: u64) -> StyledDoc {
    let parser = Parser::new_ext(text, parser_options()).into_offset_iter();
    let mut doc = StyledDoc {
        version,
        ..StyledDoc::default()
    };
    let mut open: Option<OpenBlock> = None;
    let mut inline: Vec<u8> = Vec::new();
    let mut pending_item: Option<usize> = None;

    for (event, range) in parser {
        match event {
            Event::Start(tag) => {
                if let Some((block_tag, kind)) = block_tag(&tag) {
                    ensure_block(&mut open, block_tag, kind, range.start);
                } else {
                    // Inline/structure tag: push its style bit.
                    let bit = match tag {
                        Tag::Strong => Some(InlineStyle::STRONG),
                        Tag::Emphasis => Some(InlineStyle::EMPHASIS),
                        Tag::Strikethrough => Some(InlineStyle::STRIKE),
                        Tag::Link { .. } => Some(InlineStyle::LINK),
                        _ => None,
                    };
                    if let Some(bit) = bit {
                        inline.push(bit);
                    }
                    // A list item opening inside a list block arms bullet
                    // capture for the item's first content span. A whitespace
                    // gap since the previous span (newlines, nesting indent)
                    // is kept verbatim; anything else is markup and skipped.
                    if matches!(tag, Tag::Item)
                        && open.as_ref().is_some_and(|b| b.tag == BlockTag::List)
                    {
                        if let Some(current) = open.as_mut() {
                            let gap_start = current.spans.last().map(|s| s.end);
                            if let Some(start) = gap_start {
                                let end = range.start.min(text.len());
                                let keep = end > start
                                    && text.get(start..end).is_some_and(|gap| {
                                        !gap.is_empty()
                                            && gap
                                                .bytes()
                                                .all(|b| b == b'\n' || b == b'\r' || b == b' ')
                                    });
                                if keep {
                                    current.spans.push(StyledSpan {
                                        start,
                                        end,
                                        style: InlineStyle::empty(),
                                    });
                                }
                            }
                        }
                        pending_item = Some(range.start);
                    }
                    // Stray inline tag with no block: open a paragraph so its
                    // text is not dropped (e.g. tight-list edge cases).
                    if open.is_none() {
                        ensure_block(&mut open, BlockTag::Para, BlockKind::Para, range.start);
                    }
                }
            }
            Event::End(end) => {
                if let Some(current) = open.take() {
                    if end_tag_matches(current.tag, &end) {
                        let end = range.end.min(text.len()).max(current.start);
                        close_block(&mut doc, current, end);
                    } else {
                        open = Some(current);
                    }
                }
                if matches!(end, TagEnd::Item) {
                    pending_item = None;
                    // Keep the item's trailing newline (tight lists have no
                    // break event between items), but only when it is pure
                    // whitespace — never markup.
                    if let Some(current) = open.as_mut() {
                        let tail_start = current.spans.last().map(|s| s.end);
                        if let Some(start) = tail_start {
                            let end = range.end.min(text.len());
                            let whitespace_tail = end > start
                                && text.get(start..end).is_some_and(|tail| {
                                    !tail.is_empty()
                                        && tail.bytes().all(|b| b == b'\n' || b == b'\r')
                                });
                            if whitespace_tail {
                                current.spans.push(StyledSpan {
                                    start,
                                    end,
                                    style: InlineStyle::empty(),
                                });
                            }
                        }
                    }
                }
                // Pop one inline bit for the matching inline end tag.
                if matches!(
                    end,
                    TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link
                ) {
                    inline.pop();
                }
            }
            Event::Text(content) => {
                if open.is_none() {
                    ensure_block(&mut open, BlockTag::Para, BlockKind::Para, range.start);
                }
                if let Some(current) = open.as_mut() {
                    let mut style = InlineStyle::empty();
                    for &bit in &inline {
                        style = style.with(bit);
                    }
                    if !content.is_empty() {
                        push_span(
                            current,
                            text,
                            &mut pending_item,
                            range.start..range.end,
                            style,
                            &content,
                        );
                    } else {
                        pending_item.take();
                    }
                }
            }
            Event::Code(content) => {
                if open.is_none() {
                    ensure_block(&mut open, BlockTag::Para, BlockKind::Para, range.start);
                }
                if let Some(current) = open.as_mut() {
                    let mut style = InlineStyle::empty().with(InlineStyle::CODE);
                    for &bit in &inline {
                        style = style.with(bit);
                    }
                    push_span(
                        current,
                        text,
                        &mut pending_item,
                        range.start..range.end,
                        style,
                        &content,
                    );
                }
            }
            Event::TaskListMarker(_) => {
                if let Some(current) = open.as_mut() {
                    // Swallow following spaces so "[ ]todo" can't happen.
                    let mut end = range.end;
                    while text.as_bytes().get(end) == Some(&b' ') {
                        end += 1;
                    }
                    push_span(
                        current,
                        text,
                        &mut pending_item,
                        range.start..end,
                        InlineStyle::empty().with(InlineStyle::CODE),
                        "",
                    );
                }
            }
            Event::Rule => {
                // A rule always stands alone; flush any open block first.
                if let Some(current) = open.take() {
                    close_block(&mut doc, current, range.start);
                }
                doc.blocks.push(StyledBlock {
                    kind: BlockKind::Hr,
                    span: SourceSpan {
                        start: range.start,
                        end: range.end,
                    },
                    spans: Vec::new(),
                });
            }
            Event::SoftBreak | Event::HardBreak => {
                if open.is_none() {
                    ensure_block(&mut open, BlockTag::Para, BlockKind::Para, range.start);
                }
                if let Some(current) = open.as_mut() {
                    // Record the break so rendered text keeps its newlines.
                    push_span(
                        current,
                        text,
                        &mut pending_item,
                        range.start..range.end,
                        InlineStyle::empty(),
                        "\n",
                    );
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                if open.is_none() {
                    ensure_block(&mut open, BlockTag::Para, BlockKind::Para, range.start);
                }
                if let Some(current) = open.as_mut() {
                    current.spans.push(StyledSpan {
                        start: range.start,
                        end: range.end,
                        style: InlineStyle::empty(),
                    });
                }
            }
            _ => {}
        }
    }

    if let Some(current) = open.take() {
        let end = text.len();
        close_block(&mut doc, current, end);
    }
    doc
}

fn ensure_block(open: &mut Option<OpenBlock>, tag: BlockTag, kind: BlockKind, start: usize) {
    if open.is_none() {
        *open = Some(OpenBlock {
            kind,
            tag,
            start,
            spans: Vec::new(),
            text: String::new(),
        });
    }
}

fn close_block(doc: &mut StyledDoc, block: OpenBlock, end: usize) {
    let end = end.max(block.start);
    if let BlockKind::Heading(level) = block.kind {
        doc.headings.push(Heading {
            level,
            text: block.text.trim().to_string(),
            offset: block.start,
        });
    }
    doc.blocks.push(StyledBlock {
        kind: block.kind,
        span: SourceSpan {
            start: block.start,
            end,
        },
        spans: block.spans,
    });
}

/// Last-resort doc: one plain paragraph. The editor renders this instead of
/// ever going blank.
pub fn parse_fallback(text: &str, version: u64) -> StyledDoc {
    StyledDoc {
        version,
        blocks: vec![StyledBlock {
            kind: BlockKind::Para,
            span: SourceSpan {
                start: 0,
                end: text.len(),
            },
            spans: Vec::new(),
        }],
        headings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_doc_has_no_blocks() {
        let doc = parse("", 0);
        assert!(doc.blocks.is_empty());
        assert!(doc.headings.is_empty());
        assert_eq!(doc.version, 0);
    }

    #[test]
    fn headings_levels_offsets_text() {
        let text = "# Hello\n\nbody\n\n### Deep dive\n";
        let doc = parse(text, 7);
        assert_eq!(doc.version, 7);
        assert_eq!(doc.headings.len(), 2);
        assert_eq!(doc.headings[0].level, 1);
        assert_eq!(doc.headings[0].text, "Hello");
        assert_eq!(doc.headings[0].offset, 0);
        assert_eq!(doc.headings[1].level, 3);
        assert_eq!(doc.headings[1].offset, "# Hello\n\nbody\n\n".len());
        assert!(matches!(doc.blocks[0].kind, BlockKind::Heading(1)));
    }

    #[test]
    fn strong_span_byte_offsets() {
        let text = "a **bold** tail\n";
        let doc = parse(text, 0);
        assert_eq!(doc.blocks.len(), 1);
        let strong: Vec<_> = doc.blocks[0]
            .spans
            .iter()
            .filter(|s| s.style.contains(InlineStyle::STRONG))
            .collect();
        assert_eq!(strong.len(), 1);
        assert_eq!(&text[strong[0].start..strong[0].end], "bold");
    }

    #[test]
    fn emphasis_strike_code_styles() {
        let doc = parse("*em* ~~gone~~ `code`\n", 0);
        let kinds: Vec<u8> = doc.blocks[0]
            .spans
            .iter()
            .filter(|s| !s.style.is_plain())
            .map(|s| {
                let st = s.style;
                if st.contains(InlineStyle::EMPHASIS) {
                    1
                } else if st.contains(InlineStyle::STRIKE) {
                    2
                } else if st.contains(InlineStyle::CODE) {
                    3
                } else {
                    0
                }
            })
            .collect();
        assert_eq!(kinds, vec![1, 2, 3]);
    }

    #[test]
    fn task_list_and_code_fence() {
        let text = "- [ ] todo\n- [x] done\n\n```rust\nlet x = 1;\n```\n";
        let doc = parse(text, 0);
        assert!(matches!(doc.blocks[0].kind, BlockKind::List(false)));
        let markers = doc.blocks[0]
            .spans
            .iter()
            .filter(|s| s.style.contains(InlineStyle::CODE))
            .count();
        assert!(markers >= 2, "task markers recorded as spans");
        let fence = doc.blocks.iter().find_map(|b| match &b.kind {
            BlockKind::CodeFence(lang) => Some(lang.clone()),
            _ => None,
        });
        assert_eq!(fence, Some(Some("rust".to_string())));
    }

    #[test]
    fn table_block_present() {
        let doc = parse("| a | b |\n|---|---|\n| 1 | 2 |\n", 0);
        assert!(doc.blocks.iter().any(|b| b.kind == BlockKind::Table));
    }

    #[test]
    fn unclosed_fence_does_not_panic() {
        let doc = parse("# T\n\n```rust\nlet x = 1;\n", 0);
        assert!(!doc.blocks.is_empty());
        for block in &doc.blocks {
            assert!(block.span.start <= block.span.end);
            for span in &block.spans {
                assert!(span.start <= span.end);
            }
        }
    }

    #[test]
    fn fallback_is_single_para() {
        let doc = parse_fallback("anything\nat all", 3);
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].kind, BlockKind::Para);
        assert_eq!(doc.blocks[0].span.end, "anything\nat all".len());
    }

    fn rendered_text(text: &str, block: &StyledBlock) -> String {
        block.spans.iter().map(|s| &text[s.start..s.end]).collect()
    }

    #[test]
    fn list_bullets_survive_in_spans() {
        let text = "- a\n  - b\n";
        let doc = parse(text, 0);
        assert_eq!(doc.blocks.len(), 1);
        assert!(matches!(doc.blocks[0].kind, BlockKind::List(false)));
        let rendered = rendered_text(text, &doc.blocks[0]);
        assert!(rendered.contains("- a"), "bullet kept: {rendered:?}");
        assert!(
            rendered.contains("  - b"),
            "nested indent kept: {rendered:?}"
        );

        let text = "1. c\n2. d\n";
        let doc = parse(text, 0);
        assert!(matches!(doc.blocks[0].kind, BlockKind::List(true)));
        let rendered = rendered_text(text, &doc.blocks[0]);
        assert!(
            rendered.contains("1. c"),
            "ordered marker kept: {rendered:?}"
        );
    }

    #[test]
    fn soft_breaks_keep_newlines() {
        let text = "a\nb\n";
        let doc = parse(text, 0);
        assert_eq!(doc.blocks.len(), 1);
        let rendered = rendered_text(text, &doc.blocks[0]);
        // Interior newline kept; the terminal one has no event (invisible).
        assert_eq!(rendered, "a\nb");
    }

    #[test]
    fn task_items_keep_bullet_and_marker() {
        let text = "- [ ] todo\n";
        let doc = parse(text, 0);
        let rendered = rendered_text(text, &doc.blocks[0]);
        assert_eq!(rendered, "- [ ] todo\n");
    }
}
