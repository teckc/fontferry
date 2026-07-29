use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use async_trait::async_trait;
use fontferry_core::{
    FontDefinition, FontFerryError, FontInstaller, InstallOutcome, InstalledFont, PreparedFont,
    Result, RollbackSnapshot,
};

use crate::AppPaths;

#[derive(Clone, Debug)]
pub struct PlatformFontInstaller {
    paths: AppPaths,
}

impl PlatformFontInstaller {
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    fn install_directory(&self) -> Result<PathBuf> {
        platform::install_directory()
    }

    fn backup(&self, previous: &InstalledFont) -> Result<RollbackSnapshot> {
        let directory = self
            .paths
            .backups
            .join(&previous.font_id)
            .join(safe_component(&previous.version));
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(platform_error)?;
        }
        fs::create_dir_all(&directory).map_err(platform_error)?;
        for source in &previous.owned_files {
            if source.is_file() {
                let name = source.file_name().ok_or_else(|| {
                    FontFerryError::Platform(format!(
                        "owned font has no filename: {}",
                        source.display()
                    ))
                })?;
                fs::copy(source, directory.join(name)).map_err(platform_error)?;
            }
        }
        Ok(RollbackSnapshot {
            version: previous.version.clone(),
            variant_ids: previous.variant_ids.clone(),
            backup_directory: directory,
        })
    }

    fn copy_prepared(
        &self,
        font: &FontDefinition,
        prepared: &[PreparedFont],
    ) -> Result<Vec<PathBuf>> {
        let directory = self.install_directory()?;
        fs::create_dir_all(&directory).map_err(platform_error)?;
        let mut destinations = Vec::with_capacity(prepared.len());
        let mut names = BTreeSet::new();
        for item in prepared {
            let extension = item
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("ttf")
                .to_ascii_lowercase();
            let stem = format!("{}-{}", font.id, &item.sha256[..12.min(item.sha256.len())]);
            let mut name = format!("{stem}.{extension}");
            let mut suffix = 1_u32;
            while !names.insert(name.clone()) {
                name = format!("{stem}-{suffix}.{extension}");
                suffix += 1;
            }
            let destination = directory.join(name);
            fs::copy(&item.path, &destination).map_err(platform_error)?;
            destinations.push(destination);
        }
        Ok(destinations)
    }
}

#[async_trait]
impl FontInstaller for PlatformFontInstaller {
    async fn install(
        &self,
        font: &FontDefinition,
        _version: &str,
        prepared: &[PreparedFont],
        previous: Option<&InstalledFont>,
    ) -> Result<InstallOutcome> {
        if prepared.is_empty() {
            return Err(FontFerryError::FontRejected(
                "the artifact contains no supported fonts".into(),
            ));
        }
        let snapshot = previous.map(|installed| self.backup(installed)).transpose()?;
        let destinations = self.copy_prepared(font, prepared)?;
        if let Err(error) = platform::register(&destinations) {
            remove_files(&destinations);
            return Err(error);
        }
        if let Some(installed) = previous {
            platform::unregister(&installed.owned_files)?;
            remove_files(&installed.owned_files);
        }
        platform::refresh()?;
        Ok(InstallOutcome {
            owned_files: destinations,
            previous_snapshot: snapshot,
            restart_recommended: cfg!(target_os = "macos"),
            warnings: Vec::new(),
        })
    }

    async fn uninstall(&self, installed: &InstalledFont) -> Result<()> {
        platform::unregister(&installed.owned_files)?;
        remove_files(&installed.owned_files);
        if let Some(snapshot) = &installed.previous {
            if snapshot.backup_directory.exists() {
                fs::remove_dir_all(&snapshot.backup_directory).map_err(platform_error)?;
            }
        }
        platform::refresh()
    }

    async fn restore(
        &self,
        _font: &FontDefinition,
        snapshot: &RollbackSnapshot,
        current: &InstalledFont,
    ) -> Result<InstallOutcome> {
        platform::unregister(&current.owned_files)?;
        remove_files(&current.owned_files);
        let directory = self.install_directory()?;
        fs::create_dir_all(&directory).map_err(platform_error)?;
        let mut restored = Vec::new();
        for entry in fs::read_dir(&snapshot.backup_directory).map_err(platform_error)? {
            let entry = entry.map_err(platform_error)?;
            if entry.file_type().map_err(platform_error)?.is_file() {
                let destination = directory.join(entry.file_name());
                fs::copy(entry.path(), &destination).map_err(platform_error)?;
                restored.push(destination);
            }
        }
        if restored.is_empty() {
            return Err(FontFerryError::NoRollbackSnapshot);
        }
        platform::register(&restored)?;
        platform::refresh()?;
        fs::remove_dir_all(&snapshot.backup_directory).map_err(platform_error)?;
        Ok(InstallOutcome {
            owned_files: restored,
            previous_snapshot: None,
            restart_recommended: cfg!(target_os = "macos"),
            warnings: Vec::new(),
        })
    }
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn remove_files(paths: &[PathBuf]) {
    for path in paths {
        let _ignored = fs::remove_file(path);
    }
}

fn platform_error(error: std::io::Error) -> FontFerryError {
    FontFerryError::Platform(error.to_string())
}

fn run(command: &mut Command) -> Result<()> {
    let output = command.output().map_err(platform_error)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(FontFerryError::Platform(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        env,
        ffi::OsStr,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
    };

    use fontferry_core::{FontFerryError, Result};
    use windows_sys::Win32::{
        Graphics::Gdi::{AddFontResourceExW, FR_PRIVATE, RemoveFontResourceExW},
        UI::WindowsAndMessaging::{
            HWND_BROADCAST, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_FONTCHANGE,
        },
    };
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    const FONT_REGISTRY_KEY: &str =
        r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";

    pub fn install_directory() -> Result<PathBuf> {
        let base = env::var_os("LOCALAPPDATA").ok_or_else(|| {
            FontFerryError::Platform("LOCALAPPDATA is not available".into())
        })?;
        Ok(PathBuf::from(base).join("Microsoft").join("Windows").join("Fonts"))
    }

    pub fn register(paths: &[PathBuf]) -> Result<()> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(FONT_REGISTRY_KEY)
            .map_err(|error| FontFerryError::Platform(error.to_string()))?
            .0;
        for path in paths {
            let name = registry_name(path)?;
            key.set_value(&name, &path.as_os_str())
                .map_err(|error| FontFerryError::Platform(error.to_string()))?;
            let wide = wide_path(path);
            // SAFETY: `wide` is NUL-terminated and remains alive for the duration of the call.
            let added = unsafe { AddFontResourceExW(wide.as_ptr(), FR_PRIVATE, 0) };
            if added == 0 {
                let _ignored = key.delete_value(&name);
                return Err(FontFerryError::Platform(format!(
                    "Windows rejected font {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    pub fn unregister(paths: &[PathBuf]) -> Result<()> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(FONT_REGISTRY_KEY)
            .map_err(|error| FontFerryError::Platform(error.to_string()))?
            .0;
        for path in paths {
            let name = registry_name(path)?;
            let wide = wide_path(path);
            // SAFETY: `wide` is NUL-terminated and remains alive for the duration of the call.
            unsafe {
                RemoveFontResourceExW(wide.as_ptr(), FR_PRIVATE, 0);
            }
            let _ignored = key.delete_value(name);
        }
        Ok(())
    }

    pub fn refresh() -> Result<()> {
        let mut result = 0_usize;
        // SAFETY: Broadcasting WM_FONTCHANGE requires no pointer payload; the result pointer is valid.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_FONTCHANGE,
                0,
                0,
                SMTO_ABORTIFHUNG,
                1_000,
                &mut result,
            );
        }
        Ok(())
    }

    fn registry_name(path: &Path) -> Result<String> {
        let stem = path
            .file_stem()
            .and_then(OsStr::to_str)
            .ok_or_else(|| FontFerryError::Platform("invalid font filename".into()))?;
        Ok(format!("{stem} (TrueType)"))
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::{env, path::PathBuf};

    use fontferry_core::{FontFerryError, Result};

    pub fn install_directory() -> Result<PathBuf> {
        let home = env::var_os("HOME")
            .ok_or_else(|| FontFerryError::Platform("HOME is not available".into()))?;
        Ok(PathBuf::from(home).join("Library").join("Fonts"))
    }

    pub fn register(_paths: &[PathBuf]) -> Result<()> {
        Ok(())
    }

    pub fn unregister(_paths: &[PathBuf]) -> Result<()> {
        Ok(())
    }

    pub fn refresh() -> Result<()> {
        // CoreText observes changes in ~/Library/Fonts. A restart recommendation is surfaced.
        Ok(())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::{env, path::PathBuf, process::Command};

    use fontferry_core::{FontFerryError, Result};

    use super::run;

    pub fn install_directory() -> Result<PathBuf> {
        if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
            return Ok(PathBuf::from(data_home).join("fonts").join("fontferry"));
        }
        let home = env::var_os("HOME")
            .ok_or_else(|| FontFerryError::Platform("HOME is not available".into()))?;
        Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("fonts")
            .join("fontferry"))
    }

    pub fn register(_paths: &[PathBuf]) -> Result<()> {
        Ok(())
    }

    pub fn unregister(_paths: &[PathBuf]) -> Result<()> {
        Ok(())
    }

    pub fn refresh() -> Result<()> {
        run(Command::new("fc-cache").arg("-f"))
    }
}
