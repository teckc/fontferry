use std::{net::IpAddr, str::FromStr, sync::Arc, time::Duration};

use async_trait::async_trait;
use fontferry_core::{
    FontDefinition, FontFerryError, Release, ReleaseAsset, ReleaseSource, Result, VersionProvider,
};
use reqwest::{
    Client,
    header::{ACCEPT, ETAG, LAST_MODIFIED, USER_AGENT},
    redirect::{Attempt, Policy},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use url::Url;

use crate::SqliteState;

const GITHUB_API: &str = "https://api.github.com";
const FONT_AWESOME_RELEASES: &str = "https://api.fontawesome.com/releases";

#[derive(Clone, Debug)]
pub struct HttpClient {
    inner: Client,
}

#[derive(Clone, Debug)]
pub struct CachedReleaseSource {
    upstream: HttpClient,
    state: Arc<SqliteState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseCacheEntry {
    #[serde(with = "time::serde::rfc3339")]
    checked_at: OffsetDateTime,
    releases: Vec<Release>,
}

impl CachedReleaseSource {
    #[must_use]
    pub fn new(upstream: HttpClient, state: Arc<SqliteState>) -> Self {
        Self { upstream, state }
    }
}

#[async_trait]
impl ReleaseSource for CachedReleaseSource {
    async fn releases(
        &self,
        font: &FontDefinition,
        provider: &VersionProvider,
    ) -> Result<Vec<Release>> {
        let key = format!("release-cache:{}", font.id);
        match self.upstream.releases(font, provider).await {
            Ok(releases) => {
                self.state.set_setting(
                    &key,
                    &ReleaseCacheEntry {
                        checked_at: OffsetDateTime::now_utc(),
                        releases: releases.clone(),
                    },
                )?;
                Ok(releases)
            }
            Err(network_error) => {
                if let Some(cached) = self.state.get_setting::<ReleaseCacheEntry>(&key)? {
                    tracing::warn!(
                        font_id = %font.id,
                        checked_at = %cached.checked_at,
                        error = %network_error,
                        "using cached release metadata"
                    );
                    Ok(cached.releases)
                } else {
                    Err(network_error)
                }
            }
        }
    }
}

impl HttpClient {
    pub fn new() -> Result<Self> {
        let policy = Policy::custom(|attempt: Attempt<'_>| {
            if attempt.previous().len() >= 5 {
                return attempt.error("too many redirects");
            }
            if validate_public_https(attempt.url()).is_err() {
                return attempt.error("redirect target is not an allowed public HTTPS URL");
            }
            attempt.follow()
        });
        let inner = Client::builder()
            .redirect(policy)
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| FontFerryError::Network(error.to_string()))?;
        Ok(Self { inner })
    }

    pub fn raw(&self) -> &Client {
        &self.inner
    }
}

#[async_trait]
impl ReleaseSource for HttpClient {
    async fn releases(
        &self,
        _font: &FontDefinition,
        provider: &VersionProvider,
    ) -> Result<Vec<Release>> {
        match provider {
            VersionProvider::GitHubRelease {
                repository,
                channel: _,
            } => self.github_releases(repository).await,
            VersionProvider::JsonEndpoint {
                url,
                version_pointer,
                date_pointer,
            } => {
                self.json_release(url, version_pointer, date_pointer.as_deref())
                    .await
            }
            VersionProvider::FontAwesomeReleaseApi { major } => {
                self.font_awesome_releases(*major).await
            }
            VersionProvider::HttpFingerprint { url } => self.fingerprint_release(url).await,
        }
    }
}

impl HttpClient {
    async fn github_releases(&self, repository: &str) -> Result<Vec<Release>> {
        validate_repository(repository)?;
        let url = Url::parse(&format!(
            "{GITHUB_API}/repos/{repository}/releases?per_page=20"
        ))
        .map_err(|error| FontFerryError::Network(error.to_string()))?;
        let response = self
            .inner
            .get(url)
            .header(USER_AGENT, "FontFerry/0.2")
            .header(ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| FontFerryError::Network(error.to_string()))?
            .error_for_status()
            .map_err(|error| FontFerryError::Network(error.to_string()))?;
        let releases: Vec<GitHubRelease> = response
            .json()
            .await
            .map_err(|error| FontFerryError::Network(error.to_string()))?;
        Ok(releases
            .into_iter()
            .filter(|release| !release.draft)
            .map(|release| Release {
                version: release.tag_name,
                published_at: release.published_at,
                prerelease: release.prerelease,
                assets: release
                    .assets
                    .into_iter()
                    .map(|asset| ReleaseAsset {
                        name: asset.name,
                        url: asset.browser_download_url,
                        size: asset.size,
                        digest: asset.digest,
                    })
                    .collect(),
            })
            .collect())
    }

    async fn font_awesome_releases(&self, major: Option<u64>) -> Result<Vec<Release>> {
        let response = self
            .inner
            .get(FONT_AWESOME_RELEASES)
            .header(USER_AGENT, "FontFerry/0.2")
            .send()
            .await
            .map_err(|error| FontFerryError::Network(error.to_string()))?
            .error_for_status()
            .map_err(|error| FontFerryError::Network(error.to_string()))?;
        let body: FontAwesomeResponse = response
            .json()
            .await
            .map_err(|error| FontFerryError::Network(error.to_string()))?;
        Ok(body
            .releases
            .into_iter()
            .filter(|release| {
                major.is_none_or(|expected| {
                    semver::Version::parse(&release.version)
                        .is_ok_and(|version| version.major == expected)
                })
            })
            .filter_map(|release| {
                let published_at = OffsetDateTime::parse(
                    &format!("{}T00:00:00Z", release.date),
                    &time::format_description::well_known::Rfc3339,
                )
                .ok()?;
                Some(Release {
                    version: release.version,
                    published_at,
                    prerelease: false,
                    assets: Vec::new(),
                })
            })
            .collect())
    }

    async fn json_release(
        &self,
        url: &Url,
        version_pointer: &str,
        date_pointer: Option<&str>,
    ) -> Result<Vec<Release>> {
        validate_public_https(url)?;
        let response = self
            .inner
            .get(url.clone())
            .header(USER_AGENT, "FontFerry/0.2")
            .send()
            .await
            .map_err(|error| FontFerryError::Network(error.to_string()))?
            .error_for_status()
            .map_err(|error| FontFerryError::Network(error.to_string()))?;
        let value: Value = response
            .json()
            .await
            .map_err(|error| FontFerryError::Network(error.to_string()))?;
        let version = value
            .pointer(version_pointer)
            .and_then(Value::as_str)
            .ok_or_else(|| FontFerryError::Network("version JSON pointer is missing".into()))?;
        let published_at = date_pointer
            .and_then(|pointer| value.pointer(pointer))
            .and_then(Value::as_str)
            .and_then(|value| {
                OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
            })
            .unwrap_or_else(OffsetDateTime::now_utc);
        Ok(vec![Release {
            version: version.to_owned(),
            published_at,
            prerelease: false,
            assets: Vec::new(),
        }])
    }

    async fn fingerprint_release(&self, url: &Url) -> Result<Vec<Release>> {
        validate_public_https(url)?;
        let response = self
            .inner
            .head(url.clone())
            .header(USER_AGENT, "FontFerry/0.2")
            .send()
            .await
            .map_err(|error| FontFerryError::Network(error.to_string()))?
            .error_for_status()
            .map_err(|error| FontFerryError::Network(error.to_string()))?;
        let fingerprint = response
            .headers()
            .get(ETAG)
            .or_else(|| response.headers().get(LAST_MODIFIED))
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                FontFerryError::Network("server provides no ETag or Last-Modified".into())
            })?;
        Ok(vec![Release {
            version: fingerprint.trim_matches('"').to_owned(),
            published_at: OffsetDateTime::now_utc(),
            prerelease: false,
            assets: Vec::new(),
        }])
    }
}

pub fn validate_public_https(url: &Url) -> Result<()> {
    if url.scheme() != "https" {
        return Err(FontFerryError::DownloadRejected(
            "only HTTPS URLs are allowed".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| FontFerryError::DownloadRejected("URL has no host".into()))?;
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") {
        return Err(FontFerryError::DownloadRejected(
            "local network hosts are not allowed".into(),
        ));
    }
    if let Ok(address) = IpAddr::from_str(host)
        && (address.is_loopback() || address.is_unspecified() || is_private_or_link_local(address))
    {
        return Err(FontFerryError::DownloadRejected(
            "private network addresses are not allowed".into(),
        ));
    }
    Ok(())
}

fn is_private_or_link_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_private() || address.is_link_local(),
        IpAddr::V6(address) => address.is_unique_local() || address.is_unicast_link_local(),
    }
}

fn validate_repository(repository: &str) -> Result<()> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(FontFerryError::InvalidCatalog(format!(
            "invalid GitHub repository '{repository}'"
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(with = "time::serde::rfc3339")]
    published_at: OffsetDateTime,
    prerelease: bool,
    draft: bool,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FontAwesomeResponse {
    releases: Vec<FontAwesomeRelease>,
}

#[derive(Debug, Deserialize)]
struct FontAwesomeRelease {
    version: String,
    date: String,
}
