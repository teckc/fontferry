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
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub checked_at: Option<OffsetDateTime>,
    #[serde(default)]
    pub from_cache: bool,
}

impl UpdateStatus {
    pub fn refresh_local(&mut self, current: Option<String>, fingerprint: bool) {
        self.current_version = current;
        self.from_cache = true;
        self.update_available = match (&self.current_version, &self.available_version) {
            (Some(current), Some(available)) => {
                if fingerprint {
                    current != available
                } else {
                    is_update_available(current, available)
                }
            }
            (None, Some(_)) => true,
            _ => false,
        };
    }
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
    operation_lock_path: PathBuf,
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
            operation_lock_path: staging_root.join("operations.lock"),
            staging_root,
        }
    }

    #[must_use]
    pub fn with_operation_lock(mut self, path: PathBuf) -> Self {
        self.operation_lock_path = path;
        self
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
                (Some(current), Some(available)) => match font.version_provider {
                    crate::VersionProvider::HttpFingerprint { .. } => current != available,
                    _ => is_update_available(current, available),
                },
                (None, Some(_)) => true,
                _ => false,
            },
            delivery_policy: font.delivery_policy,
            checked_at: Some(OffsetDateTime::now_utc()),
            from_cache: false,
        })
    }

    pub async fn install(&self, request: InstallRequest) -> Result<InstalledFont> {
        let _guard = self.operation_lock.lock().await;
        let _process_guard = self.acquire_operation_lock()?;
        self.installer.recover(self.state.as_ref()).await?;
        let font_id = request.font_id.clone();
        let result = self.install_inner(request).await;
        match result {
            Ok(value) => {
                self.report_activity(
                    Some(font_id),
                    ActivityLevel::Info,
                    "install completed".into(),
                )
                .await;
                Ok(value)
            }
            Err(error) => {
                let message = match self.installer.recover(self.state.as_ref()).await {
                    Ok(()) => error.to_string(),
                    Err(recovery) => format!(
                        "{error}; recovery incomplete: {recovery}; retain journal and backups, then restart"
                    ),
                };
                self.report_activity(Some(font_id.clone()), ActivityLevel::Error, message.clone())
                    .await;
                Err(FontFerryError::State(message))
            }
        }
    }

    async fn install_inner(&self, mut request: InstallRequest) -> Result<InstalledFont> {
        request.variant_ids.sort();
        request.variant_ids.dedup();
        let font = self.font(&request.font_id)?.clone();
        if (!font.variants.is_empty() && request.variant_ids.is_empty())
            || request
                .variant_ids
                .iter()
                .any(|id| !font.variants.iter().any(|v| v.id == *id))
        {
            return Err(FontFerryError::DownloadRejected(
                "请选择有效的字体包；空选择不会安装默认项".into(),
            ));
        }
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
        for warning in &outcome.warnings {
            self.report_activity(
                Some(font.id.clone()),
                ActivityLevel::Warning,
                warning.clone(),
            )
            .await;
        }
        if outcome.restart_recommended {
            self.report_activity(
                Some(font.id.clone()),
                ActivityLevel::Warning,
                "请重新启动使用字体的应用，使新字体生效".into(),
            )
            .await;
        }
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
        self.installer.finish(self.state.as_ref()).await?;
        self.report_activity(
            Some(font.id),
            ActivityLevel::Info,
            format!("Installed {}", release.version),
        )
        .await;
        Ok(installed)
    }

    /// Scheduled updates read the current variant selection and version under the operation lock.
    pub async fn update_installed(&self, font_id: &str) -> Result<Option<InstalledFont>> {
        let _guard = self.operation_lock.lock().await;
        let _process_guard = self.acquire_operation_lock()?;
        self.installer.recover(self.state.as_ref()).await?;
        let Some(current) = self.state.get_installed(font_id).await? else {
            return Ok(None);
        };
        let status = match self.check_font(font_id).await {
            Ok(status) => status,
            Err(error) => {
                self.report_activity(
                    Some(font_id.into()),
                    ActivityLevel::Error,
                    error.to_string(),
                )
                .await;
                return Err(error);
            }
        };
        if !status.update_available {
            return Ok(None);
        }
        let variants = if current.variant_ids.is_empty() {
            self.font(font_id)?
                .variants
                .iter()
                .filter(|v| v.default)
                .map(|v| v.id.clone())
                .collect()
        } else {
            current.variant_ids
        };
        match self
            .install_inner(InstallRequest {
                font_id: font_id.into(),
                version: status.available_version,
                variant_ids: variants,
                accept_license: false,
            })
            .await
        {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                let recovery = self.installer.recover(self.state.as_ref()).await;
                let message = match recovery {
                    Ok(()) => error.to_string(),
                    Err(recovery) => format!("{error}; recovery incomplete: {recovery}"),
                };
                self.report_activity(Some(font_id.into()), ActivityLevel::Error, message.clone())
                    .await;
                Err(FontFerryError::State(message))
            }
        }
    }

    pub async fn uninstall(&self, font_id: &str) -> Result<()> {
        let _guard = self.operation_lock.lock().await;
        let _process_guard = self.acquire_operation_lock()?;
        self.installer.recover(self.state.as_ref()).await?;
        let operation_font_id = font_id.to_owned();
        let result = self.uninstall_inner(font_id).await;
        match result {
            Ok(value) => {
                self.report_activity(
                    Some(operation_font_id),
                    ActivityLevel::Info,
                    "uninstall completed".into(),
                )
                .await;
                Ok(value)
            }
            Err(error) => {
                let message = match self.installer.recover(self.state.as_ref()).await {
                    Ok(()) => error.to_string(),
                    Err(recovery) => format!(
                        "{error}; recovery incomplete: {recovery}; retain journal and backups, then restart"
                    ),
                };
                self.report_activity(
                    Some(operation_font_id),
                    ActivityLevel::Error,
                    message.clone(),
                )
                .await;
                Err(FontFerryError::State(message))
            }
        }
    }

    async fn uninstall_inner(&self, font_id: &str) -> Result<()> {
        if let Some(installed) = self.state.get_installed(font_id).await? {
            self.installer.uninstall(&installed).await?;
            self.state.remove_installed(font_id).await?;
            self.installer.finish(self.state.as_ref()).await?;
        }
        Ok(())
    }

    pub async fn rollback(&self, font_id: &str) -> Result<InstalledFont> {
        let _guard = self.operation_lock.lock().await;
        let _process_guard = self.acquire_operation_lock()?;
        self.installer.recover(self.state.as_ref()).await?;
        let operation_font_id = font_id.to_owned();
        let result = self.rollback_inner(font_id).await;
        match result {
            Ok(value) => {
                self.report_activity(
                    Some(operation_font_id),
                    ActivityLevel::Info,
                    "rollback completed".into(),
                )
                .await;
                Ok(value)
            }
            Err(error) => {
                let message = match self.installer.recover(self.state.as_ref()).await {
                    Ok(()) => error.to_string(),
                    Err(recovery) => format!(
                        "{error}; recovery incomplete: {recovery}; retain journal and backups, then restart"
                    ),
                };
                self.report_activity(
                    Some(operation_font_id),
                    ActivityLevel::Error,
                    message.clone(),
                )
                .await;
                Err(FontFerryError::State(message))
            }
        }
    }

    async fn rollback_inner(&self, font_id: &str) -> Result<InstalledFont> {
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
            previous: outcome.previous_snapshot,
            manual_version: None,
        };
        self.state.save_installed(&restored).await?;
        self.installer.finish(self.state.as_ref()).await?;
        Ok(restored)
    }

    /// Nonblocking OS lock, shared by GUI, CLI and recovery. Never delete the lock file.
    pub fn acquire_operation_lock(&self) -> Result<std::fs::File> {
        crate::operation::acquire(&self.operation_lock_path)
    }

    pub async fn recover(&self) -> Result<()> {
        let _guard = self.operation_lock.lock().await;
        let _process_guard = self.acquire_operation_lock()?;
        self.installer.recover(self.state.as_ref()).await
    }

    fn font(&self, font_id: &str) -> Result<&FontDefinition> {
        self.catalog
            .get(font_id)
            .ok_or_else(|| FontFerryError::UnknownFont(font_id.to_owned()))
    }

    pub async fn record_failure(&self, font_id: &str, message: &str) -> Result<()> {
        self.activity(
            Some(font_id.to_owned()),
            ActivityLevel::Error,
            message.to_owned(),
        )
        .await
    }

    // Activity persistence is diagnostic, never the commit authority for font operations.
    async fn report_activity(
        &self,
        font_id: Option<String>,
        level: ActivityLevel,
        message: String,
    ) {
        if let Err(error) = self.activity(font_id.clone(), level, message).await {
            tracing::error!(font_id = ?font_id, error = %error, "activity persistence failed; operation result is unchanged");
        }
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
            .find(|release| {
                release.version == version && crate::release_eligible(release, channel, policy)
            })
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
        async fn recover(&self, _state: &dyn StateRepository) -> Result<()> {
            self.uninstall_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

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
            Ok(InstallOutcome {
                owned_files: vec![PathBuf::from("restored.ttf")],
                previous_snapshot: None,
                restart_recommended: false,
                warnings: Vec::new(),
            })
        }
    }

    #[derive(Default)]
    struct FailingState {
        observed: Option<ObservedFont>,
        installed: Mutex<Option<InstalledFont>>,
        fail_commit: bool,
        fail_activity: bool,
        activities: Mutex<Vec<Activity>>,
    }

    #[async_trait]
    impl StateRepository for FailingState {
        async fn list_installed(&self) -> Result<Vec<InstalledFont>> {
            Ok(Vec::new())
        }

        async fn get_installed(&self, _font_id: &str) -> Result<Option<InstalledFont>> {
            Ok(self.installed.lock().await.clone())
        }

        async fn save_installed(&self, installed: &InstalledFont) -> Result<()> {
            if self.fail_commit {
                return Err(FontFerryError::State("injected commit failure".into()));
            }
            *self.installed.lock().await = Some(installed.clone());
            Ok(())
        }

        async fn remove_installed(&self, _font_id: &str) -> Result<()> {
            *self.installed.lock().await = None;
            Ok(())
        }

        async fn is_license_accepted(&self, _font_id: &str, _revision: &str) -> Result<bool> {
            Ok(true)
        }

        async fn accept_license(&self, _font_id: &str, _revision: &str) -> Result<()> {
            Ok(())
        }

        async fn append_activity(&self, activity: &Activity) -> Result<()> {
            if self.fail_activity {
                return Err(FontFerryError::State("injected activity failure".into()));
            }
            self.activities.lock().await.push(activity.clone());
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
            Arc::new(FailingState {
                observed: None,
                fail_commit: true,
                ..Default::default()
            }),
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
        assert_eq!(uninstall_count.load(Ordering::SeqCst), 2);
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
                ..Default::default()
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
    #[tokio::test]
    async fn activity_failure_does_not_change_committed_operation_results()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let staging = tempfile::tempdir()?;
        let state = Arc::new(FailingState {
            fail_activity: true,
            ..Default::default()
        });
        let engine = FontEngine::new(
            vec![font(DeliveryPolicy::AutoInstall)?],
            Arc::new(StaticReleases),
            Arc::new(EmptyArtifact),
            Arc::new(PreparedArtifact),
            Arc::new(TrackingInstaller {
                uninstall_count: Arc::new(AtomicUsize::new(0)),
            }),
            state.clone(),
            staging.path().into(),
        );
        let installed = engine
            .install(InstallRequest {
                font_id: "test-font".into(),
                version: None,
                variant_ids: vec![],
                accept_license: false,
            })
            .await?;
        assert_eq!(
            state.get_installed("test-font").await?,
            Some(installed.clone())
        );
        let mut with_backup = installed;
        with_backup.previous = Some(RollbackSnapshot {
            version: "1.0.0".into(),
            variant_ids: vec![],
            backup_directory: staging.path().join("snapshot"),
        });
        state.save_installed(&with_backup).await?;
        let restored = engine.rollback("test-font").await?;
        assert_eq!(restored.version, "1.0.0");
        assert_eq!(state.get_installed("test-font").await?, Some(restored));
        engine.uninstall("test-font").await?;
        assert!(state.get_installed("test-font").await?.is_none());
        Ok(())
    }

    struct OfflineReleases;
    #[async_trait]
    impl ReleaseSource for OfflineReleases {
        async fn releases(&self, _: &FontDefinition, _: &VersionProvider) -> Result<Vec<Release>> {
            Err(FontFerryError::Network("injected offline check".into()))
        }
    }

    #[tokio::test]
    async fn scheduled_recheck_failure_is_recorded_with_font_and_reason()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let staging = tempfile::tempdir()?;
        let state = Arc::new(FailingState::default());
        state
            .save_installed(&InstalledFont {
                font_id: "test-font".into(),
                version: "1.0".into(),
                variant_ids: vec![],
                owned_files: vec![],
                previous: None,
                manual_version: None,
                installed_at: datetime!(2026-01-01 0:00 UTC),
            })
            .await?;
        let engine = FontEngine::new(
            vec![font(DeliveryPolicy::AutoInstall)?],
            Arc::new(OfflineReleases),
            Arc::new(EmptyArtifact),
            Arc::new(PreparedArtifact),
            Arc::new(TrackingInstaller {
                uninstall_count: Arc::new(AtomicUsize::new(0)),
            }),
            state.clone(),
            staging.path().into(),
        );
        assert!(engine.update_installed("test-font").await.is_err());
        let activities = state.activities.lock().await;
        assert!(
            activities
                .iter()
                .any(|a| a.font_id.as_deref() == Some("test-font")
                    && a.message.contains("injected offline check"))
        );
        Ok(())
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use time::macros::{date, datetime};
    #[test]
    fn explicit_versions_cannot_bypass_channel_or_entitlements() {
        let release = Release {
            version: "2.0.0".into(),
            published_at: datetime!(2026-06-01 0:00 UTC),
            prerelease: false,
            assets: Vec::new(),
        };
        for policy in [
            crate::VersionPolicy {
                major: Some(1),
                ..Default::default()
            },
            crate::VersionPolicy {
                maximum_version: Some("1.9".into()),
                ..Default::default()
            },
            crate::VersionPolicy {
                updates_through: Some(date!(2026 - 01 - 01)),
                ..Default::default()
            },
        ] {
            assert!(
                requested_release(
                    std::slice::from_ref(&release),
                    Some("2.0.0"),
                    ReleaseChannel::Stable,
                    &policy
                )
                .is_err()
            );
        }
        let mut preview = release.clone();
        preview.prerelease = true;
        assert!(
            requested_release(
                &[preview],
                Some("2.0.0"),
                ReleaseChannel::Stable,
                &Default::default()
            )
            .is_err()
        );
        assert!(
            requested_release(
                &[release],
                Some("2.0.0"),
                ReleaseChannel::Stable,
                &Default::default()
            )
            .is_ok()
        );
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;
    #[test]
    fn local_mutations_rederive_cached_dashboard_without_network() {
        let mut status = UpdateStatus {
            font_id: "font".into(),
            current_version: Some("1.9".into()),
            available_version: Some("1.10".into()),
            update_available: true,
            delivery_policy: DeliveryPolicy::AutoInstall,
            checked_at: Some(OffsetDateTime::UNIX_EPOCH),
            from_cache: false,
        };
        status.refresh_local(Some("1.10".into()), false);
        assert!(!status.update_available);
        status.refresh_local(Some("1.9".into()), false);
        assert!(status.update_available);
        status.refresh_local(None, false);
        assert!(status.current_version.is_none());
        assert_eq!(status.checked_at, Some(OffsetDateTime::UNIX_EPOCH));
        assert!(status.from_cache);
    }
    #[test]
    fn fingerprints_detect_change_in_either_lexical_direction() {
        let mut status = UpdateStatus {
            font_id: "font".into(),
            current_version: Some("z".into()),
            available_version: Some("a".into()),
            update_available: false,
            delivery_policy: DeliveryPolicy::NotifyOnly,
            checked_at: None,
            from_cache: false,
        };
        status.refresh_local(Some("z".into()), true);
        assert!(status.update_available);
        status.refresh_local(Some("a".into()), true);
        assert!(!status.update_available);
    }
}
