//! The tutorial mekuto created on the very first run, so a newcomer lands in
//! a notebook that teaches itself instead of an empty window.

use std::path::{Path, PathBuf};

use crate::config;

/// The note opened first.
pub(crate) const FIRST_NOTE: &str = "1 Start here.md";

const NOTES: [(&str, &str); 6] = [
    (FIRST_NOTE, include_str!("../assets/demo/1 Start here.md")),
    (
        "2 Write like you think.md",
        include_str!("../assets/demo/2 Write like you think.md"),
    ),
    (
        "3 Make it look right.md",
        include_str!("../assets/demo/3 Make it look right.md"),
    ),
    (
        "4 Checklists and tables.md",
        include_str!("../assets/demo/4 Checklists and tables.md"),
    ),
    (
        "5 Find things fast.md",
        include_str!("../assets/demo/5 Find things fast.md"),
    ),
    (
        "6 Your notes are just files.md",
        include_str!("../assets/demo/6 Your notes are just files.md"),
    ),
];

/// The mekuto to start in when there is none to reopen: `~/Documents/Meku`.
/// The first time, it is created with the tutorial notes (`true` = just
/// created). Afterwards it is only reused while it still exists; a deleted
/// one is not brought back.
pub(crate) fn default_start() -> Option<(PathBuf, bool)> {
    let dir = config::default_mekuto()?;
    if config::state_value("demo_seeded").is_some() {
        return dir.is_dir().then_some((dir, false));
    }
    write_notes(&dir).ok()?;
    config::set_state_value("demo_seeded", true.into());
    Some((dir, true))
}

/// Write the notes, never replacing a file that is already there.
fn write_notes(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, text) in NOTES {
        let path = dir.join(name);
        if !path.exists() {
            std::fs::write(path, text)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_every_note_and_keeps_existing_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Meku Welcome");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(FIRST_NOTE), "mine").unwrap();
        write_notes(&root).unwrap();
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), NOTES.len());
        assert_eq!(
            std::fs::read_to_string(root.join(FIRST_NOTE)).unwrap(),
            "mine"
        );
    }
}
