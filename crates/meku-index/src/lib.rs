//! `meku-index`: cheap derived cache over a mekuto.
//!
//! Keeps per-file metadata (title, headings) and fuzzy file matching.
//! No GPUI dependency; content is pushed in via [`FileIndex::upsert`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use meku_vault::VaultFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    /// Byte offset of the heading line's start in the source.
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct FileMeta {
    pub title: String,
    pub headings: Vec<Heading>,
    pub mtime: SystemTime,
}

#[derive(Debug, Default)]
pub struct FileIndex {
    files: HashMap<PathBuf, FileMeta>,
}

impl FileIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn get(&self, rel: &Path) -> Option<&FileMeta> {
        self.files.get(rel)
    }

    /// Rebuild from a vault scan, reading each file from disk.
    /// Unreadable files are skipped, never fatal.
    pub fn rebuild(&mut self, files: &[VaultFile]) {
        self.files.clear();
        for file in files {
            let text = match std::fs::read_to_string(&file.abs) {
                Ok(text) => text,
                Err(_) => continue,
            };
            self.upsert(&file.rel, &text, file.mtime);
        }
    }

    pub fn upsert(&mut self, rel: &Path, text: &str, mtime: SystemTime) {
        let (title, headings) = extract_headings(text);
        let title = if title.is_empty() {
            rel.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string()
        } else {
            title
        };
        self.files.insert(
            rel.to_path_buf(),
            FileMeta {
                title,
                headings,
                mtime,
            },
        );
    }

    pub fn remove(&mut self, rel: &Path) {
        self.files.remove(rel);
    }

    /// Fuzzy file match (for Quick Switcher). Case-insensitive subsequence
    /// match scored towards contiguous, early, filename-relative matches.
    pub fn fuzzy_match(&self, query: &str, limit: usize) -> Vec<PathBuf> {
        if query.is_empty() {
            let mut all: Vec<PathBuf> = self.files.keys().cloned().collect();
            all.sort();
            all.truncate(limit);
            return all;
        }
        let mut scored: Vec<(u64, PathBuf)> = self
            .files
            .keys()
            .filter_map(|rel| {
                fuzzy_score(query, &rel.to_string_lossy()).map(|score| (score, rel.clone()))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.truncate(limit);
        scored.into_iter().map(|(_, rel)| rel).collect()
    }
}

/// First `#`-heading wins as title; fenced code blocks are not headings.
fn extract_headings(text: &str) -> (String, Vec<Heading>) {
    let mut title = String::new();
    let mut headings = Vec::new();
    let mut in_fence = false;
    let mut offset = 0usize;

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if !in_fence && trimmed.starts_with('#') {
            let level = trimmed.bytes().take_while(|&b| b == b'#').count();
            if (1..=6).contains(&level) && trimmed.as_bytes().get(level) == Some(&b' ') {
                let heading_text = trimmed[level + 1..].trim_end().to_string();
                if title.is_empty() {
                    title = heading_text.clone();
                }
                headings.push(Heading {
                    level: level as u8,
                    text: heading_text,
                    offset,
                });
            }
        }
        offset += line.len();
    }
    (title, headings)
}

fn fuzzy_score(query: &str, candidate: &str) -> Option<u64> {
    let query: Vec<char> = query.to_lowercase().chars().collect();
    let candidate_lower: Vec<char> = candidate.to_lowercase().chars().collect();
    if query.is_empty() {
        return Some(0);
    }

    // Subsequence scan, tracking runs of contiguous matches.
    let mut score: u64 = 0;
    let mut qi = 0;
    let mut run: u64 = 0;
    let mut first_at: Option<usize> = None;
    for (ci, &c) in candidate_lower.iter().enumerate() {
        if qi < query.len() && c == query[qi] {
            if first_at.is_none() {
                first_at = Some(ci);
            }
            run += 1;
            score += 10 + run * run; // contiguous runs win big
                                     // Bonus for match right after a path separator or at start.
            if ci == 0 || candidate_lower[ci - 1] == '/' {
                score += 15;
            }
            qi += 1;
        } else {
            run = 0;
        }
    }
    if qi != query.len() {
        return None; // not a subsequence
    }
    // Earlier first match and shorter candidates win ties.
    let first = first_at.unwrap_or(candidate_lower.len()) as u64;
    let len = candidate_lower.len().max(1) as u64;
    score += 1000 / (1 + first) + 500 / len;

    // Filename matches beat directory-only matches.
    let file_name = candidate.rsplit('/').next().unwrap_or(candidate);
    if fuzzy_subsequence(
        &query,
        &file_name.to_lowercase().chars().collect::<Vec<_>>(),
    ) {
        score += 200;
    }
    Some(score)
}

fn fuzzy_subsequence(query: &[char], candidate: &[char]) -> bool {
    let mut qi = 0;
    for &c in candidate {
        if qi < query.len() && c == query[qi] {
            qi += 1;
        }
    }
    qi == query.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn mtime() -> SystemTime {
        UNIX_EPOCH
    }

    #[test]
    fn headings_extracted_with_offsets_and_title() {
        let mut index = FileIndex::new();
        let text = "# Hello\n\nbody\n\n## Sub *x*\n";
        index.upsert(Path::new("a.md"), text, mtime());
        let meta = index.get(Path::new("a.md")).unwrap();
        assert_eq!(meta.title, "Hello");
        assert_eq!(meta.headings.len(), 2);
        assert_eq!(meta.headings[0].level, 1);
        assert_eq!(meta.headings[0].offset, 0);
        assert_eq!(meta.headings[1].level, 2);
        assert_eq!(meta.headings[1].offset, "# Hello\n\nbody\n\n".len());
    }

    #[test]
    fn fenced_code_is_not_a_heading() {
        let mut index = FileIndex::new();
        index.upsert(
            Path::new("a.md"),
            "```\n# not a heading\n```\n\n# Real\n",
            mtime(),
        );
        let meta = index.get(Path::new("a.md")).unwrap();
        assert_eq!(meta.headings.len(), 1);
        assert_eq!(meta.title, "Real");
    }

    #[test]
    fn filename_fallback_title() {
        let mut index = FileIndex::new();
        index.upsert(Path::new("notes/plan.md"), "just text\n", mtime());
        assert_eq!(index.get(Path::new("notes/plan.md")).unwrap().title, "plan");
    }

    #[test]
    fn fuzzy_prefers_filename_and_contiguous() {
        let mut index = FileIndex::new();
        for rel in [
            "notes/meeting-jan.md",
            "notes/meeting-notes-feb.md",
            "archive/old-meeting.md",
            "unrelated.md",
        ] {
            index.upsert(Path::new(rel), "x\n", mtime());
        }
        let hits = index.fuzzy_match("meet", 10);
        assert_eq!(hits.len(), 3);
        // Contiguous early filename match first, deep path last.
        assert_eq!(hits[0], PathBuf::from("notes/meeting-jan.md"));
        assert_eq!(hits[2], PathBuf::from("archive/old-meeting.md"));
    }

    #[test]
    fn fuzzy_no_match_and_remove() {
        let mut index = FileIndex::new();
        index.upsert(Path::new("a.md"), "x\n", mtime());
        assert!(index.fuzzy_match("zzz", 5).is_empty());
        index.remove(Path::new("a.md"));
        assert!(index.is_empty());
    }
}
