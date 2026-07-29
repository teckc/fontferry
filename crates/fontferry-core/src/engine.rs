use std::{collections::HashMap, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use time::OffsetDateTime;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    Activity, ActivityLevel, ArtifactSource, DeliveryPolicy, FontDefinition, FontFerryError,
    FontInstaller, FontPreparer, InstalledFont, Release, ReleaseChannel, ReleaseSource, Result,
    RollbackSnapshot, StateRepository, is_update_available, select_latest,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub font_id: String,
    pub current_version: Option<String>,
    pub available_version: Option<String>,
    pub update_available: bool,
    pub delivery_policy: DeliveryPolicy,
}

#[derive(Clone, Debug)]
pub struct InstallRequest {
    pub font_id: String,
    pub version: Option<String>,
    pub variant_ids: Vec<String>,
    pub accept_license: bool,
}

pub struct FontEngine {
    catalog: HashMap<String, FontDefinition>,
    releases: Arc<dyn ReleaseSource>,
    artifacts: Arc<dyn ArtifactSource>,
    preparer: Arc<dyn FontPreparer>,
    installer: Arc<dyn FontInstaller>,
    state: Arc<dyn StateRepository>,
    operation_lock: Mutex<()>,
    staging_root: PathBuf,
}

impl std::fmt::Debug for FontEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FontEngine")
            .field("catalog_size", &self.catalog.len())
            .field("staging_root", &self.staging_root)
            .finish_non_exhaustive()
    }
}

impl FontEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        catalog: Vec<FontDefinition>,
        releases: Arc<dyn ReleaseSource>,
        artifacts: Arc<dyn ArtifactSource>,
        preparer: Arc<dyn FontPreparer>,
        installer: Arc<dyn FontInstaller>,
        state: Arc<dyn StateRepository>,
        staging_root: PathBuf,
    ) -> Self {
        Self {
            catalog: catalog
                .into_iter()
                .map(|font| (font.id.clone(), font))
                .collect(),
            releases,
            artifacts,
            preparer,
            installer,
            state,
            operation_lock: Mutex::new(()),
            staging_root,
        }
    }

    pub fn fonts(&self) -> Vec<FontDefinition> {
        let mut fonts: Vec<_> = self.catalog.values().cloned().collect();
        fonts.sort_by(|left, right| left.name.cmp(&right.name));
        fonts
    }

    pub async fn check_font(&self, font_id: &str) -> Result<UpdateStatus> {
        let font = self.font(font_id)?;
        let releases = self
            .releases
            .releases(font, &font.version_provider)
            .await?;
        let latest = select_latest(
            &releases,
            release_channel(&font.version_provider),
            &font.version_policy,
        );
        let installed = self.state.get_installed(font_id).await?;
        let current = installed
            .as_ref()
            .and_then(|item| item.manual_version.as_ref().or(Some(&item.version)));
        let available = latest.map(|item| item.version.as_str());
        Ok(UpdateStatus {
            font_id: font_id.to_owned(),
            current_version: current.cloned(),
            available_version: available.map(str::to_owned),
            update_available: match (current, available) {
                (Some(current), Some(available)) => is_update_available(current, available),
                (None, Some(_)) => true,
                _ => false,
            },
            delivery_policy: font.delivery_policy,
        })
    }

    pub async fn install(&self, request: InstallRequest) -> Result<InstalledFont> {
        let _guard = self.operation_lock.lock().await;
        let font = self.font(&request.font_id)?.clone();
        if font.delivery_policy == DeliveryPolicy::NotifyOnly {
            return Err(FontFerryError::ReminderOnly);
        }
        if font.license.requires_acceptance
            && !self
                .state
                .is_license_accepted(&font.id, &font.license.revision)
                .await?
        {
            if !request.accept_license {
                return Err(FontFerryError::LicenseAcceptanceRequired);
            }
            self.state
                .accept_license(&font.id, &font.license.revision)
                .await?;
        }

        let releases = self
            .releases
            .releases(&font, &font.version_provider)
            .await?;
        let release = requested_release(
            &releases,
            request.version.as_deref(),
            release_channel(&font.version_provider),
            &font.version_policy,
        )?;
        let provider = font
            .artifact_provider
            .as_ref()
            .ok_or(FontFerryError::ReminderOnly)?;
        let staging = TempDir::new_in(&self.staging_root)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        let downloaded = self
            .artifacts
            .download(
                &font,
                provider,
                release,
                &request.variant_ids,
                staging.path(),
            )
            .await?;
        let prepared = self
            .preparer
            .prepare(&downloaded, staging.path())
            .await?;
        let previous = self.state.get_installed(&font.id).await?;
        let outcome = self
            .installer
            .install(&font, &release.version, &prepared, previous.as_ref())
            .await?;
        let installed = InstalledFont {
            font_id: font.id.clone(),
            version: release.version.clone(),
            variant_ids: request.variant_ids,
            installed_at: OffsetDateTime::now_utc(),
            owned_files: outcome.owned_files,
            previous: outcome.previous_snapshot,
            manual_version: None,
        };
        self.state.save_installed(&installed).await?;
        self.activity(
            Some(font.id),
            ActivityLevel::Info,
            format!("Installed {}", release.version),
        )
        .await?;
        Ok(installed)
    }

    pub async fn uninstall(&self, font_id: &str) -> Result<()> {
        let _guard = self.operation_lock.lock().await;
        if let Some(installed) = self.state.get_installed(font_id).await? {
            self.installer.uninstall(&installed).await?;
            self.state.remove_installed(font_id).await?;
        }
        Ok(())
    }

    pub async fn rollback(&self, font_id: &str) -> Result<InstalledFont> {
        let _guard = self.operation_lock.lock().await;
        let font = self.font(font_id)?.clone();
        let current = self
            .state
            .get_installed(font_id)
            .await?
            .ok_or(FontFerryError::NoRollbackSnapshot)?;
        let snapshot = current
            .previous
            .as_ref()
            .ok_or(FontFerryError::NoRollbackSnapshot)?;
        let outcome = self.installer.restore(&font, snapshot, &current).await?;
        let restored = InstalledFont {
            font_id: current.font_id,
            version: snapshot.version.clone(),
            variant_ids: snapshot.variant_ids.clone(),
            installed_at: OffsetDateTime::now_utc(),
            owned_files: outcome.owned_files,
            previous: None,
            manual_version: None,
        };
        self.state.save_installed(&restored).await?;
        Ok(restored)
    }

    fn font(&self, font_id: &str) -> Result<&FontDefinition> {
        self.catalog
            .get(font_id)
            .ok_or_else(|| FontFerryError::UnknownFont(font_id.to_owned()))
    }

    async fn activity(
        &self,
        font_id: Option<String>,
        level: ActivityLevel,
        message: String,
    ) -> Result<()> {
        self.state
            .append_activity(&Activity {
                id: Uuid::new_v4().to_string(),
                font_id,
                level,
                message,
                created_at: OffsetDateTime::now_utc(),
            })
            .await
    }
}

fn release_channel(provider: &crate::VersionProvider) -> ReleaseChannel {
    match provider {
        crate::VersionProvider::GitHubRelease { channel, .. } => *channel,
        _ => ReleaseChannel::Stable,
    }
}

fn requested_release<'a>(
    releases: &'a [Release],
    version: Option<&str>,
    channel: ReleaseChannel,
    policy: &crate::VersionPolicy,
) -> Result<&'a Release> {
    if let Some(version) = version {
        releases
            .iter()
            .find(|release| release.version == version)
            .ok_or(FontFerryError::NoEligibleVersion)
    } else {
        select_latest(releases, channel, policy).ok_or(FontFerryError::NoEligibleVersion)
    }
}
