//! Images and files on the clipboard.
//!
//! Pasting: a copied picture arrives as image data; files copied in a file
//! manager arrive as a list of paths (macOS and Windows) or as plain text
//! holding `file://` links or paths (most Linux file managers).
//!
//! Copying out: macOS, Windows and X11 take image data from gpui directly.
//! gpui on Wayland only offers text, so there a helper program does the job
//! (`wl-copy` on compositors that allow it, `xclip` through XWayland, which
//! is how GNOME gets it).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gpui_kit::{App, ClipboardEntry, ClipboardItem, Image, ImageFormat};

/// File extensions Meku treats as pictures.
pub(crate) const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"];

pub(crate) fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Existing files on the clipboard that are all pictures, if that is what
/// it holds. Anything else (ordinary text, a mix of file types) is `None`, so
/// a normal paste is never hijacked.
pub(crate) fn image_files(item: &ClipboardItem) -> Option<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in item.entries() {
        if let ClipboardEntry::ExternalPaths(paths) = entry {
            files.extend(paths.paths().iter().cloned());
        }
    }
    if files.is_empty() {
        files = paths_in_text(&item.text()?)?;
    }
    (!files.is_empty() && files.iter().all(|f| f.is_file() && is_image_path(f))).then_some(files)
}

/// Every non-empty line of `text` read as a `file://` link or an absolute
/// path. `None` unless all of them are.
pub(crate) fn paths_in_text(text: &str) -> Option<Vec<PathBuf>> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        out.push(path_from_line(line)?);
    }
    (!out.is_empty()).then_some(out)
}

fn path_from_line(line: &str) -> Option<PathBuf> {
    if let Some(rest) = line.strip_prefix("file://") {
        // `file:///home/a/b.png` (empty host) or `file://localhost/...`.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let decoded = meku_markdown::live::percent_decode(rest);
        // `file:///C:/x.png` on Windows keeps a leading slash before the drive.
        let bytes = decoded.as_bytes();
        let path = if bytes.len() > 2 && bytes[0] == b'/' && bytes[2] == b':' {
            &decoded[1..]
        } else {
            decoded.as_str()
        };
        return Some(PathBuf::from(path));
    }
    let path = Path::new(line);
    path.is_absolute().then(|| path.to_path_buf())
}

pub(crate) fn format_of(path: &Path) -> Option<(ImageFormat, &'static str)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => (ImageFormat::Png, "image/png"),
        "jpg" | "jpeg" => (ImageFormat::Jpeg, "image/jpeg"),
        "gif" => (ImageFormat::Gif, "image/gif"),
        "webp" => (ImageFormat::Webp, "image/webp"),
        "svg" => (ImageFormat::Svg, "image/svg+xml"),
        "bmp" => (ImageFormat::Bmp, "image/bmp"),
        _ => return None,
    })
}

/// Put the picture at `path` on the clipboard. The error is a sentence for
/// the status bar.
pub(crate) fn copy_image(path: &Path, cx: &mut App) -> Result<(), String> {
    let (format, mime) = format_of(path).ok_or("This kind of image cannot be copied")?;
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read the image: {e}"))?;
    if wayland_session() {
        return copy_with_helper(&bytes, mime, &HELPERS);
    }
    let id = bytes.len() as u64 ^ (bytes.first().copied().unwrap_or(0) as u64) << 56;
    cx.write_to_clipboard(ClipboardItem::new_image(&Image { format, bytes, id }));
    Ok(())
}

fn wayland_session() -> bool {
    // Tests run against gpui's in-memory clipboard, not a helper program.
    cfg!(target_os = "linux")
        && !cfg!(test)
        && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

/// Programs that can put an image on a Wayland clipboard, with their
/// arguments (`{mime}` is replaced by the image type).
const HELPERS: [(&str, &[&str]); 2] = [
    ("wl-copy", &["--type", "{mime}"]),
    ("xclip", &["-selection", "clipboard", "-t", "{mime}", "-i"]),
];

/// Hand the bytes to the first helper that works here.
fn copy_with_helper(bytes: &[u8], mime: &str, helpers: &[(&str, &[&str])]) -> Result<(), String> {
    let mut found = false;
    for (program, args) in helpers {
        let args: Vec<String> = args.iter().map(|a| a.replace("{mime}", mime)).collect();
        let Ok(mut child) = Command::new(program)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        found = true;
        if let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(bytes).is_err()
        {
            continue;
        }
        // Both tools stay running to serve the clipboard (or fork away), so
        // wait briefly for an early failure rather than for them to exit.
        std::thread::sleep(std::time::Duration::from_millis(250));
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => continue,
            _ => return Ok(()),
        }
    }
    Err(if found {
        "Could not copy the image to the clipboard".into()
    } else {
        "To copy images on Linux, install xclip (or wl-clipboard)".into()
    })
}

#[cfg(test)]
mod tests {
    use super::{path_from_line, paths_in_text};
    use std::path::PathBuf;

    #[test]
    fn file_links_and_absolute_paths_become_paths() {
        assert_eq!(
            paths_in_text("file:///home/a/My%20pic.png\n/home/a/b.jpg\n"),
            Some(vec![
                PathBuf::from("/home/a/My pic.png"),
                PathBuf::from("/home/a/b.jpg")
            ])
        );
        assert_eq!(
            path_from_line("file://localhost/tmp/x.png"),
            Some(PathBuf::from("/tmp/x.png"))
        );
        assert_eq!(
            path_from_line("file:///C:/Users/a/x.png"),
            Some(PathBuf::from("C:/Users/a/x.png"))
        );
    }

    #[test]
    fn ordinary_text_is_not_a_file_list() {
        assert_eq!(paths_in_text("hello world"), None);
        assert_eq!(
            paths_in_text("/tmp/a.png and more"),
            Some(vec![PathBuf::from("/tmp/a.png and more")])
        );
        assert_eq!(
            paths_in_text("one line\n/tmp/a.png"),
            None,
            "every line must be a path"
        );
        assert_eq!(paths_in_text(""), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_helper_receives_the_image_bytes_and_the_next_one_is_tried_on_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("got.bin");
        let good = dir.path().join("good");
        std::fs::write(&good, format!("#!/bin/sh\ncat > {}\n", out.display())).unwrap();
        std::fs::set_permissions(&good, std::fs::Permissions::from_mode(0o755)).unwrap();
        let bad = dir.path().join("bad");
        std::fs::write(&bad, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (bad, good) = (bad.to_str().unwrap(), good.to_str().unwrap());

        assert_eq!(
            super::copy_with_helper(b"PNGDATA", "image/png", &[(bad, &[]), (good, &["{mime}"])]),
            Ok(())
        );
        assert_eq!(std::fs::read(&out).unwrap(), b"PNGDATA");
        let missing = dir.path().join("nope");
        let err = super::copy_with_helper(b"x", "image/png", &[(missing.to_str().unwrap(), &[])]);
        assert!(err.unwrap_err().contains("install xclip"));
    }
}
