use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use crate::AppPaths;
use async_trait::async_trait;
use fontferry_core::{
    FontDefinition, FontFerryError, FontInstaller, InstallOutcome, InstalledFont, PreparedFont,
    Result, RollbackSnapshot, StateRepository,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct PlatformFontInstaller {
    paths: AppPaths,
    system: std::sync::Arc<dyn FontSystem>,
}

trait FontSystem: std::fmt::Debug + Send + Sync {
    fn directory(&self) -> Result<PathBuf>;
    fn copy(&self, source: &Path, target: &Path) -> Result<()> {
        copy_new(source, target)
    }
    fn register(&self, paths: &[PathBuf]) -> Result<()>;
    fn unregister(&self, paths: &[PathBuf]) -> Result<()>;
    fn refresh(&self) -> Result<()>;
    fn remove(&self, path: &Path) -> std::io::Result<()> {
        fs::remove_file(path)
    }
}
#[derive(Debug)]
struct NativeFontSystem;
impl FontSystem for NativeFontSystem {
    fn directory(&self) -> Result<PathBuf> {
        platform::install_directory()
    }
    fn register(&self, paths: &[PathBuf]) -> Result<()> {
        platform::register(paths)
    }
    fn unregister(&self, paths: &[PathBuf]) -> Result<()> {
        platform::unregister(paths)
    }
    fn refresh(&self) -> Result<()> {
        platform::refresh()
    }
}

/// Written before touching installed files; retained until all cleanup succeeds.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Operation {
    font_id: String,
    version: Option<String>,
    old: Option<InstalledFont>,
    backup: Option<RollbackSnapshot>,
    old_hashes: BTreeMap<PathBuf, String>,
    target: BTreeMap<PathBuf, String>,
}

impl PlatformFontInstaller {
    pub fn new(paths: AppPaths) -> Self {
        Self {
            paths,
            system: std::sync::Arc::new(NativeFontSystem),
        }
    }
    fn journal(&self) -> PathBuf {
        self.paths.data.join("font-operation.json")
    }

    fn backup(
        &self,
        previous: &InstalledFont,
    ) -> Result<(RollbackSnapshot, BTreeMap<PathBuf, String>)> {
        let directory = self.paths.backups.join(Uuid::new_v4().to_string());
        fs::create_dir(&directory).map_err(platform_error)?;
        let mut hashes = BTreeMap::new();
        for source in &previous.owned_files {
            let hash = file_hash(source)?;
            let name = source
                .file_name()
                .ok_or_else(|| FontFerryError::Platform("owned file has no name".into()))?;
            let destination = directory.join(name);
            copy_new(source, &destination)?;
            if file_hash(&destination)? != hash {
                return Err(FontFerryError::Platform(
                    "backup verification failed".into(),
                ));
            }
            hashes.insert(source.clone(), hash);
        }
        let mut manifest = File::create(directory.join("manifest.json")).map_err(platform_error)?;
        serde_json::to_writer(&mut manifest, &hashes)
            .map_err(|e| FontFerryError::State(e.to_string()))?;
        manifest.sync_all().map_err(platform_error)?;
        Ok((
            RollbackSnapshot {
                version: previous.version.clone(),
                variant_ids: previous.variant_ids.clone(),
                backup_directory: directory,
            },
            hashes,
        ))
    }

    fn begin(
        &self,
        font: &FontDefinition,
        version: Option<&str>,
        prepared: &[PreparedFont],
        old: Option<&InstalledFont>,
    ) -> Result<InstallOutcome> {
        if self.journal().exists() {
            return Err(FontFerryError::State(
                "unfinished font operation; restart to recover before retrying".into(),
            ));
        }
        let directory = self.system.directory()?;
        fs::create_dir_all(&directory).map_err(platform_error)?;
        let old_files: BTreeSet<_> = old
            .into_iter()
            .flat_map(|item| item.owned_files.iter().cloned())
            .collect();
        let mut target = BTreeMap::new();
        let mut sources = BTreeMap::new();
        for item in prepared {
            let hash = file_hash(&item.path)?;
            if hash != item.sha256 {
                return Err(FontFerryError::FontRejected("prepared font changed".into()));
            }
            if target.values().any(|value| *value == hash) {
                continue;
            }
            let extension = item
                .path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("ttf")
                .to_ascii_lowercase();
            // Full digest avoids prefix collisions; identical payloads share one managed path.
            let mut path = directory.join(format!("{}-{hash}.{extension}", font.id));
            // Reuse legacy managed names when their full content matches.
            if let Some(existing) = old_files
                .iter()
                .find(|p| file_hash(p).is_ok_and(|h| h == hash))
            {
                path = existing.clone();
            }
            if fs::symlink_metadata(&path).is_ok() && !old_files.contains(&path) {
                return Err(FontFerryError::Platform(format!(
                    "refusing to overwrite unmanaged file: {}",
                    path.display()
                )));
            }
            target.insert(path.clone(), hash);
            sources.insert(path, item.path.clone());
        }
        let (backup, old_hashes) = match old {
            Some(old) => {
                let (snapshot, hashes) = self.backup(old)?;
                (Some(snapshot), hashes)
            }
            None => (None, BTreeMap::new()),
        };
        let operation = Operation {
            font_id: font.id.clone(),
            version: version.map(str::to_owned),
            old: old.cloned(),
            backup: backup.clone(),
            old_hashes,
            target,
        };
        let mut journal = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.journal())
            .map_err(platform_error)?;
        serde_json::to_writer(&mut journal, &operation)
            .map_err(|e| FontFerryError::State(e.to_string()))?;
        journal.flush().map_err(platform_error)?;
        journal.sync_all().map_err(platform_error)?;
        // No old file is removed before the state commit. Any failure leaves the journal recoverable.
        let added: Vec<_> = operation
            .target
            .keys()
            .filter(|p| !old_files.contains(*p))
            .cloned()
            .collect();
        for path in &added {
            self.system.copy(&sources[path], path)?;
        }
        self.system.register(&added)?;
        let removed: Vec<_> = old_files
            .iter()
            .filter(|p| !operation.target.contains_key(*p))
            .cloned()
            .collect();
        self.system.unregister(&removed)?;
        self.system.refresh()?;
        Ok(InstallOutcome {
            owned_files: operation.target.keys().cloned().collect(),
            previous_snapshot: backup,
            restart_recommended: cfg!(target_os = "macos"),
            warnings: Vec::new(),
        })
    }

    fn validate_operation(&self, operation: &Operation) -> Result<()> {
        let invalid = || {
            FontFerryError::State(
                "invalid recovery journal invariants; preserve journal and files for manual repair"
                    .into(),
            )
        };
        let directory = self.system.directory()?;
        if operation.font_id.is_empty()
            || operation.version.is_some() == operation.target.is_empty()
            || operation.old.is_some() != operation.backup.is_some()
            || operation
                .old
                .as_ref()
                .is_some_and(|old| old.font_id != operation.font_id)
        {
            return Err(invalid());
        }
        let owned: BTreeSet<_> = operation
            .old
            .iter()
            .flat_map(|old| old.owned_files.iter())
            .collect();
        if owned != operation.old_hashes.keys().collect() {
            return Err(invalid());
        }
        for (path, hash) in operation.old_hashes.iter().chain(&operation.target) {
            if path.parent() != Some(directory.as_path())
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                || hash.len() != 64
                || !hash.bytes().all(|b| b.is_ascii_hexdigit())
                || !path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with(&format!("{}-", operation.font_id))
                })
            {
                return Err(invalid());
            }
        }
        for (path, hash) in &operation.target {
            if operation
                .old_hashes
                .get(path)
                .is_some_and(|old| old != hash)
            {
                return Err(invalid());
            }
        }
        if let (Some(old), Some(backup)) = (&operation.old, &operation.backup)
            && (backup.version != old.version
                || backup.variant_ids != old.variant_ids
                || old
                    .previous
                    .as_ref()
                    .is_some_and(|previous| previous.backup_directory == backup.backup_directory))
        {
            return Err(invalid());
        }
        for snapshot in operation
            .backup
            .iter()
            .chain(operation.old.iter().filter_map(|old| old.previous.as_ref()))
        {
            let relative = snapshot
                .backup_directory
                .strip_prefix(&self.paths.backups)
                .map_err(|_| invalid())?;
            if relative.as_os_str().is_empty()
                || relative
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err(invalid());
            }
            let mut current = self.paths.backups.clone();
            for part in relative.components() {
                current.push(part);
                match fs::symlink_metadata(&current) {
                    Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
                        return Err(invalid());
                    }
                    Ok(_) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => return Err(platform_error(error)),
                }
            }
        }
        Ok(())
    }

    fn resolve(&self, operation: &Operation, committed: bool) -> Result<()> {
        self.validate_operation(operation)?;
        let old: BTreeSet<_> = operation.old_hashes.keys().cloned().collect();
        let target: BTreeSet<_> = operation.target.keys().cloned().collect();
        let (keep, remove) = if committed {
            (&target, &old)
        } else {
            (&old, &target)
        };
        let obsolete: Vec<_> = remove.difference(keep).cloned().collect();
        let obsolete_hashes = if committed {
            &operation.old_hashes
        } else {
            &operation.target
        };
        // Preflight the entire plan before copies, registration, or deletion. A partial
        // copy has no verified final hash: keep it and the journal for manual recovery.
        for path in &obsolete {
            verify_existing(path, &obsolete_hashes[path])?;
        }
        let mut copies = Vec::new();
        if committed {
            for (path, hash) in &operation.target {
                if !verify_existing(path, hash)? {
                    return Err(FontFerryError::State(
                        "committed font is missing; preserve journal and backups for repair".into(),
                    ));
                }
            }
        } else if let Some(snapshot) = &operation.backup {
            for (path, hash) in &operation.old_hashes {
                if verify_existing(path, hash)? {
                    continue;
                }
                let source =
                    snapshot
                        .backup_directory
                        .join(path.file_name().ok_or_else(|| {
                            FontFerryError::State("invalid recovery path".into())
                        })?);
                if !verify_existing(&source, hash)? {
                    return Err(FontFerryError::State(
                        "recovery backup is missing; preserve journal and current files".into(),
                    ));
                }
                copies.push((source, path));
            }
        }
        for (source, path) in copies {
            copy_new(&source, path)?;
        }
        if !committed && operation.backup.is_some() {
            self.system
                .register(&old.difference(&target).cloned().collect::<Vec<_>>())?;
        }
        self.system.unregister(&obsolete)?;
        for path in &obsolete {
            // Recheck immediately before deletion as well as during the whole-plan preflight.
            if !verify_existing(path, &obsolete_hashes[path])? {
                continue;
            }
            match self.system.remove(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(platform_error(e)),
            }
        }
        self.system.refresh()?;
        // Keep the newly created snapshot after an install. Previous history is obsolete only now.
        if committed {
            if let Some(previous) = operation.old.as_ref().and_then(|old| old.previous.as_ref()) {
                remove_directory(&previous.backup_directory)?;
            }
            if operation.version.is_none()
                && let Some(backup) = &operation.backup
            {
                remove_directory(&backup.backup_directory)?;
            }
        } else if let Some(backup) = &operation.backup {
            remove_directory(&backup.backup_directory)?;
        }
        fs::remove_file(self.journal()).map_err(platform_error)
    }
}

#[async_trait]
impl FontInstaller for PlatformFontInstaller {
    async fn recover(&self, state: &dyn StateRepository) -> Result<()> {
        let journal = self.journal();
        if !journal.exists() {
            return Ok(());
        }
        let bytes = fs::read(&journal).map_err(platform_error)?;
        let operation: Operation = serde_json::from_slice(&bytes).map_err(|e| {
            FontFerryError::State(format!(
                "invalid recovery journal; preserve files and restore journal: {e}"
            ))
        })?;
        let installed = state.get_installed(&operation.font_id).await?;
        let committed = match (&operation.version, &installed) {
            (None, None) => true,
            (Some(version), Some(installed)) => {
                installed.version == *version
                    && installed
                        .owned_files
                        .iter()
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        == operation.target.keys().cloned().collect()
                    && installed.previous == operation.backup
            }
            _ => false,
        };
        if !committed && installed != operation.old {
            return Err(FontFerryError::State(
                "database does not match either journal state; manual recovery required".into(),
            ));
        }
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.resolve(&operation, committed))
            .await
            .map_err(|e| FontFerryError::State(e.to_string()))?
    }

    async fn install(
        &self,
        font: &FontDefinition,
        version: &str,
        prepared: &[PreparedFont],
        previous: Option<&InstalledFont>,
    ) -> Result<InstallOutcome> {
        if prepared.is_empty() {
            return Err(FontFerryError::FontRejected(
                "the artifact contains no supported fonts".into(),
            ));
        }
        let (this, font, version, prepared, previous) = (
            self.clone(),
            font.clone(),
            version.to_owned(),
            prepared.to_vec(),
            previous.cloned(),
        );
        tokio::task::spawn_blocking(move || {
            this.begin(&font, Some(&version), &prepared, previous.as_ref())
        })
        .await
        .map_err(|e| FontFerryError::State(e.to_string()))?
    }

    async fn uninstall(&self, installed: &InstalledFont) -> Result<()> {
        let (this, installed) = (self.clone(), installed.clone());
        tokio::task::spawn_blocking(move || {
            // The definition is irrelevant to uninstall; only a journal and backup are needed.
            if this.journal().exists() {
                return Err(FontFerryError::State(
                    "unfinished operation requires recovery".into(),
                ));
            }
            let (backup, old_hashes) = this.backup(&installed)?;
            let operation = Operation {
                font_id: installed.font_id.clone(),
                version: None,
                old: Some(installed.clone()),
                backup: Some(backup),
                old_hashes,
                target: BTreeMap::new(),
            };
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(this.journal())
                .map_err(platform_error)?;
            serde_json::to_writer(&mut file, &operation)
                .map_err(|e| FontFerryError::State(e.to_string()))?;
            file.sync_all().map_err(platform_error)?;
            this.system.unregister(&installed.owned_files)?;
            this.system.refresh()
        })
        .await
        .map_err(|e| FontFerryError::State(e.to_string()))?
    }

    async fn restore(
        &self,
        font: &FontDefinition,
        snapshot: &RollbackSnapshot,
        current: &InstalledFont,
    ) -> Result<InstallOutcome> {
        let (this, font, snapshot, current) = (
            self.clone(),
            font.clone(),
            snapshot.clone(),
            current.clone(),
        );
        tokio::task::spawn_blocking(move || {
            let manifest_path = snapshot.backup_directory.join("manifest.json");
            let manifest: Option<BTreeMap<PathBuf, String>> = if manifest_path.exists() {
                Some(
                    serde_json::from_slice(&fs::read(&manifest_path).map_err(platform_error)?)
                        .map_err(|e| FontFerryError::State(e.to_string()))?,
                )
            } else {
                None
            }; // Legacy snapshots remain readable; font parsing is still mandatory.
            if let Some(manifest) = &manifest {
                if manifest.is_empty() {
                    return Err(FontFerryError::NoRollbackSnapshot);
                }
                for (path, expected) in manifest {
                    let name = path
                        .file_name()
                        .ok_or_else(|| FontFerryError::State("invalid snapshot manifest".into()))?;
                    if file_hash(&snapshot.backup_directory.join(name))? != *expected {
                        return Err(FontFerryError::FontRejected(
                            "corrupt rollback snapshot".into(),
                        ));
                    }
                }
            }
            let mut prepared = Vec::new();
            for entry in fs::read_dir(&snapshot.backup_directory).map_err(platform_error)? {
                let entry = entry.map_err(platform_error)?;
                if entry.file_name() == "manifest.json" {
                    continue;
                }
                if let Some(manifest) = &manifest
                    && !manifest
                        .keys()
                        .any(|path| path.file_name() == Some(entry.file_name().as_os_str()))
                {
                    return Err(FontFerryError::FontRejected(
                        "unexpected file in rollback snapshot".into(),
                    ));
                }
                if !entry.file_type().map_err(platform_error)?.is_file() {
                    return Err(FontFerryError::FontRejected(
                        "backup contains non-regular file".into(),
                    ));
                }
                prepared.push(crate::inspect_font_file(&entry.path())?);
            }
            if prepared.is_empty() {
                return Err(FontFerryError::NoRollbackSnapshot);
            }
            this.begin(&font, Some(&snapshot.version), &prepared, Some(&current))
        })
        .await
        .map_err(|e| FontFerryError::State(e.to_string()))?
    }
}

// NotFound is the only benign absence. Symlinks, access errors and changed bytes
// are never treated as an owned file that can be overwritten or removed.
fn verify_existing(path: &Path, expected: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(platform_error(error)),
        Ok(meta) if meta.is_file() && file_hash(path)? == expected => Ok(true),
        Ok(_) => Err(FontFerryError::State(format!(
            "file differs from recovery journal: {}; preserve file and journal for manual recovery",
            path.display()
        ))),
    }
}

fn file_hash(path: &Path) -> Result<String> {
    if !fs::symlink_metadata(path)
        .map_err(platform_error)?
        .is_file()
    {
        return Err(FontFerryError::Platform(
            "font path is not a regular file".into(),
        ));
    }
    let mut file = File::open(path).map_err(platform_error)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    loop {
        let count = file.read(&mut bytes).map_err(platform_error)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}
fn copy_new(source: &Path, destination: &Path) -> Result<()> {
    let mut input = File::open(source).map_err(platform_error)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(platform_error)?;
    std::io::copy(&mut input, &mut output).map_err(platform_error)?;
    output.sync_all().map_err(platform_error)
}
fn remove_directory(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(platform_error(e)),
    }
}
fn platform_error(error: std::io::Error) -> FontFerryError {
    FontFerryError::Platform(error.to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn run(command: &mut std::process::Command) -> Result<()> {
    let output = command.output().map_err(platform_error)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(FontFerryError::Platform(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

#[cfg(any(windows, test))]
fn remove_session_reference(
    add: impl FnOnce() -> bool,
    remove: impl FnOnce() -> Result<usize>,
) -> Result<()> {
    if !add() {
        return Err(FontFerryError::Platform(
            "could not establish font session reference for unregister; preserve journal and retry"
                .into(),
        ));
    }
    if remove()? == 0 {
        return Err(FontFerryError::Platform(
            "font session removal failed; preserve journal and retry after closing font users"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use std::{
        env,
        ffi::OsStr,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
    };

    use fontferry_core::{FontFerryError, Result};
    use windows_sys::Win32::{
        Graphics::Gdi::{AddFontResourceExW, RemoveFontResourceExW},
        UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_FONTCHANGE,
        },
    };
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    const FONT_REGISTRY_KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";

    pub fn install_directory() -> Result<PathBuf> {
        let base = env::var_os("LOCALAPPDATA")
            .ok_or_else(|| FontFerryError::Platform("LOCALAPPDATA is not available".into()))?;
        Ok(PathBuf::from(base)
            .join("Microsoft")
            .join("Windows")
            .join("Fonts"))
    }

    pub fn register(paths: &[PathBuf]) -> Result<()> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(FONT_REGISTRY_KEY)
            .map_err(|error| FontFerryError::Platform(error.to_string()))?
            .0;
        for path in paths {
            let name = registry_name(path)?;
            let persisted = registry_matches(&key, &name, path)?;
            let wide = wide_path(path);
            // Registry presence is not evidence that the current session loaded the font.
            // Normalize any references left by a interrupted attempt before adding exactly one.
            unload_session(&wide, 0)?;
            unload_session(&wide, 0x10)?; // FR_PRIVATE, for legacy resources in this process.
            // SAFETY: `wide` is NUL-terminated and live; flags select the public session.
            let added = unsafe { AddFontResourceExW(wide.as_ptr(), 0, std::ptr::null()) };
            if added == 0 {
                return Err(FontFerryError::Platform(format!(
                    "Windows rejected font {}",
                    path.display()
                )));
            }
            if !persisted && let Err(error) = key.set_value(&name, &path.as_os_str()) {
                // SAFETY: same path and flags as the single successful add above.
                let removed = unsafe { RemoveFontResourceExW(wide.as_ptr(), 0, std::ptr::null()) };
                return Err(FontFerryError::Platform(format!(
                    "registry write failed: {error}; session compensation succeeded: {}",
                    removed != 0
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
            let persisted = registry_matches(&key, &name, path)?;
            let wide = wide_path(path);
            if path.exists() {
                // A preceding attempt may have drained GDI but failed deleting the registry
                // value. Establish one known public reference so a zero initial count is
                // not confused with an unregister failure on every subsequent retry.
                super::remove_session_reference(
                    || unsafe { AddFontResourceExW(wide.as_ptr(), 0, std::ptr::null()) } != 0,
                    || unload_session(&wide, 0),
                )?;
                unload_session(&wide, 0x10)?; // Matching legacy FR_PRIVATE flags.
            } else {
                unload_session(&wide, 0)?;
                unload_session(&wide, 0x10)?;
            }
            if persisted {
                key.delete_value(name)
                    .map_err(|error| FontFerryError::Platform(error.to_string()))?;
            }
        }
        Ok(())
    }

    fn registry_matches(key: &RegKey, name: &str, path: &Path) -> Result<bool> {
        match key.get_value::<String, _>(name) {
            Ok(existing) if Path::new(&existing) == path => Ok(true),
            Ok(_) => Err(FontFerryError::Platform(
                "font registry ownership mismatch".into(),
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(FontFerryError::Platform(error.to_string())),
        }
    }

    fn unload_session(wide: &[u16], flags: u32) -> Result<usize> {
        // Microsoft documents repeated removal when outstanding resource references exist.
        // The cap prevents an externally changing resource count from hanging recovery.
        for removed in 0..1024 {
            // SAFETY: callers supply a live NUL-terminated managed path and matching flags.
            if unsafe { RemoveFontResourceExW(wide.as_ptr(), flags, std::ptr::null()) } == 0 {
                return Ok(removed);
            }
        }
        Err(FontFerryError::Platform(
            "font resource references did not drain; recovery required".into(),
        ))
    }

    pub fn refresh() -> Result<()> {
        let mut result = 0_usize;
        // SAFETY: Broadcasting WM_FONTCHANGE requires no pointer payload; the result pointer is valid.
        let sent = unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_FONTCHANGE,
                0,
                0,
                SMTO_ABORTIFHUNG,
                1_000,
                &mut result,
            )
        };
        if sent == 0 {
            return Err(FontFerryError::Platform(
                "WM_FONTCHANGE broadcast failed or timed out".into(),
            ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteState;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct TestSystem {
        root: PathBuf,
        registered: Mutex<BTreeSet<PathBuf>>,
        fail_register: AtomicBool,
        fail_copy: AtomicUsize,
        fail_remove: AtomicBool,
        fail_refresh: AtomicBool,
        fail_unregister: AtomicBool,
    }
    impl FontSystem for TestSystem {
        fn copy(&self, source: &Path, target: &Path) -> Result<()> {
            if self
                .fail_copy
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .ok()
                == Some(1)
            {
                fs::write(target, b"partial").map_err(platform_error)?;
                return Err(FontFerryError::Platform("injected copy failure".into()));
            }
            copy_new(source, target)
        }
        fn directory(&self) -> Result<PathBuf> {
            Ok(self.root.clone())
        }
        fn register(&self, paths: &[PathBuf]) -> Result<()> {
            let mut registered = self
                .registered
                .lock()
                .map_err(|e| FontFerryError::State(e.to_string()))?;
            for path in paths {
                registered.insert(path.clone());
                if self.fail_register.swap(false, Ordering::SeqCst) {
                    return Err(FontFerryError::Platform(
                        "injected partial registration failure".into(),
                    ));
                }
            }
            Ok(())
        }
        fn unregister(&self, paths: &[PathBuf]) -> Result<()> {
            let mut registered = self
                .registered
                .lock()
                .map_err(|e| FontFerryError::State(e.to_string()))?;
            for path in paths {
                registered.remove(path);
                if self.fail_unregister.swap(false, Ordering::SeqCst) {
                    return Err(FontFerryError::Platform(
                        "injected unregister failure".into(),
                    ));
                }
            }
            Ok(())
        }
        fn refresh(&self) -> Result<()> {
            if self.fail_refresh.swap(false, Ordering::SeqCst) {
                return Err(FontFerryError::Platform("injected refresh failure".into()));
            }
            Ok(())
        }
        fn remove(&self, path: &Path) -> std::io::Result<()> {
            if self.fail_remove.swap(false, Ordering::SeqCst) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected file in use",
                ));
            }
            fs::remove_file(path)
        }
    }

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    fn setup(root: &Path) -> Result<(PlatformFontInstaller, Arc<TestSystem>, SqliteState)> {
        let paths = AppPaths {
            data: root.join("data"),
            cache: root.join("cache"),
            logs: root.join("logs"),
            staging: root.join("staging"),
            backups: root.join("backups"),
        };
        for path in [
            &paths.data,
            &paths.cache,
            &paths.logs,
            &paths.staging,
            &paths.backups,
        ] {
            fs::create_dir_all(path).map_err(platform_error)?;
        }
        let state = SqliteState::open(&paths.database())?;
        let system = Arc::new(TestSystem {
            root: root.join("fonts"),
            registered: Mutex::new(BTreeSet::new()),
            fail_register: AtomicBool::new(false),
            fail_copy: AtomicUsize::new(0),
            fail_remove: AtomicBool::new(false),
            fail_refresh: AtomicBool::new(false),
            fail_unregister: AtomicBool::new(false),
        });
        Ok((
            PlatformFontInstaller {
                paths,
                system: system.clone(),
            },
            system,
            state,
        ))
    }
    fn definition() -> Result<FontDefinition> {
        let catalog: fontferry_core::Catalog =
            serde_json::from_str(include_str!("../../../catalog/builtin/catalog.json"))
                .map_err(|e| FontFerryError::State(e.to_string()))?;
        catalog
            .fonts
            .into_iter()
            .next()
            .ok_or_else(|| FontFerryError::State("empty test catalog".into()))
    }
    fn prepared(root: &Path, name: &str, bytes: &[u8]) -> Result<PreparedFont> {
        let path = root.join(name);
        fs::write(&path, bytes).map_err(platform_error)?;
        Ok(PreparedFont {
            sha256: file_hash(&path)?,
            path,
            family: "test".into(),
            style: "regular".into(),
            postscript_name: None,
            version: None,
        })
    }
    fn record(font: &FontDefinition, version: &str, outcome: InstallOutcome) -> InstalledFont {
        InstalledFont {
            font_id: font.id.clone(),
            version: version.into(),
            variant_ids: vec!["test".into()],
            installed_at: time::OffsetDateTime::now_utc(),
            owned_files: outcome.owned_files,
            previous: outcome.previous_snapshot,
            manual_version: None,
        }
    }

    #[tokio::test]
    async fn repeated_install_and_partial_upgrade_preserve_shared_files() -> TestResult {
        let root = tempfile::tempdir()?;
        let (installer, system, state) = setup(root.path())?;
        let font = definition()?;
        let a = prepared(root.path(), "a.ttf", b"payload a")?;
        let b = prepared(root.path(), "b.ttf", b"payload b")?;
        let first = record(
            &font,
            "1.0",
            installer
                .install(&font, "1.0", &[a.clone(), a.clone()], None)
                .await?,
        );
        assert_eq!(first.owned_files.len(), 1);
        state.save_installed(&first).await?;
        installer.finish(&state).await?;
        let repeated = record(
            &font,
            "1.0",
            installer
                .install(&font, "1.0", std::slice::from_ref(&a), Some(&first))
                .await?,
        );
        state.save_installed(&repeated).await?;
        installer.finish(&state).await?;
        assert_eq!(first.owned_files, repeated.owned_files);
        let second = record(
            &font,
            "2.0",
            installer
                .install(&font, "2.0", &[a, b], Some(&repeated))
                .await?,
        );
        state.save_installed(&second).await?;
        installer.finish(&state).await?;
        assert!(second.owned_files.iter().all(|p| p.is_file()));
        assert_eq!(
            system.registered.lock().map_err(|e| e.to_string())?.len(),
            2
        );
        assert!(
            second
                .previous
                .as_ref()
                .is_some_and(|p| p.backup_directory.is_dir())
        );
        Ok(())
    }

    #[tokio::test]
    async fn uncommitted_registration_failure_recovers_files_and_sqlite() -> TestResult {
        let root = tempfile::tempdir()?;
        let (installer, system, state) = setup(root.path())?;
        let font = definition()?;
        let a = prepared(root.path(), "a.ttf", b"a")?;
        let first = record(&font, "1", installer.install(&font, "1", &[a], None).await?);
        state.save_installed(&first).await?;
        installer.finish(&state).await?;
        let b = prepared(root.path(), "b.ttf", b"b")?;
        system.fail_register.store(true, Ordering::SeqCst);
        assert!(
            installer
                .install(&font, "2", &[b], Some(&first))
                .await
                .is_err()
        );
        assert!(installer.journal().exists());
        // Reconstruct installer to model a fresh process, preserving only disk and system state.
        let restarted = PlatformFontInstaller {
            paths: installer.paths.clone(),
            system: system.clone(),
        };
        restarted.recover(&state).await?;
        assert_eq!(state.get_installed(&font.id).await?, Some(first.clone()));
        assert!(first.owned_files.iter().all(|p| p.exists()));
        assert_eq!(fs::read_dir(&system.root)?.count(), 1);
        assert!(!restarted.journal().exists());
        Ok(())
    }

    #[tokio::test]
    async fn database_failure_and_committed_restart_select_opposite_recovery_directions()
    -> TestResult {
        let root = tempfile::tempdir()?;
        let (installer, system, state) = setup(root.path())?;
        let font = definition()?;
        let a = prepared(root.path(), "a.ttf", b"a")?;
        let first = record(&font, "1", installer.install(&font, "1", &[a], None).await?);
        state.save_installed(&first).await?;
        installer.finish(&state).await?;
        let b = prepared(root.path(), "b.ttf", b"b")?;
        let uncommitted = record(
            &font,
            "2",
            installer
                .install(&font, "2", std::slice::from_ref(&b), Some(&first))
                .await?,
        );
        // Trigger a real SQLite write failure, without altering the existing record.
        let connection = rusqlite::Connection::open(installer.paths.database())?;
        connection.execute_batch("CREATE TRIGGER fail_update BEFORE UPDATE ON installed_fonts BEGIN SELECT RAISE(FAIL, 'injected commit failure'); END;")?;
        assert!(state.save_installed(&uncommitted).await.is_err());
        installer.recover(&state).await?;
        assert!(first.owned_files.iter().all(|p| p.exists()));
        assert!(uncommitted.owned_files.iter().all(|p| !p.exists()));
        connection.execute_batch("DROP TRIGGER fail_update;")?;
        let committed = record(
            &font,
            "2",
            installer.install(&font, "2", &[b], Some(&first)).await?,
        );
        state.save_installed(&committed).await?;
        assert!(first.owned_files.iter().all(|p| p.exists()));
        installer.recover(&state).await?;
        assert!(first.owned_files.iter().all(|p| !p.exists()));
        assert!(committed.owned_files.iter().all(|p| p.exists()));
        assert_eq!(fs::read_dir(&system.root)?.count(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn second_copy_failure_preserves_unverified_partial_file_and_journal() -> TestResult {
        let root = tempfile::tempdir()?;
        let (installer, system, state) = setup(root.path())?;
        let font = definition()?;
        let a = prepared(root.path(), "a.ttf", b"a")?;
        let b = prepared(root.path(), "b.ttf", b"b")?;
        system.fail_copy.store(2, Ordering::SeqCst);
        assert!(installer.install(&font, "1", &[a, b], None).await.is_err());
        assert!(installer.recover(&state).await.is_err());
        assert!(installer.journal().exists());
        assert_eq!(fs::read_dir(&system.root)?.count(), 2);
        assert!(
            fs::read_dir(&system.root)?
                .filter_map(std::result::Result::ok)
                .any(|entry| fs::read(entry.path()).is_ok_and(|bytes| bytes == b"partial"))
        );
        assert!(state.get_installed(&font.id).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn cleanup_failure_retains_journal_and_retries_after_commit() -> TestResult {
        let root = tempfile::tempdir()?;
        let (installer, system, state) = setup(root.path())?;
        let font = definition()?;
        let a = prepared(root.path(), "a.ttf", b"a")?;
        let installed = record(&font, "1", installer.install(&font, "1", &[a], None).await?);
        state.save_installed(&installed).await?;
        installer.finish(&state).await?;
        installer.uninstall(&installed).await?;
        state.remove_installed(&font.id).await?;
        system.fail_remove.store(true, Ordering::SeqCst);
        assert!(installer.finish(&state).await.is_err());
        assert!(installer.journal().exists());
        assert!(installed.owned_files.iter().all(|p| p.exists()));
        installer.recover(&state).await?;
        assert!(!installer.journal().exists());
        assert!(installed.owned_files.iter().all(|p| !p.exists()));
        Ok(())
    }

    #[tokio::test]
    async fn unregister_and_refresh_failures_restore_old_registration() -> TestResult {
        for fail_unregister in [true, false] {
            let root = tempfile::tempdir()?;
            let (installer, system, state) = setup(root.path())?;
            let font = definition()?;
            let a = prepared(root.path(), "a.ttf", b"a")?;
            let first = record(&font, "1", installer.install(&font, "1", &[a], None).await?);
            state.save_installed(&first).await?;
            installer.finish(&state).await?;
            let b = prepared(root.path(), "b.ttf", b"b")?;
            if fail_unregister {
                system.fail_unregister.store(true, Ordering::SeqCst);
            } else {
                system.fail_refresh.store(true, Ordering::SeqCst);
            }
            assert!(
                installer
                    .install(&font, "2", &[b], Some(&first))
                    .await
                    .is_err()
            );
            installer.recover(&state).await?;
            assert_eq!(
                *system.registered.lock().map_err(|e| e.to_string())?,
                first.owned_files.iter().cloned().collect()
            );
            assert!(first.owned_files.iter().all(|p| p.exists()));
        }
        Ok(())
    }

    #[test]
    fn session_cleanup_can_retry_after_gdi_was_drained_before_registry_failure() -> TestResult {
        let references = std::cell::Cell::new(0);
        for _ in 0..2 {
            remove_session_reference(
                || {
                    references.set(references.get() + 1);
                    true
                },
                || Ok(references.replace(0)),
            )?;
            assert_eq!(references.get(), 0);
        }
        assert!(remove_session_reference(|| true, || Ok(0)).is_err());
        assert!(remove_session_reference(|| false, || Ok(1)).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn recovery_preserves_replaced_obsolete_files_in_both_directions() -> TestResult {
        for committed in [false, true] {
            let root = tempfile::tempdir()?;
            let (installer, system, state) = setup(root.path())?;
            let font = definition()?;
            let a = prepared(root.path(), "a.ttf", b"a")?;
            let first = record(&font, "1", installer.install(&font, "1", &[a], None).await?);
            state.save_installed(&first).await?;
            installer.finish(&state).await?;
            let b = prepared(root.path(), "b.ttf", b"b")?;
            let next = record(
                &font,
                "2",
                installer.install(&font, "2", &[b], Some(&first)).await?,
            );
            if committed {
                state.save_installed(&next).await?;
            }
            let obsolete = if committed {
                &first.owned_files[0]
            } else {
                &next.owned_files[0]
            };
            fs::write(obsolete, b"external replacement")?;
            let before = system.registered.lock().map_err(|e| e.to_string())?.clone();
            assert!(installer.recover(&state).await.is_err());
            assert_eq!(fs::read(obsolete)?, b"external replacement");
            assert_eq!(
                *system.registered.lock().map_err(|e| e.to_string())?,
                before
            );
            assert!(installer.journal().exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn recovery_preflights_all_sources_and_destination_conflicts() -> TestResult {
        for conflict in [false, true] {
            let root = tempfile::tempdir()?;
            let (installer, system, state) = setup(root.path())?;
            let font = definition()?;
            let a = prepared(root.path(), "a.ttf", b"a")?;
            let b = prepared(root.path(), "b.ttf", b"b")?;
            let first = record(
                &font,
                "1",
                installer.install(&font, "1", &[a, b], None).await?,
            );
            state.save_installed(&first).await?;
            installer.finish(&state).await?;
            installer.uninstall(&first).await?;
            let operation: Operation = serde_json::from_slice(&fs::read(installer.journal())?)?;
            let paths: Vec<_> = operation.old_hashes.keys().collect();
            fs::remove_file(paths[0])?;
            if conflict {
                fs::write(paths[1], b"replacement")?;
            } else {
                fs::remove_file(paths[1])?;
                let backup = operation.backup.as_ref().ok_or("missing snapshot")?;
                fs::write(
                    backup
                        .backup_directory
                        .join(paths[1].file_name().ok_or("missing name")?),
                    b"corrupt",
                )?;
            }
            let before = system.registered.lock().map_err(|e| e.to_string())?.clone();
            assert!(installer.recover(&state).await.is_err());
            assert!(!paths[0].exists());
            assert_eq!(
                *system.registered.lock().map_err(|e| e.to_string())?,
                before
            );
            assert!(installer.journal().exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn invalid_journal_invariants_do_not_mutate_files() -> TestResult {
        let root = tempfile::tempdir()?;
        let (installer, _, state) = setup(root.path())?;
        let font = definition()?;
        let a = prepared(root.path(), "a.ttf", b"a")?;
        installer.install(&font, "1", &[a], None).await?;
        let mut operation: Operation = serde_json::from_slice(&fs::read(installer.journal())?)?;
        let outside = root.path().join("unmanaged.ttf");
        fs::write(&outside, b"unmanaged")?;
        operation
            .target
            .insert(outside.clone(), file_hash(&outside)?);
        fs::write(installer.journal(), serde_json::to_vec(&operation)?)?;
        assert!(installer.recover(&state).await.is_err());
        assert_eq!(fs::read(outside)?, b"unmanaged");
        assert!(installer.journal().exists());
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recovery_refuses_dangling_symlink_before_restoring_any_file() -> TestResult {
        let root = tempfile::tempdir()?;
        let (installer, _, state) = setup(root.path())?;
        let font = definition()?;
        let a = prepared(root.path(), "a.ttf", b"a")?;
        let b = prepared(root.path(), "b.ttf", b"b")?;
        let first = record(
            &font,
            "1",
            installer.install(&font, "1", &[a, b], None).await?,
        );
        state.save_installed(&first).await?;
        installer.finish(&state).await?;
        installer.uninstall(&first).await?;
        for path in &first.owned_files {
            fs::remove_file(path)?;
        }
        std::os::unix::fs::symlink(root.path().join("absent"), &first.owned_files[1])?;
        assert!(installer.recover(&state).await.is_err());
        assert!(!first.owned_files[0].exists());
        assert!(
            fs::symlink_metadata(&first.owned_files[1])?
                .file_type()
                .is_symlink()
        );
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_snapshot_is_rejected_before_removing_current_font() -> TestResult {
        let root = tempfile::tempdir()?;
        let (installer, _, state) = setup(root.path())?;
        let font = definition()?;
        let a = prepared(root.path(), "a.ttf", b"a")?;
        let first = record(&font, "1", installer.install(&font, "1", &[a], None).await?);
        state.save_installed(&first).await?;
        installer.finish(&state).await?;
        let missing = RollbackSnapshot {
            version: "0".into(),
            variant_ids: Vec::new(),
            backup_directory: root.path().join("missing"),
        };
        assert!(installer.restore(&font, &missing, &first).await.is_err());
        fs::create_dir(&missing.backup_directory)?;
        assert!(installer.restore(&font, &missing, &first).await.is_err());
        fs::write(missing.backup_directory.join("broken.ttf"), b"broken")?;
        assert!(installer.restore(&font, &missing, &first).await.is_err());
        assert!(first.owned_files.iter().all(|p| p.exists()));
        Ok(())
    }
}
