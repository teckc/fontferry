use std::{collections::HashMap, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use time::OffsetDateTime;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    Activity, ActivityLevel, ArtifactSource, DeliveryPolicy, FontDefinition, FontFerryError,
    FontInstaller, FontPreparer, InstalledFont, Release, ReleaseChannel, ReleaseSource, Result,
    StateRepository, is_update_available, select_latest,
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
        let releases = self.releases.releases(font, &font.version_provider).await?;
        let latest = select_latest(
            &releases,
            release_channel(&font.version_provider),
            &font.version_policy,
        );
        let installed = self.state.get_installed(font_id).await?;
        let observed = self.state.get_observed(font_id).await?;
        let current = installed
            .as_ref()
            .and_then(|item| item.manual_version.as_ref().or(Some(&item.version)))
            .or_else(|| {
                observed.as_ref().and_then(|item| {
                    item.manual_version
                        .as_ref()
                        .or(item.detected_version.as_ref())
                })
            });
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
        let prepared = self.preparer.prepare(&downloaded, staging.path()).await?;
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
        if let Err(state_error) = self.state.save_installed(&installed).await {
            let cleanup_result = if let (Some(_previous), Some(snapshot)) =
                (previous.as_ref(), installed.previous.as_ref())
            {
                self.installer
                    .restore(&font, snapshot, &installed)
                    .await
                    .map(|_| ())
            } else {
                self.installer.uninstall(&installed).await
            };
            return match cleanup_result {
                Ok(_) => Err(state_error),
                Err(cleanup_error) => Err(FontFerryError::State(format!(
                    "{state_error}; compensation also failed: {cleanup_error}"
                ))),
            };
        }
        let _activity_result = self
            .activity(
                Some(font.id),
                ActivityLevel::Info,
                format!("Installed {}", release.version),
            )
            .await;
        Ok(installed)
    }

    pub async fn uninstall(&self, font_id: &str) -> Result<()> {
        let _guard = self.operation_lock.lock().await;
        if let Some(installed) = self.state.get_installed(font_id).await? {
            self.state.remove_installed(font_id).await?;
            if let Err(platform_error) = self.installer.uninstall(&installed).await {
                self.state.save_installed(&installed).await?;
                return Err(platform_error);
            }
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

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use time::macros::datetime;
    use url::Url;

    use super::*;
    use crate::{
        ArtifactProvider, ArtifactSource, FontPreparer, InstallOutcome, LicensePolicy,
        ObservedFont, Platform, PreparedFont, ReleaseAsset, RollbackSnapshot, VersionPolicy,
        VersionProvider,
    };

    struct StaticReleases;

    #[async_trait]
    impl ReleaseSource for StaticReleases {
        async fn releases(
            &self,
            _font: &FontDefinition,
            _provider: &VersionProvider,
        ) -> Result<Vec<Release>> {
            Ok(vec![Release {
                version: "2.0.0".into(),
                published_at: datetime!(2026-01-01 0:00 UTC),
                prerelease: false,
                assets: vec![ReleaseAsset {
                    name: "font.zip".into(),
                    url: "https://example.com/font.zip".into(),
                    size: 10,
                    digest: None,
                }],
            }])
        }
    }

    struct EmptyArtifact;

    #[async_trait]
    impl ArtifactSource for EmptyArtifact {
        async fn download(
            &self,
            _font: &FontDefinition,
            _provider: &ArtifactProvider,
            _release: &Release,
            _variant_ids: &[String],
            _staging_directory: &Path,
        ) -> Result<Vec<PathBuf>> {
            Ok(Vec::new())
        }
    }

    struct PreparedArtifact;

    #[async_trait]
    impl FontPreparer for PreparedArtifact {
        async fn prepare(
            &self,
            _downloaded: &[PathBuf],
            staging_directory: &Path,
        ) -> Result<Vec<PreparedFont>> {
            Ok(vec![PreparedFont {
                path: staging_directory.join("font.ttf"),
                family: "Test".into(),
                style: "Regular".into(),
                postscript_name: Some("Test-Regular".into()),
                version: Some("2.0.0".into()),
                sha256: "00".repeat(32),
            }])
        }
    }

    struct TrackingInstaller {
        uninstall_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl FontInstaller for TrackingInstaller {
        async fn install(
            &self,
            _font: &FontDefinition,
            _version: &str,
            _prepared: &[PreparedFont],
            _previous: Option<&InstalledFont>,
        ) -> Result<InstallOutcome> {
            Ok(InstallOutcome {
                owned_files: vec![PathBuf::from("managed.ttf")],
                previous_snapshot: None,
                restart_recommended: false,
                warnings: Vec::new(),
            })
        }

        async fn uninstall(&self, _installed: &InstalledFont) -> Result<()> {
            self.uninstall_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn restore(
            &self,
            _font: &FontDefinition,
            _snapshot: &RollbackSnapshot,
            _current: &InstalledFont,
        ) -> Result<InstallOutcome> {
            Err(FontFerryError::NoRollbackSnapshot)
        }
    }

    struct FailingState {
        observed: Option<ObservedFont>,
    }

    #[async_trait]
    impl StateRepository for FailingState {
        async fn list_installed(&self) -> Result<Vec<InstalledFont>> {
            Ok(Vec::new())
        }

        async fn get_installed(&self, _font_id: &str) -> Result<Option<InstalledFont>> {
            Ok(None)
        }

        async fn save_installed(&self, _installed: &InstalledFont) -> Result<()> {
            Err(FontFerryError::State("injected commit failure".into()))
        }

        async fn remove_installed(&self, _font_id: &str) -> Result<()> {
            Ok(())
        }

        async fn is_license_accepted(&self, _font_id: &str, _revision: &str) -> Result<bool> {
            Ok(true)
        }

        async fn accept_license(&self, _font_id: &str, _revision: &str) -> Result<()> {
            Ok(())
        }

        async fn append_activity(&self, _activity: &Activity) -> Result<()> {
            Ok(())
        }

        async fn get_observed(&self, _font_id: &str) -> Result<Option<ObservedFont>> {
            Ok(self.observed.clone())
        }
    }

    fn font(
        delivery_policy: DeliveryPolicy,
    ) -> std::result::Result<FontDefinition, url::ParseError> {
        Ok(FontDefinition {
            id: "test-font".into(),
            name: "Test Font".into(),
            description: "Test".into(),
            homepage: Url::parse("https://example.com")?,
            license: LicensePolicy {
                name: "OFL".into(),
                url: Url::parse("https://example.com/license")?,
                spdx: Some("OFL-1.1".into()),
                revision: "1".into(),
                requires_acceptance: false,
                redistribution_allowed: true,
            },
            version_provider: VersionProvider::GitHubRelease {
                repository: "example/font".into(),
                channel: ReleaseChannel::Stable,
            },
            artifact_provider: (delivery_policy == DeliveryPolicy::AutoInstall).then(|| {
                ArtifactProvider::GitHubAsset {
                    repository: "example/font".into(),
                }
            }),
            delivery_policy,
            version_policy: VersionPolicy::default(),
            variants: Vec::new(),
            platforms: BTreeSet::from([Platform::Windows]),
        })
    }

    #[tokio::test]
    async fn compensates_platform_install_when_state_commit_fails()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let staging = tempfile::tempdir()?;
        let uninstall_count = Arc::new(AtomicUsize::new(0));
        let engine = FontEngine::new(
            vec![font(DeliveryPolicy::AutoInstall)?],
            Arc::new(StaticReleases),
            Arc::new(EmptyArtifact),
            Arc::new(PreparedArtifact),
            Arc::new(TrackingInstaller {
                uninstall_count: uninstall_count.clone(),
            }),
            Arc::new(FailingState { observed: None }),
            staging.path().to_path_buf(),
        );
        let result = engine
            .install(InstallRequest {
                font_id: "test-font".into(),
                version: None,
                variant_ids: Vec::new(),
                accept_license: false,
            })
            .await;
        assert!(matches!(result, Err(FontFerryError::State(_))));
        assert_eq!(uninstall_count.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn uses_observed_version_for_reminder_only_fonts()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let staging = tempfile::tempdir()?;
        let engine = FontEngine::new(
            vec![font(DeliveryPolicy::NotifyOnly)?],
            Arc::new(StaticReleases),
            Arc::new(EmptyArtifact),
            Arc::new(PreparedArtifact),
            Arc::new(TrackingInstaller {
                uninstall_count: Arc::new(AtomicUsize::new(0)),
            }),
            Arc::new(FailingState {
                observed: Some(ObservedFont {
                    font_id: "test-font".into(),
                    detected_version: Some("1.0.0".into()),
                    manual_version: None,
                    observed_files: vec![PathBuf::from("observed.ttf")],
                    scanned_at: datetime!(2026-01-01 0:00 UTC),
                }),
            }),
            staging.path().to_path_buf(),
        );
        let status = engine.check_font("test-font").await?;
        assert_eq!(status.current_version.as_deref(), Some("1.0.0"));
        assert_eq!(status.available_version.as_deref(), Some("2.0.0"));
        assert!(status.update_available);
        assert_eq!(status.delivery_policy, DeliveryPolicy::NotifyOnly);
        Ok(())
    }
}
