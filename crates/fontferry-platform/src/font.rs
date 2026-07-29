use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use flate2::read::GzDecoder;
use fontferry_core::{FontFerryError, FontPreparer, PreparedFont, Result};
use sha2::{Digest, Sha256};
use tar::Archive;
use ttf_parser::{Face, name_id};
use walkdir::WalkDir;
use zip::ZipArchive;

const MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;

#[derive(Clone, Debug, Default)]
pub struct SafeFontPreparer;

#[async_trait]
impl FontPreparer for SafeFontPreparer {
    async fn prepare(
        &self,
        downloaded: &[PathBuf],
        staging_directory: &Path,
    ) -> Result<Vec<PreparedFont>> {
        let downloaded = downloaded.to_vec();
        let staging_directory = staging_directory.to_path_buf();
        tokio::task::spawn_blocking(move || prepare_sync(&downloaded, &staging_directory))
            .await
            .map_err(|error| FontFerryError::State(error.to_string()))?
    }
}

fn prepare_sync(downloaded: &[PathBuf], staging_directory: &Path) -> Result<Vec<PreparedFont>> {
    let extracted = staging_directory.join("extracted");
    fs::create_dir_all(&extracted).map_err(|error| FontFerryError::State(error.to_string()))?;
    for (index, path) in downloaded.iter().enumerate() {
        let destination = extracted.join(index.to_string());
        fs::create_dir_all(&destination)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        let lower = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if is_font_path(path) {
            let filename = path
                .file_name()
                .ok_or_else(|| FontFerryError::FontRejected("font has no filename".into()))?;
            fs::copy(path, destination.join(filename))
                .map_err(|error| FontFerryError::State(error.to_string()))?;
        } else if lower.ends_with(".zip") {
            extract_zip(path, &destination)?;
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            extract_tar_gz(path, &destination)?;
        } else if lower.ends_with(".7z") {
            sevenz_rust::decompress_file(path, &destination)
                .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
        } else {
            return Err(FontFerryError::ArchiveRejected(format!(
                "unsupported file '{}'",
                path.display()
            )));
        }
    }
    validate_extracted_tree(&extracted)?;

    let mut prepared = Vec::new();
    for entry in WalkDir::new(&extracted).follow_links(false) {
        let entry = entry.map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
        if entry.file_type().is_file() && is_font_path(entry.path()) {
            prepared.push(inspect_font(entry.path())?);
        }
    }
    if prepared.is_empty() {
        return Err(FontFerryError::FontRejected(
            "download contains no supported font files".into(),
        ));
    }
    Ok(prepared)
}

fn extract_zip(source: &Path, destination: &Path) -> Result<()> {
    let file = File::open(source).map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(FontFerryError::ArchiveRejected(
            "archive contains too many entries".into(),
        ));
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(FontFerryError::ArchiveRejected(
                "archive contains a symbolic link".into(),
            ));
        }
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            FontFerryError::ArchiveRejected("archive entry escapes destination".into())
        })?;
        let target = destination.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_EXTRACTED_BYTES {
            return Err(FontFerryError::ArchiveRejected(
                "archive exceeds the 4 GiB extraction limit".into(),
            ));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
        }
        let mut output = File::create(target)
            .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
        output
            .flush()
            .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
    }
    Ok(())
}

fn extract_tar_gz(source: &Path, destination: &Path) -> Result<()> {
    let file = File::open(source).map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut count = 0_usize;
    let mut total = 0_u64;
    for entry in archive
        .entries()
        .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?
    {
        let mut entry =
            entry.map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err(FontFerryError::ArchiveRejected(
                "archive contains too many entries".into(),
            ));
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(FontFerryError::ArchiveRejected(
                "archive contains a link".into(),
            ));
        }
        total = total.saturating_add(entry.header().size().unwrap_or(0));
        if total > MAX_EXTRACTED_BYTES {
            return Err(FontFerryError::ArchiveRejected(
                "archive exceeds the 4 GiB extraction limit".into(),
            ));
        }
        entry
            .unpack_in(destination)
            .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
    }
    Ok(())
}

fn validate_extracted_tree(root: &Path) -> Result<()> {
    let canonical_root =
        fs::canonicalize(root).map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
    let mut total = 0_u64;
    let mut count = 0_usize;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err(FontFerryError::ArchiveRejected(
                "extracted tree contains too many entries".into(),
            ));
        }
        if entry.path_is_symlink() {
            return Err(FontFerryError::ArchiveRejected(
                "extracted tree contains a symbolic link".into(),
            ));
        }
        let canonical = fs::canonicalize(entry.path())
            .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(FontFerryError::ArchiveRejected(
                "extracted path escapes destination".into(),
            ));
        }
        if entry.file_type().is_file() {
            total = total.saturating_add(
                entry
                    .metadata()
                    .map_err(|error| FontFerryError::ArchiveRejected(error.to_string()))?
                    .len(),
            );
            if total > MAX_EXTRACTED_BYTES {
                return Err(FontFerryError::ArchiveRejected(
                    "extracted tree exceeds 4 GiB".into(),
                ));
            }
        }
    }
    Ok(())
}

fn inspect_font(path: &Path) -> Result<PreparedFont> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| FontFerryError::FontRejected(error.to_string()))?;
    let face = Face::parse(&bytes, 0)
        .map_err(|_| FontFerryError::FontRejected(format!("invalid font '{}'", path.display())))?;
    let name = |id| {
        face.names()
            .into_iter()
            .filter(|name| name.name_id == id)
            .find_map(|name| name.to_string())
    };
    let family = name(name_id::TYPOGRAPHIC_FAMILY)
        .or_else(|| name(name_id::FAMILY))
        .ok_or_else(|| FontFerryError::FontRejected("font has no family name".into()))?;
    let style = name(name_id::TYPOGRAPHIC_SUBFAMILY)
        .or_else(|| name(name_id::SUBFAMILY))
        .unwrap_or_else(|| "Regular".into());
    let version = name(name_id::VERSION).map(|value| {
        value
            .strip_prefix("Version ")
            .unwrap_or(&value)
            .trim()
            .to_owned()
    });
    Ok(PreparedFont {
        path: path.to_path_buf(),
        family,
        style,
        postscript_name: name(name_id::POST_SCRIPT_NAME),
        version,
        sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

fn is_font_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    #[test]
    fn rejects_zip_path_traversal() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let root = temporary.path();
        let archive_path = root.join("bad.zip");
        let file = File::create(&archive_path)?;
        let mut archive = ZipWriter::new(file);
        archive.start_file("../escape.ttf", SimpleFileOptions::default())?;
        archive.write_all(b"not-a-font")?;
        archive.finish()?;
        assert!(extract_zip(&archive_path, &root.join("out")).is_err());
        Ok(())
    }

    #[test]
    fn rejects_non_font_bytes() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let path = temporary.path().join("fake.ttf");
        fs::write(&path, b"not a font")?;
        assert!(inspect_font(&path).is_err());
        Ok(())
    }
}
