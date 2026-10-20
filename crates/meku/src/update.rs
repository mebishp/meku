//! Update check: ask GitHub for the latest release, compare it with the
//! running version, and say how to update for the way Meku was installed.
//!
//! The request is one plain HTTPS GET of the public release list, made with
//! the system's `curl` (on macOS, Windows 10+ and nearly every Linux), so
//! Meku carries no network stack of its own. Nothing about the user or their
//! notes is sent, and the whole feature is off with the `check_updates`
//! setting.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

pub(crate) const REPO: &str = "mebishp/meku";
pub(crate) const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Release {
    /// `1.0.3` (no leading `v`).
    pub version: String,
    /// The release page on GitHub.
    pub page: String,
    pub assets: Vec<Asset>,
}

/// `v1.2.3` or `1.2.3-rc1` as numbers; anything else is `None`.
pub(crate) fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// The latest published release from GitHub's API answer. Drafts and
/// pre-releases are ignored.
pub(crate) fn parse_release(json: &str) -> Option<Release> {
    let v: Value = serde_json::from_str(json).ok()?;
    if v.get("draft")?.as_bool()? || v.get("prerelease")?.as_bool()? {
        return None;
    }
    let tag = v.get("tag_name")?.as_str()?;
    parse_version(tag)?;
    let assets = v
        .get("assets")?
        .as_array()?
        .iter()
        .filter_map(|a| {
            Some(Asset {
                name: a.get("name")?.as_str()?.to_string(),
                url: a.get("browser_download_url")?.as_str()?.to_string(),
            })
        })
        .collect();
    Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        page: v.get("html_url")?.as_str()?.to_string(),
        assets,
    })
}

/// Ask GitHub for the latest release. Runs on a background thread.
pub(crate) fn fetch_latest() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut curl = Command::new("curl");
    curl.args([
        "-fsSL",
        "--max-time",
        "10",
        "-H",
        "Accept: application/vnd.github+json",
        "-A",
        &format!("meku/{CURRENT}"),
        &url,
    ])
    .stdin(Stdio::null())
    .stderr(Stdio::null());
    hide_console(&mut curl);
    let out = curl.output().map_err(|_| {
        "Could not look for updates: `curl` is not available on this system".to_string()
    })?;
    if !out.status.success() {
        return Err("Could not reach GitHub to look for updates".to_string());
    }
    parse_release(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| "GitHub's answer was not a release".to_string())
}

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn hide_console(_: &mut Command) {}

/// How this copy of Meku was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Deb,
    Arch,
    AppImage,
    LinuxArchive,
    LinuxSystem,
    MacApp,
    Windows,
    Compiled,
}

pub(crate) fn detect_source() -> Source {
    if cfg!(windows) {
        return Source::Windows;
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_default();
    let text = exe.to_string_lossy();
    if cfg!(target_os = "macos") {
        return if text.contains(".app/Contents/MacOS") {
            Source::MacApp
        } else {
            Source::Compiled
        };
    }
    if std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty()) {
        return Source::AppImage;
    }
    if text.contains("/target/") || text.contains("/.cargo/bin/") {
        return Source::Compiled;
    }
    if text.starts_with("/usr/") || text.starts_with("/opt/") {
        if owned_by("dpkg", &["-S"], &exe) {
            return Source::Deb;
        }
        if owned_by("pacman", &["-Qo"], &exe) {
            return Source::Arch;
        }
        return Source::LinuxSystem;
    }
    Source::LinuxArchive
}

/// Whether a package manager says it owns `file`.
fn owned_by(program: &str, args: &[&str], file: &Path) -> bool {
    Command::new(program)
        .args(args)
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// What to show the user: where Meku came from and what to do.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Howto {
    /// "Debian / Ubuntu package", "AppImage", ...
    pub installed_as: &'static str,
    /// Plain steps, in order.
    pub steps: Vec<String>,
    /// A command to copy, when there is one.
    pub command: Option<String>,
}

fn asset<'a>(release: &'a Release, suffix: &str) -> Option<&'a Asset> {
    release.assets.iter().find(|a| a.name.ends_with(suffix))
}

/// `appimage_path` is where the running AppImage lives (its `$APPIMAGE`).
pub(crate) fn how_to_update(
    source: Source,
    release: &Release,
    appimage_path: Option<&Path>,
) -> Howto {
    let page = format!(
        "Open the release page ({}) and download the file for your system.",
        release.page
    );
    match source {
        Source::Deb => match asset(release, "_amd64.deb") {
            Some(a) => Howto {
                installed_as: "Debian / Ubuntu package (.deb)",
                steps: vec![
                    "Download the new package and install it over the old one.".into(),
                    "Your notes and settings are not touched. Restart Meku afterwards.".into(),
                ],
                command: Some(format!(
                    "curl -LO {} && sudo apt install ./{}",
                    a.url, a.name
                )),
            },
            None => fallback("Debian / Ubuntu package (.deb)", page),
        },
        Source::Arch => match asset(release, "-x86_64.pkg.tar.zst") {
            Some(a) => Howto {
                installed_as: "Arch Linux package",
                steps: vec![
                    "Download the new package and install it over the old one.".into(),
                    "Your notes and settings are not touched. Restart Meku afterwards.".into(),
                ],
                command: Some(format!("curl -LO {} && sudo pacman -U {}", a.url, a.name)),
            },
            None => fallback("Arch Linux package", page),
        },
        Source::AppImage => match asset(release, ".AppImage") {
            Some(a) => {
                let target = appimage_path
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| a.name.clone());
                Howto {
                    installed_as: "AppImage",
                    steps: vec![
                        "Download the new AppImage over the old file, then make it executable."
                            .into(),
                        "Close Meku first, then start the new file.".into(),
                    ],
                    command: Some(format!(
                        "curl -L -o \"{target}\" {} && chmod +x \"{target}\"",
                        a.url
                    )),
                }
            }
            None => fallback("AppImage", page),
        },
        Source::LinuxArchive => match asset(release, "-linux.tar.gz") {
            Some(a) => Howto {
                installed_as: "tar.gz download",
                steps: vec![
                    "Download the new archive and unpack it over (or beside) the old folder."
                        .into(),
                    "Close Meku first, then run the new `meku`.".into(),
                ],
                command: Some(format!("curl -LO {} && tar -xzf {}", a.url, a.name)),
            },
            None => fallback("tar.gz download", page),
        },
        Source::LinuxSystem => Howto {
            installed_as: "system package",
            steps: vec![
                "Update Meku with the package manager or store you installed it from.".into(),
                page,
            ],
            command: None,
        },
        Source::MacApp => {
            if cfg!(target_arch = "aarch64") {
                match asset(release, "macos-arm64.dmg") {
                    Some(a) => Howto {
                        installed_as: "macOS app",
                        steps: vec![
                            format!("Download {} from the release page and open it.", a.name),
                            "Drag Meku onto Applications and choose Replace.".into(),
                            "If macOS warns about an unidentified developer, right-click Meku and choose Open.".into(),
                        ],
                        command: None,
                    },
                    None => fallback("macOS app", page),
                }
            } else {
                Howto {
                    installed_as: "macOS app",
                    steps: vec![
                        "There is no Intel Mac build yet. Apple silicon Macs are supported.".into(),
                        page,
                    ],
                    command: None,
                }
            }
        }
        Source::Windows => match asset(release, "windows-x64.zip") {
            Some(a) => Howto {
                installed_as: "Windows download",
                steps: vec![
                    format!("Download {} from the release page.", a.name),
                    "Close Meku, then unzip it over the old folder (replace Meku.exe).".into(),
                    "If SmartScreen warns, choose More info, then Run anyway.".into(),
                ],
                command: None,
            },
            None => fallback("Windows download", page),
        },
        Source::Compiled => Howto {
            installed_as: "built from source",
            steps: vec!["In your Meku clone, pull the new version and build it again.".into()],
            command: Some("git pull && cargo build --release".into()),
        },
    }
}

fn fallback(installed_as: &'static str, page: String) -> Howto {
    Howto {
        installed_as,
        steps: vec![page],
        command: None,
    }
}

/// The `$APPIMAGE` path, when running from one.
pub(crate) fn appimage_path() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "tag_name": "v1.0.3", "draft": false, "prerelease": false,
        "html_url": "https://github.com/mebishp/meku/releases/tag/v1.0.3",
        "assets": [
          {"name": "meku_1.0.3_amd64.deb", "browser_download_url": "https://x/meku_1.0.3_amd64.deb"},
          {"name": "meku-1.0.3-1-x86_64.pkg.tar.zst", "browser_download_url": "https://x/meku-1.0.3-1-x86_64.pkg.tar.zst"},
          {"name": "Meku-1.0.3-x86_64.AppImage", "browser_download_url": "https://x/Meku-1.0.3-x86_64.AppImage"},
          {"name": "meku-1.0.3-x86_64-linux.tar.gz", "browser_download_url": "https://x/meku-1.0.3-x86_64-linux.tar.gz"},
          {"name": "Meku-1.0.3-macos-arm64.dmg", "browser_download_url": "https://x/Meku-1.0.3-macos-arm64.dmg"},
          {"name": "Meku-1.0.3-windows-x64.zip", "browser_download_url": "https://x/Meku-1.0.3-windows-x64.zip"}
        ]}"#;

    fn release() -> Release {
        parse_release(SAMPLE).unwrap()
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert_eq!(parse_version("v1.0.2"), Some((1, 0, 2)));
        assert_eq!(parse_version("1.10"), Some((1, 10, 0)));
        assert_eq!(parse_version("2.0.0-rc1"), Some((2, 0, 0)));
        assert_eq!(parse_version("one"), None);
        assert!(is_newer("1.0.10", "1.0.9"), "10 is newer than 9");
        assert!(is_newer("v1.1.0", "1.0.9"));
        assert!(
            !is_newer("1.0.2", "1.0.2"),
            "the same version is not an update"
        );
        assert!(!is_newer("1.0.1", "1.0.2"));
        assert!(
            !is_newer("garbage", "1.0.2"),
            "an unreadable version never nags"
        );
    }

    #[test]
    fn a_release_is_read_from_githubs_answer_and_drafts_are_ignored() {
        let r = release();
        assert_eq!(r.version, "1.0.3");
        assert_eq!(r.assets.len(), 6);
        assert!(r.page.ends_with("/v1.0.3"));
        assert_eq!(
            parse_release(&SAMPLE.replace("\"draft\": false", "\"draft\": true")),
            None
        );
        assert_eq!(
            parse_release(&SAMPLE.replace("\"prerelease\": false", "\"prerelease\": true")),
            None
        );
        assert_eq!(parse_release("{\"message\": \"Not Found\"}"), None);
    }

    #[test]
    fn each_install_source_gets_its_own_instructions() {
        let r = release();
        let deb = how_to_update(Source::Deb, &r, None);
        assert_eq!(
            deb.command.as_deref(),
            Some(
                "curl -LO https://x/meku_1.0.3_amd64.deb && sudo apt install ./meku_1.0.3_amd64.deb"
            )
        );
        let arch = how_to_update(Source::Arch, &r, None);
        assert!(
            arch.command
                .unwrap()
                .contains("sudo pacman -U meku-1.0.3-1-x86_64.pkg.tar.zst")
        );
        let app = how_to_update(
            Source::AppImage,
            &r,
            Some(Path::new("/home/a/Apps/Meku.AppImage")),
        );
        let cmd = app.command.unwrap();
        assert!(cmd.contains("-o \"/home/a/Apps/Meku.AppImage\"") && cmd.contains("chmod +x"));
        assert!(
            how_to_update(Source::LinuxArchive, &r, None)
                .command
                .unwrap()
                .contains("tar -xzf")
        );
        assert_eq!(
            how_to_update(Source::Compiled, &r, None).command.as_deref(),
            Some("git pull && cargo build --release")
        );
        let win = how_to_update(Source::Windows, &r, None);
        assert!(win.command.is_none() && win.steps[0].contains("Meku-1.0.3-windows-x64.zip"));
        let mac = how_to_update(Source::MacApp, &r, None);
        assert!(mac.command.is_none() && !mac.steps.is_empty());
    }

    #[test]
    fn a_missing_file_falls_back_to_the_release_page() {
        let mut r = release();
        r.assets.clear();
        let h = how_to_update(Source::Deb, &r, None);
        assert!(h.command.is_none());
        assert!(h.steps[0].contains(&r.page));
    }

    /// Needs the network: `cargo test -p meku --bin meku -- --ignored fetch_latest_live --nocapture`.
    #[test]
    #[ignore]
    fn fetch_latest_live() {
        let r = fetch_latest().expect("GitHub answers");
        eprintln!(
            "latest: {} ({} files) {}",
            r.version,
            r.assets.len(),
            r.page
        );
        assert!(parse_version(&r.version).is_some());
        assert!(!r.assets.is_empty());
    }
}
