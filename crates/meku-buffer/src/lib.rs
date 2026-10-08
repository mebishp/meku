//! `meku-buffer`: open-file state, GPUI-free.
//!
//! Single owner of the [`ropey::Rope`]; edits bump a monotonic version used
//! to drop stale background parses. Never touches the window system.

use std::fs::File;
use std::io::BufReader;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ropey::{LineType, Rope};

/// Monotonic edit counter; background tasks check it to discard stale work.
pub type BufferVersion = u64;

#[derive(Debug)]
pub struct OpenBuffer {
    /// Path relative to the mekuto root.
    pub rel: PathBuf,
    text: Rope,
    pub version: BufferVersion,
    pub dirty: bool,
    pub disk_mtime: Option<SystemTime>,
}

impl OpenBuffer {
    /// Stream a file into a rope without an intermediate whole-file String.
    pub fn load(rel: PathBuf, abs: &Path) -> anyhow::Result<Self> {
        let file = File::open(abs)?;
        let disk_mtime = file.metadata()?.modified().ok();
        let text = Rope::from_reader(BufReader::new(file))?;
        Ok(Self {
            rel,
            text,
            version: 0,
            dirty: false,
            disk_mtime,
        })
    }

    pub fn from_text(rel: PathBuf, text: String) -> Self {
        Self {
            rel,
            text: Rope::from_str(&text),
            version: 0,
            dirty: false,
            disk_mtime: None,
        }
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }

    pub fn len_bytes(&self) -> usize {
        self.text.len()
    }

    pub fn line_count(&self) -> usize {
        self.text.len_lines(LineType::LF)
    }

    /// Apply an edit over a **byte** range (parser-style offsets) and return
    /// the new version. Coalescing of undo steps happens above this layer.
    pub fn apply_edit(&mut self, range: Range<usize>, insert: &str) -> BufferVersion {
        debug_assert!(
            range.start <= range.end
                && range.end <= self.text.len()
                && self.text.is_char_boundary(range.start)
                && self.text.is_char_boundary(range.end),
            "edit range must sit on char boundaries"
        );
        self.text.remove(range.clone());
        self.text.insert(range.start, insert);
        self.version += 1;
        self.dirty = true;
        self.version
    }

    pub fn mark_saved(&mut self, mtime: SystemTime) {
        self.dirty = false;
        self.disk_mtime = Some(mtime);
    }

    /// Clone out the text for a background parse. The version lets the
    /// parser result be discarded if another edit landed meanwhile.
    pub fn snapshot(&self) -> (BufferVersion, String) {
        (self.version, self.text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn buf(text: &str) -> OpenBuffer {
        OpenBuffer::from_text(PathBuf::from("a.md"), text.to_string())
    }

    #[test]
    fn edit_bumps_version_and_dirties() {
        let mut b = buf("hello\n");
        assert_eq!(b.version, 0);
        assert!(!b.dirty);
        let v = b.apply_edit(0..0, "# ");
        assert_eq!(v, 1);
        assert_eq!(b.version, 1);
        assert!(b.dirty);
        assert_eq!(b.snapshot().1, "# hello\n");
    }

    #[test]
    fn edit_replaces_range() {
        let mut b = buf("hello world\n");
        b.apply_edit(6..11, "meku");
        assert_eq!(b.snapshot().1, "hello meku\n");
    }

    #[test]
    fn multibyte_offsets_are_byte_based() {
        let mut b = buf("héllo\n");
        // 'é' is 2 bytes: byte range 1..3 covers it.
        b.apply_edit(1..3, "e");
        assert_eq!(b.snapshot().1, "hello\n");
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let mut b = buf("x\n");
        b.apply_edit(0..0, "y");
        assert!(b.dirty);
        b.mark_saved(UNIX_EPOCH);
        assert!(!b.dirty);
        assert_eq!(b.disk_mtime, Some(UNIX_EPOCH));
    }

    #[test]
    fn snapshot_is_isolated_from_later_edits() {
        let mut b = buf("one\n");
        let (v, text) = b.snapshot();
        b.apply_edit(0..0, "two\n");
        assert_eq!(v, 0);
        assert_eq!(text, "one\n");
        assert_eq!(b.version, 1);
    }

    #[test]
    fn load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().join("note.md");
        std::fs::write(&abs, "# Title\n\nbody").unwrap();
        let b = OpenBuffer::load(PathBuf::from("note.md"), &abs).unwrap();
        assert!(!b.dirty);
        assert!(b.disk_mtime.is_some());
        assert_eq!(b.snapshot().1, "# Title\n\nbody");
        assert_eq!(b.line_count(), 3);
    }
}
