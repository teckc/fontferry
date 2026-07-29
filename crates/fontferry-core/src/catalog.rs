use std::collections::BTreeSet;

use regex::Regex;
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};
use url::Url;

use crate::{FontFerryError, Result};

pub const CATALOG_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema_version: u32,
    pub revision: String,
    pub generated_at: OffsetDateTime,
    pub fonts: Vec<FontDefinition>,
}

impl Catalog {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CATALOG_SCHEMA_VERSION {
            return Err(FontFerryError::InvalidCatalog(format!(
                "unsupported schema version {}",
                self.schema_version
            )));
        }
        let mut ids = BTreeSet::new();
        for font in &self.fonts {
            font.validate()?;
            if !ids.insert(&font.id) {
                return Err(FontFerryError::InvalidCatalog(format!(
                    "duplicate font id '{}'",
                    font.id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FontDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub homepage: Url,
    pub license: LicensePolicy,
    pub version_provider: VersionProvider,
    pub artifact_provider: Option<ArtifactProvider>,
    pub delivery_policy: DeliveryPolicy,
    #[serde(default)]
    pub version_policy: VersionPolicy,
    #[serde(default)]
    pub variants: Vec<Variant>,
    #[serde(default)]
    pub platforms: BTreeSet<Platform>,
}

impl FontDefinition {
    pub fn validate(&self) -> Result<()> {
        let valid_id = self.id.len() >= 2
            && self.id.len() <= 64
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !self.id.starts_with('-')
            && !self.id.ends_with('-');
        if !valid_id {
            return Err(FontFerryError::InvalidCatalog(format!(
                "invalid id '{}'",
                self.id
            )));
        }
        if self.homepage.scheme() != "https" {
            return Err(FontFerryError::InvalidCatalog(format!(
                "homepage for '{}' must use HTTPS",
                self.id
            )));
        }
        if self.delivery_policy == DeliveryPolicy::AutoInstall
            && self.artifact_provider.is_none()
        {
            return Err(FontFerryError::InvalidCatalog(format!(
                "auto-install font '{}' has no artifact provider",
                self.id
            )));
        }
        let mut variant_ids = BTreeSet::new();
        for variant in &self.variants {
            if !variant_ids.insert(&variant.id) {
                return Err(FontFerryError::InvalidCatalog(format!(
                    "font '{}' has duplicate variant '{}'",
                    self.id, variant.id
                )));
            }
            Regex::new(&variant.asset_pattern).map_err(|error| {
                FontFerryError::InvalidCatalog(format!(
                    "font '{}' variant '{}' has invalid pattern: {error}",
                    self.id, variant.id
                ))
            })?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LicensePolicy {
    pub name: String,
    pub url: Url,
    pub spdx: Option<String>,
    pub revision: String,
    pub requires_acceptance: bool,
    pub redistribution_allowed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum VersionProvider {
    GitHubRelease {
        repository: String,
        channel: ReleaseChannel,
    },
    JsonEndpoint {
        url: Url,
        version_pointer: String,
        date_pointer: Option<String>,
    },
    FontAwesomeReleaseApi {
        major: Option<u64>,
    },
    HttpFingerprint {
        url: Url,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ArtifactProvider {
    GitHubAsset { repository: String },
    DirectUrl { url: Url },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryPolicy {
    AutoInstall,
    NotifyOnly,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseChannel {
    Stable,
    Prerelease,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VersionPolicy {
    pub major: Option<u64>,
    pub maximum_version: Option<String>,
    pub updates_through: Option<Date>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub id: String,
    pub name: String,
    pub description: String,
    pub asset_pattern: String,
    pub default: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
}

