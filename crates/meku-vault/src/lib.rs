//! `meku-vault`: filesystem truth for a mekuto.
//!
//! A mekuto is a plain folder of Markdown files. All Meku state lives in the
//! `.meku/` sidecar so the folder stays portable. This crate has no GPUI
//! dependency and performs no rendering.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Hidden sidecar directory inside every mekuto.
pub const MEKU_DIR: &str = ".meku";
const SESSION_FILE: &str = "session.json";
const SESSION_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mekuto {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultFile {
    /// Path relative to the mekuto root.
    pub rel: PathBuf,
    pub abs: PathBuf,
    pub mtime: SystemTime,
}

#[derive(Debug, Clone)]
pub enum VaultEvent {
    Created(VaultFile),
    Modified(VaultFile),
    Removed(PathBuf),
}

/// Open a folder as a mekuto. Returns an error if it is not a directory.
pub fn open_root(path: &Path) -> anyhow::Result<Mekuto> {
    let root = path.canonicalize()?;
    if !root.is_dir() {
        anyhow::bail!("not a directory: {}", root.display());
    }
    Ok(Mekuto { root })
}

/// True for editable note files (`.md`/`.markdown`, any case).
pub fn is_note(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
    )
}

fn is_sidecar(rel: &Path) -> bool {
    rel.starts_with(MEKU_DIR)
}

/// Full recursive scan of note files, sorted by relative path.
/// Skips the `.meku/` sidecar. Unreadable entries are ignored.
pub fn scan_md(root: &Path) -> Vec<VaultFile> {
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(root).follow_links(false);
    for entry in walker.into_iter().filter_map(Result::ok) {
        let abs = entry.path().to_path_buf();
        let Ok(rel) = abs.strip_prefix(root).map(Path::to_path_buf) else {
            continue;
        };
        if rel.as_os_str().is_empty() || is_sidecar(&rel) {
            continue;
        }
        if !entry.file_type().is_file() || !is_note(&abs) {
            continue;
        }
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(UNIX_EPOCH);
        out.push(VaultFile { rel, abs, mtime });
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

pub fn session_path(root: &Path) -> PathBuf {
    root.join(MEKU_DIR).join(SESSION_FILE)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpenTab {
    /// Path relative to the mekuto root.
    pub path: PathBuf,
    /// Byte offset of the cursor.
    #[serde(default)]
    pub cursor: usize,
    #[serde(default)]
    pub scroll_px: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub active_tab: Option<PathBuf>,
    #[serde(default)]
    pub open_tabs: Vec<OpenTab>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            active_tab: None,
            open_tabs: Vec::new(),
        }
    }
}

/// Load the session sidecar. Missing or corrupt files yield an empty session;
/// corrupt files are backed up next to the original and never crash us.
pub fn load_session(root: &Path) -> Session {
    let path = session_path(root);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return Session::default(),
    };
    match serde_json::from_str::<Session>(&text) {
        Ok(mut session) => {
            session.version = SESSION_VERSION;
            session
        }
        Err(_) => {
            let backup = path.with_extension(format!(
                "corrupt-{}.json",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            let _ = fs::rename(&path, backup);
            Session::default()
        }
    }
}

/// Persist the session atomically (write temp + rename) so `kill -9` cannot
/// leave a half-written file behind.
pub fn save_session_atomic(root: &Path, session: &Session) -> anyhow::Result<()> {
    let dir = root.join(MEKU_DIR);
    fs::create_dir_all(&dir)?;
    let path = dir.join(SESSION_FILE);
    let tmp = dir.join(format!("{SESSION_FILE}.tmp"));
    let text = serde_json::to_string_pretty(session)?;
    fs::write(&tmp, text)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn vault_file_for(path: PathBuf, root: &Path) -> Option<VaultFile> {
    let rel = path.strip_prefix(root).ok()?.to_path_buf();
    if is_sidecar(&rel) || !is_note(&path) {
        return None;
    }
    let meta = fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(VaultFile {
        rel,
        abs: path,
        mtime: meta.modified().unwrap_or(UNIX_EPOCH),
    })
}

/// Watch a mekuto root and forward create/modify/remove events for note files.
/// Paths under `.meku/` are filtered out (session saves must not rescans).
/// The returned watcher must be kept alive for events to keep flowing.
pub fn start_watcher(
    root: &Path,
    tx: mpsc::Sender<VaultEvent>,
) -> notify::Result<notify::RecommendedWatcher> {
    use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

    let root = root.canonicalize().map_err(|e| {
        notify::Error::generic(&format!("cannot canonicalize {}: {e}", root.display()))
    })?;
    let watch_root = root.clone();
    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            let Ok(event) = res else { return };
            for path in event.paths {
                if event.kind.is_create() {
                    if let Some(file) = vault_file_for(path, &watch_root) {
                        let _ = tx.send(VaultEvent::Created(file));
                    }
                } else if event.kind.is_modify() {
                    if let Some(file) = vault_file_for(path, &watch_root) {
                        let _ = tx.send(VaultEvent::Modified(file));
                    }
                } else if event.kind.is_remove() {
                    if let Ok(rel) = path.strip_prefix(&watch_root).map(Path::to_path_buf) {
                        if !is_sidecar(&rel) {
                            let _ = tx.send(VaultEvent::Removed(rel));
                        }
                    }
                }
            }
        },
        notify::Config::default(),
    )?;
    watcher.watch(&root, RecursiveMode::Recursive)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fixture_root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("notes/projects")).unwrap();
        fs::create_dir_all(root.join(".meku")).unwrap();
        fs::write(root.join("notes/hello.md"), "# Hello\n").unwrap();
        fs::write(root.join("notes/projects/plan.md"), "# Plan\n").unwrap();
        fs::write(root.join("notes/image.png"), "fake").unwrap();
        fs::write(root.join(".meku/session.json"), "{}").unwrap();
        (dir, root)
    }

    #[test]
    fn scan_lists_notes_sorted_and_skips_sidecar() {
        let (_dir, root) = fixture_root();
        let files = scan_md(&root);
        let rels: Vec<_> = files.iter().map(|f| f.rel.clone()).collect();
        assert_eq!(
            rels,
            vec![
                PathBuf::from("notes/hello.md"),
                PathBuf::from("notes/projects/plan.md"),
            ]
        );
        assert!(files.iter().all(|f| f.abs.starts_with(&root)));
    }

    #[test]
    fn open_root_rejects_files() {
        let (_dir, root) = fixture_root();
        assert!(open_root(&root).is_ok());
        assert!(open_root(&root.join("notes/hello.md")).is_err());
        assert!(open_root(&root.join("missing")).is_err());
    }

    #[test]
    fn session_round_trip() {
        let (_dir, root) = fixture_root();
        // Existing "{}" sidecar is corrupt (missing version) -> empty, backed up.
        let loaded = load_session(&root);
        assert_eq!(loaded, Session::default());

        let session = Session {
            version: SESSION_VERSION,
            active_tab: Some(PathBuf::from("notes/hello.md")),
            open_tabs: vec![OpenTab {
                path: PathBuf::from("notes/hello.md"),
                cursor: 12,
                scroll_px: 320.0,
            }],
        };
        save_session_atomic(&root, &session).unwrap();
        assert_eq!(load_session(&root), session);
        // No temp file left behind.
        assert!(!root.join(MEKU_DIR).join("session.json.tmp").exists());
    }

    #[test]
    fn watcher_reports_created_note() {
        let (_dir, root) = fixture_root();
        let (tx, rx) = mpsc::channel();
        let _watcher = start_watcher(&root, tx).unwrap();
        fs::write(root.join("notes/new.md"), "# New\n").unwrap();
        let event = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        match event {
            VaultEvent::Created(file) => {
                assert_eq!(file.rel, PathBuf::from("notes/new.md"));
            }
            other => panic!("expected Created, got {other:?}"),
        }
    }

    #[test]
    fn watcher_ignores_sidecar_writes() {
        let (_dir, root) = fixture_root();
        let (tx, rx) = mpsc::channel();
        let _watcher = start_watcher(&root, tx).unwrap();
        save_session_atomic(
            &root,
            &Session {
                active_tab: None,
                ..Session::default()
            },
        )
        .unwrap();
        // Only sidecar files changed; nothing should arrive.
        assert!(rx.recv_timeout(Duration::from_millis(800)).is_err());
    }
}
