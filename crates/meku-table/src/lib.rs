//! `meku-table`: GFM pipe-table text logic. Pure strings, no dependencies.
//!
//! A table is a header line, a delimiter line (`|---|:-:|`), then body lines.
//! Cells are split on unescaped `|`; a leading/trailing pipe is optional.

use std::ops::Range;

/// Column alignment from the delimiter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    None,
    Left,
    Center,
    Right,
}

/// Byte ranges (relative to `line`) of each cell's trimmed content. Escaped
/// pipes (`\|`) and pipes inside backtick spans do not split.
pub fn split_cells(line: &str) -> Vec<Range<usize>> {
    let bytes = line.as_bytes();
    // Split points: unescaped, non-code pipes.
    let mut seps: Vec<usize> = Vec::new();
    let mut in_code = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1, // skip the escaped byte
            b'`' => in_code = !in_code,
            b'|' if !in_code => seps.push(i),
            _ => {}
        }
        i += 1;
    }
    let mut bounds: Vec<(usize, usize)> = Vec::new(); // raw cell spans
    let mut start = 0;
    for &p in &seps {
        bounds.push((start, p));
        start = p + 1;
    }
    bounds.push((start, line.len()));
    // A pipe before the first cell (or after the last) is decoration: it
    // leaves an empty edge span that is not a real cell.
    if seps.first().is_some_and(|&p| line[..p].trim().is_empty()) {
        bounds.remove(0);
    }
    if bounds.len() > 1
        && seps
            .last()
            .is_some_and(|&p| line[p + 1..].trim().is_empty())
    {
        bounds.pop();
    }
    bounds
        .into_iter()
        .map(|(a, b)| {
            let raw = &line[a..b];
            let lead = raw.len() - raw.trim_start().len();
            let content = raw.trim();
            a + lead..a + lead + content.len()
        })
        .collect()
}

/// A delimiter row: every cell is dashes with optional alignment colons.
pub fn is_delimiter_row(line: &str) -> bool {
    if !line.contains('-') {
        return false;
    }
    let cells = split_cells(line);
    !cells.is_empty()
        && cells.iter().all(|r| {
            let c = &line[r.clone()];
            let inner = c.trim_start_matches(':').trim_end_matches(':');
            !inner.is_empty() && inner.bytes().all(|b| b == b'-')
        })
}

/// Alignments declared by a delimiter row.
pub fn parse_aligns(delim: &str) -> Vec<Align> {
    split_cells(delim)
        .into_iter()
        .map(|r| {
            let c = &delim[r];
            match (c.starts_with(':'), c.ends_with(':')) {
                (true, true) => Align::Center,
                (true, false) => Align::Left,
                (false, true) => Align::Right,
                _ => Align::None,
            }
        })
        .collect()
}

/// Whether `header` + `delim` start a table (same, non-zero column count).
pub fn starts_table(header: &str, delim: &str) -> bool {
    header.contains('|')
        && is_delimiter_row(delim)
        && split_cells(header).len() == split_cells(delim).len()
}

fn width(s: &str) -> usize {
    s.chars().count()
}

/// Re-emit a table with every column padded to its widest cell. `lines[1]`
/// must be the delimiter row. Leading indent of the first line is kept.
pub fn format_table(lines: &[&str]) -> Vec<String> {
    let indent: String = lines
        .first()
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).collect())
        .unwrap_or_default();
    let delim_ix = 1.min(lines.len().saturating_sub(1));
    let aligns = lines.get(1).map(|d| parse_aligns(d)).unwrap_or_default();
    let rows: Vec<Vec<&str>> = lines
        .iter()
        .map(|l| split_cells(l).into_iter().map(|r| &l[r]).collect())
        .collect();
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![3usize; cols];
    for (i, row) in rows.iter().enumerate() {
        if i == delim_ix && lines.len() > 1 {
            continue;
        }
        for (c, cell) in row.iter().enumerate() {
            widths[c] = widths[c].max(width(cell));
        }
    }
    let pad = |cell: &str, w: usize, a: Align| -> String {
        let fill = w.saturating_sub(width(cell));
        match a {
            Align::Right => format!("{}{cell}", " ".repeat(fill)),
            Align::Center => {
                let left = fill / 2;
                format!("{}{cell}{}", " ".repeat(left), " ".repeat(fill - left))
            }
            _ => format!("{cell}{}", " ".repeat(fill)),
        }
    };
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let cells: Vec<String> = (0..cols)
                .map(|c| {
                    let a = aligns.get(c).copied().unwrap_or_default();
                    if i == delim_ix && lines.len() > 1 {
                        let w = widths[c];
                        match a {
                            Align::Left => format!(":{}", "-".repeat(w - 1)),
                            Align::Right => format!("{}:", "-".repeat(w - 1)),
                            Align::Center => format!(":{}:", "-".repeat(w - 2)),
                            Align::None => "-".repeat(w),
                        }
                    } else {
                        pad(row.get(c).copied().unwrap_or(""), widths[c], a)
                    }
                })
                .collect();
            format!("{indent}| {} |", cells.join(" | "))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(line: &str) -> Vec<&str> {
        split_cells(line).into_iter().map(|r| &line[r]).collect()
    }

    #[test]
    fn splits_with_and_without_edge_pipes() {
        assert_eq!(cells("| a | b |"), ["a", "b"]);
        assert_eq!(cells("a | b"), ["a", "b"]);
        assert_eq!(cells("|a|b|c"), ["a", "b", "c"]);
        assert_eq!(
            cells("| a |  | c |"),
            ["a", "", "c"],
            "empty middle cell kept"
        );
        assert_eq!(cells("|  |"), [""], "one empty cell");
    }

    #[test]
    fn escaped_and_code_pipes_do_not_split() {
        assert_eq!(cells(r"| a \| b | c |"), [r"a \| b", "c"]);
        assert_eq!(cells("| `x|y` | z |"), ["`x|y`", "z"]);
    }

    #[test]
    fn ranges_point_at_the_trimmed_content() {
        let line = "|  hi  | yo |";
        let r = split_cells(line);
        assert_eq!(&line[r[0].clone()], "hi");
        assert_eq!(&line[r[1].clone()], "yo");
    }

    #[test]
    fn delimiter_rows() {
        assert!(is_delimiter_row("|---|:--:|--:|"));
        assert!(is_delimiter_row("--- | ---"));
        assert!(!is_delimiter_row("| a | b |"));
        assert!(!is_delimiter_row("| - - |"));
        assert_eq!(
            parse_aligns("|:--|:-:|--:|---|"),
            [Align::Left, Align::Center, Align::Right, Align::None]
        );
    }

    #[test]
    fn table_start_needs_matching_columns() {
        assert!(starts_table("| a | b |", "|---|---|"));
        assert!(!starts_table("| a | b |", "|---|"));
        assert!(!starts_table("just text", "|---|"));
    }

    #[test]
    fn formatting_aligns_columns() {
        let out = format_table(&["|Name|Score|", "|-|--:|", "| Ada | 42 |", "|Linus|7|"]);
        assert_eq!(
            out,
            [
                "| Name  | Score |",
                "| ----- | ----: |",
                "| Ada   |    42 |",
                "| Linus |     7 |",
            ]
        );
    }

    #[test]
    fn formatting_pads_short_rows_and_centers() {
        let out = format_table(&["| a | b |", "|:-:|:-:|", "| x |"]);
        assert_eq!(out[2], "|  x  |     |");
        assert_eq!(out[1], "| :-: | :-: |");
    }
}
