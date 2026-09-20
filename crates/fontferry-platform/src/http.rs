use std::{net::IpAddr, sync::Arc, time::Duration};

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
    downloads: Client,
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
                        "network check failed; cached metadata retained for provenance only; freshness is unconfirmed"
                    );
                    Err(FontFerryError::Network(format!(
                        "在线检查失败；缓存日期 {}，无法确认是否最新",
                        cached.checked_at
                    )))
                } else {
                    Err(network_error)
                }
            }
        }
    }
}

impl HttpClient {
    pub fn new() -> Result<Self> {
        let inner = Client::builder()
            .redirect(redirect_policy())
            .dns_resolver(Arc::new(PublicResolver))
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(network_error)?;
        let downloads = Client::builder()
            .redirect(redirect_policy())
            .dns_resolver(Arc::new(PublicResolver))
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(90))
            .build()
            .map_err(network_error)?;
        Ok(Self { inner, downloads })
    }

    pub fn raw(&self) -> &Client {
        &self.inner
    }

    pub(crate) fn download_client(&self) -> &Client {
        &self.downloads
    }
}

fn redirect_policy() -> Policy {
    Policy::custom(|attempt: Attempt<'_>| {
        if attempt.previous().len() >= 5 {
            return attempt.error("too many redirects");
        }
        if validate_public_https(attempt.url()).is_err() {
            return attempt.error("redirect target is not an allowed public HTTPS URL");
        }
        attempt.follow()
    })
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
        let mut releases = Vec::new();
        for page in 1..=100 {
            let url = format!("{GITHUB_API}/repos/{repository}/releases?per_page=100&page={page}");
            let response = self
                .inner
                .get(url)
                .header(USER_AGENT, "FontFerry/0.2")
                .header(ACCEPT, "application/vnd.github+json")
                .send()
                .await
                .map_err(network_error)?
                .error_for_status()
                .map_err(network_error)?;
            let batch: Vec<GitHubRelease> = read_json(response).await?;
            let done = batch.len() < 100;
            releases.extend(batch);
            if done {
                break;
            }
            if page == 100 {
                return Err(FontFerryError::Network("release pagination exceeds safety limit; refusing incomplete version selection".into()));
            }
        }
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
            .map_err(network_error)?
            .error_for_status()
            .map_err(network_error)?;
        let body: FontAwesomeResponse = read_json(response).await?;
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
            .map_err(network_error)?
            .error_for_status()
            .map_err(network_error)?;
        let value: Value = read_json(response).await?;
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
            .map_err(network_error)?
            .error_for_status()
            .map_err(network_error)?;
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
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FontFerryError::DownloadRejected(
            "URL credentials are not allowed".into(),
        ));
    }
    match url.host() {
        Some(url::Host::Ipv4(ip)) if !public_address(IpAddr::V4(ip)) => {
            return Err(FontFerryError::DownloadRejected(
                "non-public address".into(),
            ));
        }
        Some(url::Host::Ipv6(ip)) if !public_address(IpAddr::V6(ip)) => {
            return Err(FontFerryError::DownloadRejected(
                "non-public address".into(),
            ));
        }
        Some(url::Host::Domain(host))
            if host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
                || host.trim_end_matches('.').ends_with(".local") =>
        {
            return Err(FontFerryError::DownloadRejected("local hostname".into()));
        }
        None => return Err(FontFerryError::DownloadRejected("URL has no host".into())),
        _ => {}
    }
    Ok(())
}

fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 240
                // Conservatively exclude IETF protocol assignments, including anycast exceptions.
                || ip.octets()[..3] == [192, 0, 0]
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                || (ip.octets()[0] == 198 && (18..=19).contains(&ip.octets()[1])))
        }
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or_else(
            || {
                !(ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.is_multicast()
                    || ip.segments()[0] & 0xe000 != 0x2000
                    || (ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8))
            },
            |ip| public_address(IpAddr::V4(ip)),
        ),
    }
}

#[derive(Debug)]
struct PublicResolver;
impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty() || addresses.iter().any(|a| !public_address(a.ip())) {
                return Err(std::io::Error::other("DNS returned a non-public address").into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

fn network_error(error: reqwest::Error) -> FontFerryError {
    FontFerryError::Network(error.without_url().to_string())
}

pub(crate) async fn bounded_body(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    use futures_util::StreamExt;
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(FontFerryError::DownloadRejected(
            "metadata exceeds size limit".into(),
        ));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(network_error)?;
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(FontFerryError::DownloadRejected(
                "metadata exceeds size limit".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn read_json<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    serde_json::from_slice(&bounded_body(response, 5 * 1024 * 1024).await?)
        .map_err(|_| FontFerryError::Network("invalid remote JSON".into()))
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_assignment_range_is_rejected_for_literal_and_resolved_addresses()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for last in 0..=255 {
            let ip = std::net::Ipv4Addr::new(192, 0, 0, last);
            assert!(!public_address(IpAddr::V4(ip))); // Predicate used on DNS answers.
            assert!(!public_address(IpAddr::V6(ip.to_ipv6_mapped())));
            assert!(validate_public_https(&Url::parse(&format!("https://{ip}"))?).is_err());
        }
        assert!(validate_public_https(&Url::parse("https://1.1.1.1")?).is_ok());
        Ok(())
    }

    #[test]
    fn rejects_local_ipv4_ipv6_mapped_addresses_and_credentials()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for url in [
            "http://example.com",
            "https://127.0.0.1",
            "https://10.0.0.1",
            "https://169.254.0.1",
            "https://192.0.0.1",
            "https://192.0.0.9",
            "https://[::ffff:192.0.0.1]",
            "https://[::1]",
            "https://[::ffff:127.0.0.1]",
            "https://[fc00::1]",
            "https://[fe80::1]",
            "https://localhost.",
            "https://user:secret@example.com",
        ] {
            assert!(validate_public_https(&Url::parse(url)?).is_err(), "{url}");
        }
        assert!(validate_public_https(&Url::parse("https://github.com")?).is_ok());
        Ok(())
    }
}
