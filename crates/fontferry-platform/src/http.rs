use std::{net::IpAddr, num::NonZeroU16, sync::Arc, time::Duration};

use async_trait::async_trait;
use fontferry_core::{
    ArtifactProvider, FontDefinition, FontFerryError, Release, ReleaseAsset, ReleaseSource, Result,
    VersionProvider,
};
use regex::Regex;
use reqwest::{
    Client, ClientBuilder, Proxy,
    header::{ACCEPT, ETAG, LAST_MODIFIED, USER_AGENT},
    redirect::{Attempt, Policy},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use url::Url;

use crate::SqliteState;

const GITHUB_API: &str = "https://api.github.com";
const GITHUB_RELEASE_PAGE_SIZE: usize = 20;
const MAX_GITHUB_RELEASE_PAGES: usize = 500;
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
                if let FontFerryError::Network(message) = &network_error
                    && let Some(cached) = self.state.get_setting::<ReleaseCacheEntry>(&key)?
                {
                    tracing::warn!(
                        font_id = %font.id,
                        checked_at = %cached.checked_at,
                        "network check failed; cached metadata retained for provenance only; freshness is unconfirmed"
                    );
                    Err(FontFerryError::Network(format!(
                        "{message}；缓存日期 {}，无法确认是否最新",
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
        Self::with_github_proxy(None)
    }

    pub fn with_github_proxy(port: Option<NonZeroU16>) -> Result<Self> {
        let inner = client_builder(port)?
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(network_error)?;
        let downloads = client_builder(port)?
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

    pub async fn probe_github(&self) -> (Result<()>, Result<()>) {
        const RELEASE: &str =
            "https://api.github.com/repos/atelier-anchor/smiley-sans/releases/latest";
        const ASSET: &str = "https://github.com/atelier-anchor/smiley-sans/releases/download/v2.0.1/smiley-sans-v2.0.1.zip";
        async fn probe(request: reqwest::RequestBuilder) -> Result<()> {
            request
                .header(USER_AGENT, "FontFerry/0.2")
                .timeout(Duration::from_secs(15))
                .send()
                .await
                .map_err(network_error)?
                .error_for_status()
                .map_err(network_error)?;
            Ok(())
        }
        tokio::join!(
            probe(self.inner.get(RELEASE)),
            probe(
                self.downloads
                    .get(ASSET)
                    .header(reqwest::header::RANGE, "bytes=0-0")
            ),
        )
    }
}

pub fn local_proxy_url(port: NonZeroU16) -> Result<Url> {
    Url::parse(&format!("http://127.0.0.1:{port}"))
        .map_err(|error| FontFerryError::Network(error.to_string()))
}

fn client_builder(port: Option<NonZeroU16>) -> Result<ClientBuilder> {
    let builder = Client::builder()
        .redirect(redirect_policy())
        .dns_resolver(Arc::new(PublicResolver))
        .connect_timeout(Duration::from_secs(20));
    let Some(port) = port else {
        return Ok(builder);
    };
    let proxy_url = local_proxy_url(port)?;
    Ok(builder.proxy(Proxy::custom(move |url| {
        github_proxy_target(url).then(|| proxy_url.clone())
    })))
}

fn github_proxy_target(url: &Url) -> bool {
    // CONNECT gives the proxy responsibility for DNS. Limit that to GitHub's known hosts.
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "raw.githubusercontent.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
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
        font: &FontDefinition,
        provider: &VersionProvider,
    ) -> Result<Vec<Release>> {
        match provider {
            VersionProvider::GitHubRelease {
                repository,
                channel: _,
            } => {
                let mut releases = self.github_releases(repository).await?;
                if matches!(
                    font.artifact_provider,
                    Some(ArtifactProvider::GitHubAsset { .. })
                ) {
                    retain_font_releases(font, &mut releases)?;
                }
                Ok(releases)
            }
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

fn retain_font_releases(font: &FontDefinition, releases: &mut Vec<Release>) -> Result<()> {
    let patterns = font
        .variants
        .iter()
        .map(|variant| Regex::new(&variant.asset_pattern))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| FontFerryError::InvalidCatalog(error.to_string()))?;
    // Some projects publish build-only assets under tags in the stable release feed.
    // Keep ambiguous matches for candidate validation to reject explicitly.
    releases.retain(|release| {
        release
            .assets
            .iter()
            .any(|asset| patterns.iter().any(|pattern| pattern.is_match(&asset.name)))
    });
    Ok(())
}

impl HttpClient {
    async fn github_releases(&self, repository: &str) -> Result<Vec<Release>> {
        validate_repository(repository)?;
        let mut releases = Vec::new();
        for page in 1..=MAX_GITHUB_RELEASE_PAGES {
            let url = format!(
                "{GITHUB_API}/repos/{repository}/releases?per_page={GITHUB_RELEASE_PAGE_SIZE}&page={page}"
            );
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
            let done = batch.len() < GITHUB_RELEASE_PAGE_SIZE;
            releases.extend(batch);
            if done {
                break;
            }
            if page == MAX_GITHUB_RELEASE_PAGES {
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
                        id: Some(asset.id),
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

#[derive(Debug)]
struct NonPublicDnsAddress;

impl std::fmt::Display for NonPublicDnsAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "DNS 返回了非公网地址。若 GitHub 使用 Fake-IP，请在设置中填写本机 HTTP 代理端口",
        )
    }
}

impl std::error::Error for NonPublicDnsAddress {}

impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty() {
                return Err(std::io::Error::other("DNS returned no addresses").into());
            }
            if addresses.iter().any(|a| !public_address(a.ip())) {
                return Err(
                    Box::new(NonPublicDnsAddress) as Box<dyn std::error::Error + Send + Sync>
                );
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

pub(crate) fn non_public_dns_error(error: &reqwest::Error) -> Option<FontFerryError> {
    let mut cause = std::error::Error::source(error);
    while let Some(error) = cause {
        if let Some(rejection) = error.downcast_ref::<NonPublicDnsAddress>() {
            return Some(FontFerryError::Network(rejection.to_string()));
        }
        cause = error.source();
    }
    None
}

fn network_error(error: reqwest::Error) -> FontFerryError {
    if let Some(rejection) = non_public_dns_error(&error) {
        return rejection;
    }
    if error.is_connect() {
        return FontFerryError::Network("无法连接服务器，请检查网络或本机代理设置".into());
    }
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
    id: u64,
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
    fn excludes_maple_mono_build_assets_without_hiding_ambiguous_font_packages()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let catalog =
            crate::parse_catalog(include_bytes!("../../../catalog/builtin/catalog.json"))?;
        let font = catalog
            .fonts
            .iter()
            .find(|font| font.id == "maple-mono")
            .ok_or("missing Maple Mono")?;
        let release = |version: &str, names: &[&str]| Release {
            version: version.to_owned(),
            published_at: OffsetDateTime::UNIX_EPOCH,
            prerelease: false,
            assets: names
                .iter()
                .map(|name| ReleaseAsset {
                    id: None,
                    name: (*name).into(),
                    url: String::new(),
                    size: 1,
                    digest: None,
                })
                .collect(),
        };
        let mut releases = vec![
            release("cjk-base", &["cn-base-static.zip", "cn-base-variable.zip"]),
            release("v7.9", &["MapleMono-TTF.zip", "MapleMono-TTF.zip"]),
        ];
        retain_font_releases(font, &mut releases)?;
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "v7.9");
        assert!(
            fontferry_core::select_release_assets(font, &releases[0], &["maplemono-ttf".into()])
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn metadata_and_downloads_connect_through_the_configured_proxy()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = NonZeroU16::new(listener.local_addr()?.port()).ok_or("missing proxy port")?;
        let client = HttpClient::with_github_proxy(Some(port))?;
        let proxy = async {
            for host in ["api.github.com", "release-assets.githubusercontent.com"] {
                let (socket, _) = listener.accept().await?;
                let mut stream = BufReader::new(socket);
                let mut line = String::new();
                stream.read_line(&mut line).await?;
                assert_eq!(line, format!("CONNECT {host}:443 HTTP/1.1\r\n"));
                loop {
                    line.clear();
                    if stream.read_line(&mut line).await? == 0 || line == "\r\n" {
                        break;
                    }
                }
                stream
                    .get_mut()
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                    .await?;
            }
            Ok::<_, std::io::Error>(())
        };
        let requests = async {
            assert!(client.raw().get(GITHUB_API).send().await.is_err());
            assert!(
                client
                    .download_client()
                    .get("https://release-assets.githubusercontent.com/font.zip")
                    .send()
                    .await
                    .is_err()
            );
        };
        let (proxy_result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(proxy, requests)
        })
        .await?;
        proxy_result?;
        Ok(())
    }

    #[test]
    fn proxy_scope_excludes_custom_hosts_and_ports() -> std::result::Result<(), url::ParseError> {
        for url in [
            "https://github.com.example.org",
            "https://github.com:8443",
            "http://github.com:443",
            "https://localhost",
            "https://127.0.0.1",
        ] {
            assert!(!github_proxy_target(&Url::parse(url)?), "{url}");
        }
        assert!(github_proxy_target(&Url::parse("https://github.com")?));
        Ok(())
    }

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
