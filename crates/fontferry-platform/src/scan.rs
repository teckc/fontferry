use std::{
    env,
    path::{Path, PathBuf},
};

use fontferry_core::ObservedFont;
use regex::Regex;
use time::OffsetDateTime;
use walkdir::WalkDir;

use crate::inspect_font_file;

const MAX_SCANNED_FILES: usize = 50_000;

#[must_use]
pub fn scan_font_awesome() -> Option<ObservedFont> {
    let version_pattern = Regex::new(r"(?i)(?:version\s*)?v?(\d+\.\d+(?:\.\d+)?)").ok()?;
    let mut best_version: Option<String> = None;
    let mut observed_files = Vec::new();
    let mut visited = 0_usize;
    for root in font_directories() {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if visited >= MAX_SCANNED_FILES {
                break;
            }
            visited += 1;
            if !entry.file_type().is_file() || !is_font(entry.path()) {
                continue;
            }
            let Ok(font) = inspect_font_file(entry.path()) else {
                continue;
            };
            let names = format!(
                "{} {} {}",
                font.family,
                font.style,
                font.postscript_name.as_deref().unwrap_or_default()
            );
            if !names.to_ascii_lowercase().contains("font awesome") {
                continue;
            }
            observed_files.push(entry.path().to_path_buf());
            if let Some(raw_version) = font.version
                && let Some(version) = version_pattern
                    .captures(&raw_version)
                    .and_then(|captures| captures.get(1))
                    .map(|value| value.as_str().to_owned())
                && best_version
                    .as_ref()
                    .is_none_or(|current| fontferry_core::is_update_available(current, &version))
            {
                best_version = Some(version);
            }
        }
    }
    (!observed_files.is_empty()).then_some(ObservedFont {
        font_id: "font-awesome-pro".into(),
        detected_version: best_version,
        manual_version: None,
        observed_files,
        scanned_at: OffsetDateTime::now_utc(),
    })
}

fn is_font(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

fn font_directories() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut directories = Vec::new();
        if let Some(windows) = env::var_os("WINDIR") {
            directories.push(PathBuf::from(windows).join("Fonts"));
        }
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            directories.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
        directories
    }
    #[cfg(target_os = "macos")]
    {
        let mut directories = vec![PathBuf::from("/Library/Fonts")];
        if let Some(home) = env::var_os("HOME") {
            directories.push(PathBuf::from(home).join("Library").join("Fonts"));
        }
        directories
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut directories = vec![
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/usr/local/share/fonts"),
        ];
        if let Some(data) = env::var_os("XDG_DATA_HOME") {
            directories.push(PathBuf::from(data).join("fonts"));
        } else if let Some(home) = env::var_os("HOME") {
            directories.push(
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("fonts"),
            );
        }
        directories
    }
}
