use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    FontDefinition, Release, Result,
    catalog::{ArtifactProvider, VersionProvider},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledFont {
    pub font_id: String,
    pub version: String,
    pub variant_ids: Vec<String>,
    pub installed_at: OffsetDateTime,
    pub owned_files: Vec<PathBuf>,
    pub previous: Option<RollbackSnapshot>,
    pub manual_version: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RollbackSnapshot {
    pub version: String,
    pub variant_ids: Vec<String>,
    pub backup_directory: PathBuf,
}

#[derive(Clone, Debug)]
pub struct PreparedFont {
    pub path: PathBuf,
    pub family: String,
    pub style: String,
    pub postscript_name: Option<String>,
    pub version: Option<String>,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct InstallOutcome {
    pub owned_files: Vec<PathBuf>,
    pub previous_snapshot: Option<RollbackSnapshot>,
    pub restart_recommended: bool,
    pub warnings: Vec<String>,
}

#[async_trait]
pub trait ReleaseSource: Send + Sync {
    async fn releases(
        &self,
        font: &FontDefinition,
        provider: &VersionProvider,
    ) -> Result<Vec<Release>>;
}

#[async_trait]
pub trait ArtifactSource: Send + Sync {
    async fn download(
        &self,
        font: &FontDefinition,
        provider: &ArtifactProvider,
        release: &Release,
        variant_ids: &[String],
        staging_directory: &Path,
    ) -> Result<Vec<PathBuf>>;
}

#[async_trait]
pub trait FontPreparer: Send + Sync {
    async fn prepare(
        &self,
        downloaded: &[PathBuf],
        staging_directory: &Path,
    ) -> Result<Vec<PreparedFont>>;
}

#[async_trait]
pub trait FontInstaller: Send + Sync {
    async fn install(
        &self,
        font: &FontDefinition,
        version: &str,
        prepared: &[PreparedFont],
        previous: Option<&InstalledFont>,
    ) -> Result<InstallOutcome>;

    async fn uninstall(&self, installed: &InstalledFont) -> Result<()>;

    async fn restore(
        &self,
        font: &FontDefinition,
        snapshot: &RollbackSnapshot,
        current: &InstalledFont,
    ) -> Result<InstallOutcome>;
}

#[async_trait]
pub trait StateRepository: Send + Sync {
    async fn list_installed(&self) -> Result<Vec<InstalledFont>>;
    async fn get_installed(&self, font_id: &str) -> Result<Option<InstalledFont>>;
    async fn save_installed(&self, installed: &InstalledFont) -> Result<()>;
    async fn remove_installed(&self, font_id: &str) -> Result<()>;
    async fn is_license_accepted(&self, font_id: &str, revision: &str) -> Result<bool>;
    async fn accept_license(&self, font_id: &str, revision: &str) -> Result<()>;
    async fn append_activity(&self, activity: &Activity) -> Result<()>;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: String,
    pub font_id: Option<String>,
    pub level: ActivityLevel,
    pub message: String,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActivityLevel {
    Info,
    Warning,
    Error,
}
