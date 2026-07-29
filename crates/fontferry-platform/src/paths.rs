use std::{fs, path::PathBuf};

use directories::ProjectDirs;
use fontferry_core::{FontFerryError, Result};

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub data: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub staging: PathBuf,
    pub backups: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project = ProjectDirs::from("io.github", "teckc", "FontFerry")
            .ok_or_else(|| FontFerryError::State("cannot locate user data directory".into()))?;
        let data = project.data_local_dir().to_path_buf();
        let cache = project.cache_dir().to_path_buf();
        let paths = Self {
            logs: data.join("logs"),
            staging: cache.join("staging"),
            backups: data.join("backups"),
            data,
            cache,
        };
        for directory in [
            &paths.data,
            &paths.cache,
            &paths.logs,
            &paths.staging,
            &paths.backups,
        ] {
            fs::create_dir_all(directory)
                .map_err(|error| FontFerryError::State(error.to_string()))?;
        }
        Ok(paths)
    }

    pub fn database(&self) -> PathBuf {
        self.data.join("fontferry.db")
    }
}

