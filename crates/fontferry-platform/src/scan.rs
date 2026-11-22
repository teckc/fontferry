use std::{
    env,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use fontferry_core::ObservedFont;
use regex::Regex;
use time::OffsetDateTime;
use ttf_parser::{name, name_id};
use walkdir::WalkDir;

const MAX_SCANNED_FILES: usize = 50_000;
const MAX_NAME_TABLE_BYTES: u32 = 1024 * 1024;

#[must_use]
pub fn scan_font_awesome() -> Option<ObservedFont> {
    scan_directories(font_directories())
}

fn scan_directories(roots: impl IntoIterator<Item = PathBuf>) -> Option<ObservedFont> {
    let version_pattern = Regex::new(r"(?i)(?:version\s*)?v?(\d+\.\d+(?:\.\d+)?)").ok()?;
    let mut best_version: Option<String> = None;
    let mut observed_files = Vec::new();
    let mut visited = 0_usize;
    'directories: for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if visited >= MAX_SCANNED_FILES {
                break 'directories;
            }
            visited += 1;
            if !entry.file_type().is_file() || !is_font(entry.path()) {
                continue;
            }
            let Ok(mut file) = File::open(entry.path()) else {
                continue;
            };
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            let Some(table_bytes) = read_name_table(&mut file, metadata.len()) else {
                continue;
            };
            let Some(table) = name::Table::parse(&table_bytes) else {
                continue;
            };
            let Some(version) = pro_version(table.names, &version_pattern) else {
                continue;
            };
            observed_files.push(entry.path().to_path_buf());
            if let Some(version) = version
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

/// Read only the SFNT directory and naming table, not glyphs or file hashes.
/// Collections use their first face, as did the original full-font parser.
/// This is discovery only; installation still runs inspect_font_file.
fn read_name_table<R: Read + Seek>(reader: &mut R, file_len: u64) -> Option<Vec<u8>> {
    let mut header = [0_u8; 12];
    reader.read_exact(&mut header).ok()?;
    let mut face_offset = 0;
    if &header[..4] == b"ttcf" {
        if u32::from_be_bytes(header[8..12].try_into().ok()?) == 0 {
            return None;
        }
        let mut offset = [0_u8; 4];
        reader.read_exact(&mut offset).ok()?;
        face_offset = u64::from(u32::from_be_bytes(offset));
        reader.seek(SeekFrom::Start(face_offset)).ok()?;
        reader.read_exact(&mut header).ok()?;
    }
    if !matches!(&header[..4], b"\0\x01\0\0" | b"OTTO" | b"true") {
        return None;
    }
    let entries = usize::from(u16::from_be_bytes(header[4..6].try_into().ok()?));
    if face_offset + 12 + entries as u64 * 16 > file_len {
        return None;
    }
    for _ in 0..entries {
        let mut record = [0_u8; 16];
        reader.read_exact(&mut record).ok()?;
        if &record[..4] != b"name" {
            continue;
        }
        let offset = u64::from(u32::from_be_bytes(record[8..12].try_into().ok()?));
        let length = u32::from_be_bytes(record[12..16].try_into().ok()?);
        if !(6..=MAX_NAME_TABLE_BYTES).contains(&length) || offset + u64::from(length) > file_len {
            return None;
        }
        let mut bytes = vec![0; usize::try_from(length).ok()?];
        reader.seek(SeekFrom::Start(offset)).ok()?;
        reader.read_exact(&mut bytes).ok()?;
        return Some(bytes);
    }
    None
}

// No match is different from positive Pro identification without a known version.
fn pro_version(names: name::Names<'_>, version_pattern: &Regex) -> Option<Option<String>> {
    let is_pro = names
        .into_iter()
        .filter(|name| {
            matches!(
                name.name_id,
                name_id::FAMILY
                    | name_id::TYPOGRAPHIC_FAMILY
                    | name_id::FULL_NAME
                    | name_id::POST_SCRIPT_NAME
            )
        })
        .filter_map(|name| name.to_string())
        .any(|value| {
            let compact: String = value
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .map(|ch| ch.to_ascii_lowercase())
                .collect();
            compact
                .strip_prefix("fontawesome")
                .map(|edition| edition.trim_start_matches(|ch: char| ch.is_ascii_digit()))
                .is_some_and(|edition| edition.starts_with("pro"))
        });
    is_pro.then(|| {
        names
            .into_iter()
            .filter(|name| name.name_id == name_id::VERSION)
            .filter_map(|name| name.to_string())
            .find_map(|value| {
                version_pattern
                    .captures(&value)?
                    .get(1)
                    .map(|value| value.as_str().to_owned())
            })
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

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor};

    use super::*;

    fn naming_table(family: &str, version: &str) -> Vec<u8> {
        let mut table = vec![0, 0, 0, 2, 0, 30]; // format, record count, string storage offset
        let mut strings = Vec::new();
        for (kind, value) in [(name_id::FAMILY, family), (name_id::VERSION, version)] {
            let utf16: Vec<u8> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
            for field in [
                3_u16,
                1,
                0x409,
                kind,
                utf16.len() as u16,
                strings.len() as u16,
            ] {
                table.extend(field.to_be_bytes());
            }
            strings.extend(utf16);
        }
        table.extend(strings);
        table
    }

    fn fixture(collection: bool, table: &[u8]) -> Vec<u8> {
        let mut data = if collection {
            // Collection v1, one face at absolute offset 16.
            b"ttcf\0\x01\0\0\0\0\0\x01\0\0\0\x10".to_vec()
        } else {
            Vec::new()
        };
        data.extend(b"\0\x01\0\0\0\x01\0\0\0\0\0\0");
        data.extend(b"name\0\0\0\0");
        data.extend(1024_u32.to_be_bytes());
        data.extend((table.len() as u32).to_be_bytes());
        data.resize(1024, 0);
        data.extend(table);
        data.resize(128 * 1024, 0);
        data
    }

    struct CountingReader {
        input: Cursor<Vec<u8>>,
        bytes_read: usize,
    }

    impl Read for CountingReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.input.read(buf)?;
            self.bytes_read += n;
            Ok(n)
        }
    }

    impl Seek for CountingReader {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.input.seek(position)
        }
    }

    #[test]
    fn names_only_seek_keeps_first_face_semantics_without_reading_glyphs() {
        for collection in [false, true] {
            let table = naming_table("Font Awesome 6 Pro", "Version 6.4.2");
            let data = fixture(collection, &table);
            let file_len = data.len() as u64;
            let mut reader = CountingReader {
                input: Cursor::new(data),
                bytes_read: 0,
            };
            assert_eq!(
                read_name_table(&mut reader, file_len).as_ref(),
                Some(&table)
            );
            assert_eq!(
                reader.bytes_read,
                table.len() + if collection { 44 } else { 28 }
            );
        }
    }

    #[test]
    fn scan_ignores_free_and_malformed_fonts() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let free = directory.path().join("free.ttf");
        let pro = directory.path().join("pro.otc");
        std::fs::write(
            &free,
            fixture(false, &naming_table("Font Awesome 9 Free", "9.1.0")),
        )?;
        std::fs::write(directory.path().join("malformed.otf"), b"OTTO")?;
        assert!(scan_directories([directory.path().to_path_buf()]).is_none());
        std::fs::write(
            &pro,
            fixture(true, &naming_table("Font Awesome 6 Pro", "Version 6.4.2")),
        )?;
        let observation =
            scan_directories([directory.path().to_path_buf()]).ok_or("Pro font not observed")?;
        assert_eq!(observation.detected_version.as_deref(), Some("6.4.2"));
        assert_eq!(observation.observed_files, vec![pro]);
        Ok(())
    }

    #[test]
    fn rejects_out_of_bounds_and_oversized_name_tables() {
        let mut data = fixture(false, &naming_table("Font Awesome Pro", "6.4.2"));
        data[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        let n = data.len() as u64;
        assert!(read_name_table(&mut Cursor::new(data), n).is_none());

        let mut data = fixture(false, &naming_table("Font Awesome Pro", "6.4.2"));
        data[24..28].copy_from_slice(&(MAX_NAME_TABLE_BYTES + 1).to_be_bytes());
        assert!(
            read_name_table(&mut Cursor::new(data), u64::from(MAX_NAME_TABLE_BYTES) * 2).is_none()
        );
    }
}
