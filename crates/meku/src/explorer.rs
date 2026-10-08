//! Sidebar explorer model: vault state, tree building, filesystem ops.
//!
//! UI-agnostic except for [`TreeItem`] construction (a `gpui-kit` base type).
//! All methods are synchronous and cheap except [`Explorer::open`], which
//! performs the initial vault scan.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui_kit::component::tree::TreeItem;
use meku_index::FileIndex;
use meku_vault::{Mekuto, Session, VaultEvent, VaultFile};

pub struct Explorer {
    vault: Option<Mekuto>,
    files: Vec<VaultFile>,
    dirs: HashSet<PathBuf>,
    expanded: HashSet<PathBuf>,
    index: FileIndex,
}

impl Explorer {
    pub fn new() -> Self {
        Self {
            vault: None,
            files: Vec::new(),
            dirs: HashSet::new(),
            expanded: HashSet::new(),
            index: FileIndex::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.vault.is_some()
    }

    pub fn root(&self) -> Option<&Path> {
        self.vault.as_ref().map(|v| v.root.as_path())
    }

    pub fn root_name(&self) -> String {
        self.vault
            .as_ref()
            .and_then(|v| v.root.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("mekuto")
            .to_string()
    }

    /// Open a folder as the current mekuto: scan, index, expand the first
    /// level. Returns the stored session for tab restoration.
    pub fn open(&mut self, dir: &Path) -> anyhow::Result<Session> {
        let vault = meku_vault::open_root(dir)?;
        self.files = meku_vault::scan_md(&vault.root);
        self.derive_dirs();
        self.index.rebuild(&self.files);
        let session = meku_vault::load_session(&vault.root);
        // Expand first-level directories so a fresh mekuto shows its shape.
        self.expanded = self
            .dirs
            .iter()
            .filter(|d| d.components().count() == 1)
            .cloned()
            .collect();
        self.vault = Some(vault);
        Ok(session)
    }

    /// Full rescan (used after our own file ops; watcher events apply
    /// incrementally via [`Explorer::on_vault_event`]).
    pub fn refresh(&mut self) {
        if let Some(vault) = &self.vault {
            self.files = meku_vault::scan_md(&vault.root);
            self.derive_dirs();
            self.index.rebuild(&self.files);
            self.expanded.retain(|d| self.dirs.contains(d));
        }
    }

    /// Apply a single watcher event. Returns true if the tree changed.
    pub fn on_vault_event(&mut self, event: &VaultEvent) -> bool {
        let Some(root) = self.root().map(Path::to_path_buf) else {
            return false;
        };
        match event {
            VaultEvent::Created(file) | VaultEvent::Modified(file) => {
                if let Ok(text) = std::fs::read_to_string(&file.abs) {
                    self.index.upsert(&file.rel, &text, file.mtime);
                }
                match self.files.iter_mut().find(|f| f.rel == file.rel) {
                    Some(existing) => *existing = file.clone(),
                    None => self.files.push(file.clone()),
                }
                self.files.sort_by(|a, b| a.rel.cmp(&b.rel));
                self.derive_dirs_from(&root);
                true
            }
            VaultEvent::Removed(rel) => {
                let before = self.files.len();
                self.files
                    .retain(|f| f.rel != *rel && !f.rel.starts_with(rel));
                self.index.remove(rel);
                self.expanded.retain(|d| d != rel && !d.starts_with(rel));
                self.derive_dirs_from(&root);
                before != self.files.len()
            }
        }
    }

    fn derive_dirs(&mut self) {
        self.dirs.clear();
        for file in &self.files {
            let mut parent = file.rel.parent();
            while let Some(dir) = parent {
                if dir.as_os_str().is_empty() {
                    break;
                }
                self.dirs.insert(dir.to_path_buf());
                parent = dir.parent();
            }
        }
        // Also pick up empty on-disk directories so "New Folder" is visible.
        if let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) {
            self.derive_dirs_from(&root);
        }
    }

    fn derive_dirs_from(&mut self, root: &Path) {
        let walker = walkdir::WalkDir::new(root)
            .min_depth(1)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_dir());
        for entry in walker {
            let Ok(rel) = entry.path().strip_prefix(root).map(Path::to_path_buf) else {
                continue;
            };
            if rel.starts_with(meku_vault::MEKU_DIR) {
                continue;
            }
            // Every ancestor is a visible directory too.
            let mut cursor = Some(rel.as_path());
            while let Some(dir) = cursor {
                if dir.as_os_str().is_empty() {
                    break;
                }
                self.dirs.insert(dir.to_path_buf());
                cursor = dir.parent();
            }
        }
        self.dirs.retain(|d| root.join(d).is_dir());
    }

    pub fn is_dir(&self, rel: &Path) -> bool {
        self.dirs.contains(rel)
    }

    pub fn is_expanded(&self, rel: &Path) -> bool {
        self.expanded.contains(rel)
    }

    /// Expand every ancestor of `rel` so it becomes visible in the tree.
    pub fn reveal(&mut self, rel: &Path) {
        let mut cursor = rel.parent();
        while let Some(dir) = cursor {
            if dir.as_os_str().is_empty() {
                break;
            }
            self.expanded.insert(dir.to_path_buf());
            cursor = dir.parent();
        }
    }

    pub fn toggle(&mut self, rel: &Path) {
        if !self.expanded.remove(rel) {
            self.expanded.insert(rel.to_path_buf());
        }
    }

    pub fn abs(&self, rel: &Path) -> Option<PathBuf> {
        self.root().map(|root| root.join(rel))
    }

    /// Build the nested [`TreeItem`] list: directories first, then notes,
    /// case-insensitive, with expansion applied from [`Explorer::toggle`].
    pub fn tree_items(&self) -> Vec<TreeItem> {
        self.tree_level(None)
    }

    fn tree_level(&self, parent: Option<&Path>) -> Vec<TreeItem> {
        // `Path::parent` of a top-level entry is `Some("")`, not `None`.
        let at_level = |candidate: Option<&Path>| match (candidate, parent) {
            (None, None) => true,
            (Some(a), Some(b)) => a == b,
            (None, Some(b)) => b.as_os_str().is_empty(),
            (Some(a), None) => a.as_os_str().is_empty(),
        };
        let mut dirs: Vec<&PathBuf> = self.dirs.iter().filter(|d| at_level(d.parent())).collect();
        dirs.sort_by(|a, b| {
            a.to_string_lossy()
                .to_lowercase()
                .cmp(&b.to_string_lossy().to_lowercase())
        });
        let mut files: Vec<&VaultFile> = self
            .files
            .iter()
            .filter(|f| at_level(f.rel.parent()))
            .collect();
        files.sort_by(|a, b| {
            a.rel
                .to_string_lossy()
                .to_lowercase()
                .cmp(&b.rel.to_string_lossy().to_lowercase())
        });

        let mut items = Vec::with_capacity(dirs.len() + files.len());
        for dir in dirs {
            let id = dir.to_string_lossy().to_string();
            let label = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let mut item = TreeItem::new(id, label);
            if self.expanded.contains(dir) {
                item = item.expanded(true).children(self.tree_level(Some(dir)));
            }
            items.push(item);
        }
        for file in files {
            let id = file.rel.to_string_lossy().to_string();
            let label = file
                .rel
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            items.push(TreeItem::new(id, label));
        }
        items
    }

    // -- filesystem ops (caller refreshes afterwards) ---------------------

    pub fn create_note(&self, dir: &Path, name: &str) -> anyhow::Result<PathBuf> {
        let root = self
            .root()
            .ok_or_else(|| anyhow::anyhow!("no mekuto open"))?;
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("note name must not be empty");
        }
        if name.contains(['/', '\\']) {
            anyhow::bail!("note name must not contain path separators");
        }
        let rel = if name.contains('.') {
            dir.join(name)
        } else {
            dir.join(format!("{name}.md"))
        };
        if root.join(&rel).exists() {
            anyhow::bail!("already exists: {}", rel.display());
        }
        if let Some(parent) = root.join(&rel).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(root.join(&rel), "")?;
        Ok(rel)
    }

    pub fn create_dir(&self, parent: &Path, name: &str) -> anyhow::Result<PathBuf> {
        let root = self
            .root()
            .ok_or_else(|| anyhow::anyhow!("no mekuto open"))?;
        let name = name.trim();
        if name.is_empty() || name.contains(['/', '\\']) {
            anyhow::bail!("invalid folder name");
        }
        let rel = parent.join(name);
        if root.join(&rel).exists() {
            anyhow::bail!("already exists: {}", rel.display());
        }
        std::fs::create_dir_all(root.join(&rel))?;
        Ok(rel)
    }

    /// Rename a file or directory within its parent. If a note's new name
    /// has no extension, the old one is kept so it stays a visible note.
    pub fn rename(&self, rel: &Path, new_name: &str) -> anyhow::Result<PathBuf> {
        let root = self
            .root()
            .ok_or_else(|| anyhow::anyhow!("no mekuto open"))?;
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name.contains(['/', '\\']) {
            anyhow::bail!("invalid name");
        }
        let parent = rel.parent().unwrap_or_else(|| Path::new(""));
        let mut target = parent.join(new_name);
        // Keep the note's extension when the new name has none, so the
        // renamed file stays a visible note instead of vanishing.
        if !self.is_dir(rel)
            && Path::new(new_name).extension().is_none()
            && let Some(ext) = rel.extension()
        {
            target.set_extension(ext);
        }
        if root.join(&target).exists() {
            anyhow::bail!("already exists: {}", target.display());
        }
        std::fs::rename(root.join(rel), root.join(&target))?;
        Ok(target)
    }

    /// Delete a file or directory tree from disk.
    pub fn delete(&self, rel: &Path) -> anyhow::Result<()> {
        let root = self
            .root()
            .ok_or_else(|| anyhow::anyhow!("no mekuto open"))?;
        let abs = root.join(rel);
        if self.is_dir(rel) || abs.is_dir() {
            std::fs::remove_dir_all(&abs)?;
        } else {
            std::fs::remove_file(&abs)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_fixture() -> (tempfile::TempDir, Explorer) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("notes/projects")).unwrap();
        std::fs::write(root.join("notes/b.md"), "# B\n").unwrap();
        std::fs::write(root.join("notes/a.md"), "# A\n").unwrap();
        std::fs::write(root.join("notes/projects/z.md"), "# Z\n").unwrap();
        std::fs::write(root.join("top.md"), "# Top\n").unwrap();
        let mut explorer = Explorer::new();
        let _ = explorer.open(root).unwrap();
        (dir, explorer)
    }

    #[test]
    fn tree_lists_dirs_first_sorted() {
        let (_dir, explorer) = open_fixture();
        assert!(explorer.is_open());
        // top.md at root, notes/ dir expanded by default (first level).
        let items = explorer.tree_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "notes");
        assert_eq!(items[1].label, "top");
    }

    #[test]
    fn toggle_expands_and_collapses() {
        let (_dir, mut explorer) = open_fixture();
        assert!(explorer.expanded.contains(Path::new("notes")));
        explorer.toggle(Path::new("notes"));
        assert!(!explorer.expanded.contains(Path::new("notes")));
        assert!(explorer.tree_items()[0].children.is_empty());
        explorer.toggle(Path::new("notes"));
        assert_eq!(explorer.tree_items()[0].children.len(), 3); // a, b, projects/
    }

    #[test]
    fn ops_round_trip() {
        let (_dir, mut explorer) = open_fixture();
        let rel = explorer.create_note(Path::new("notes"), "new").unwrap();
        assert_eq!(rel, PathBuf::from("notes/new.md"));
        explorer.refresh();
        assert!(explorer.tree_items()[0].children.len() == 4);

        let renamed = explorer.rename(&rel, "renamed.md").unwrap();
        assert_eq!(renamed, PathBuf::from("notes/renamed.md"));
        explorer.refresh();

        // Extension auto-kept for notes.
        let renamed2 = explorer.rename(&renamed, "final").unwrap();
        assert_eq!(renamed2, PathBuf::from("notes/final.md"));
        explorer.refresh();

        explorer.delete(&renamed2).unwrap();
        explorer.refresh();
        assert!(explorer.tree_items()[0].children.len() == 3);

        let dir = explorer.create_dir(Path::new("notes"), "sub").unwrap();
        assert_eq!(dir, PathBuf::from("notes/sub"));
        explorer.refresh();
        assert!(explorer.is_dir(&dir));
        explorer.delete(&dir).unwrap();
        explorer.refresh();
        assert!(!explorer.is_dir(&dir));
    }
}
