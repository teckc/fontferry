use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use fontferry_core::{
    ArtifactProvider, ArtifactSource, FontDefinition, FontFerryError, Release, ReleaseAsset, Result,
};
use futures_util::StreamExt;
use regex::Regex;
use reqwest::{Client, StatusCode};
use sha2::{Digest, Sha256};
use tokio::{fs::File, io::AsyncWriteExt};
use url::Url;

use crate::{HttpClient, validate_public_https};

const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_DOWNLOAD_ATTEMPTS: usize = 3;

#[async_trait]
impl ArtifactSource for HttpClient {
    async fn download(
        &self,
        font: &FontDefinition,
        provider: &ArtifactProvider,
        release: &Release,
        variant_ids: &[String],
        staging_directory: &Path,
    ) -> Result<Vec<PathBuf>> {
        let assets = match provider {
            ArtifactProvider::GitHubAsset { repository: _ } => {
                select_assets(font, release, variant_ids)?
                    .into_iter()
                    .map(|asset| {
                        let url = Url::parse(&asset.url)
                            .map_err(|error| FontFerryError::Network(error.to_string()))?;
                        Ok((url, asset.name.clone(), asset.digest.clone()))
                    })
                    .collect::<Result<Vec<_>>>()?
            }
            ArtifactProvider::DirectUrl { url } => {
                let name = url
                    .path_segments()
                    .and_then(Iterator::last)
                    .filter(|name| !name.is_empty())
                    .unwrap_or("font-download")
                    .to_owned();
                vec![(url.clone(), name, None)]
            }
        };

        let mut paths = Vec::with_capacity(assets.len());
        for (url, name, digest) in assets {
            paths.push(
                download_one(
                    self,
                    &url,
                    &safe_filename(&name),
                    digest.as_deref(),
                    staging_directory,
                )
                .await?,
            );
        }
        Ok(paths)
    }
}

fn select_assets<'a>(
    font: &FontDefinition,
    release: &'a Release,
    requested_variants: &[String],
) -> Result<Vec<&'a ReleaseAsset>> {
    let requested: BTreeSet<_> = requested_variants.iter().map(String::as_str).collect();
    let variants: Vec<_> = if requested.is_empty() {
        font.variants
            .iter()
            .filter(|variant| variant.default)
            .collect()
    } else {
        font.variants
            .iter()
            .filter(|variant| requested.contains(variant.id.as_str()))
            .collect()
    };
    if variants.is_empty() {
        return Err(FontFerryError::DownloadRejected(
            "no font variants were selected".into(),
        ));
    }

    let mut selected = Vec::new();
    for variant in variants {
        let pattern = Regex::new(&variant.asset_pattern)
            .map_err(|error| FontFerryError::InvalidCatalog(error.to_string()))?;
        let matches: Vec<_> = release
            .assets
            .iter()
            .filter(|asset| pattern.is_match(&asset.name))
            .collect();
        if matches.len() != 1 {
            return Err(FontFerryError::DownloadRejected(format!(
                "variant '{}' matched {} release assets; expected exactly one",
                variant.id,
                matches.len()
            )));
        }
        selected.push(matches[0]);
    }
    selected.sort_by(|left, right| left.name.cmp(&right.name));
    selected.dedup_by(|left, right| left.name == right.name);
    Ok(selected)
}

async fn download_one(
    client: &HttpClient,
    url: &Url,
    filename: &str,
    expected_digest: Option<&str>,
    staging_directory: &Path,
) -> Result<PathBuf> {
    validate_public_https(url)?;
    download_with_retries(
        client.download_client(),
        url,
        filename,
        expected_digest,
        staging_directory,
    )
    .await
}

async fn download_with_retries(
    client: &Client,
    url: &Url,
    filename: &str,
    expected_digest: Option<&str>,
    staging_directory: &Path,
) -> Result<PathBuf> {
    let partial = staging_directory.join(format!("{filename}.part"));
    for attempt in 1..=MAX_DOWNLOAD_ATTEMPTS {
        match download_attempt(client, url, filename, expected_digest, staging_directory).await {
            Ok(path) => return Ok(path),
            Err(AttemptError::Network(error)) => {
                let retry = attempt < MAX_DOWNLOAD_ATTEMPTS && retryable(&error);
                tracing::warn!(
                    attempt,
                    max_attempts = MAX_DOWNLOAD_ATTEMPTS,
                    retry,
                    error = ?error,
                    "font download failed"
                );
                let _ = tokio::fs::remove_file(&partial).await;
                if !retry {
                    return Err(friendly_download_error(&error));
                }
                tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
            }
            Err(AttemptError::Fatal(error)) => {
                let _ = tokio::fs::remove_file(&partial).await;
                return Err(error);
            }
        }
    }
    Err(FontFerryError::Network("下载失败，请检查网络后重试".into()))
}

enum AttemptError {
    Network(reqwest::Error),
    Fatal(FontFerryError),
}

async fn download_attempt(
    client: &Client,
    url: &Url,
    filename: &str,
    expected_digest: Option<&str>,
    staging_directory: &Path,
) -> std::result::Result<PathBuf, AttemptError> {
    let response = client
        .get(url.clone())
        .header(reqwest::header::USER_AGENT, "FontFerry/0.2")
        .send()
        .await
        .map_err(AttemptError::Network)?
        .error_for_status()
        .map_err(AttemptError::Network)?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD_BYTES)
    {
        return Err(AttemptError::Fatal(FontFerryError::DownloadRejected(
            "download exceeds the 2 GiB limit".into(),
        )));
    }

    let partial = staging_directory.join(format!("{filename}.part"));
    let completed = staging_directory.join(filename);
    let mut file = File::create(&partial)
        .await
        .map_err(|error| AttemptError::Fatal(FontFerryError::State(error.to_string())))?;
    let mut stream = response.bytes_stream();
    let mut size = 0_u64;
    let mut hash = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(AttemptError::Network)?;
        size = size.saturating_add(chunk.len() as u64);
        if size > MAX_DOWNLOAD_BYTES {
            return Err(AttemptError::Fatal(FontFerryError::DownloadRejected(
                "download exceeds the 2 GiB limit".into(),
            )));
        }
        hash.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| AttemptError::Fatal(FontFerryError::State(error.to_string())))?;
    }
    file.flush()
        .await
        .map_err(|error| AttemptError::Fatal(FontFerryError::State(error.to_string())))?;
    let actual = hex::encode(hash.finalize());
    if let Some(expected) = expected_digest.and_then(normalize_sha256)
        && !actual.eq_ignore_ascii_case(expected)
    {
        return Err(AttemptError::Fatal(FontFerryError::DownloadRejected(
            format!("SHA-256 mismatch for '{filename}'"),
        )));
    }
    tokio::fs::rename(&partial, &completed)
        .await
        .map_err(|error| AttemptError::Fatal(FontFerryError::State(error.to_string())))?;
    Ok(completed)
}

fn retryable(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error.is_connect()
        || error.is_body()
        || error.is_decode()
        || error.status().is_some_and(|status| {
            status.is_server_error()
                || status == StatusCode::REQUEST_TIMEOUT
                || status == StatusCode::TOO_MANY_REQUESTS
        })
}

fn friendly_download_error(error: &reqwest::Error) -> FontFerryError {
    let message = if error.is_timeout() {
        "下载超时：连接长时间没有收到数据，请检查网络后重试"
    } else if error.is_connect() {
        "无法连接下载服务器，请检查网络或代理设置后重试"
    } else if error.is_body() || error.is_decode() {
        "下载连接中断，收到的文件不完整，请重试"
    } else if let Some(status) = error.status() {
        return FontFerryError::Network(format!("下载服务器返回 HTTP {status}"));
    } else {
        "下载失败，请检查网络后重试"
    };
    FontFerryError::Network(message.into())
}

fn normalize_sha256(value: &str) -> Option<&str> {
    value
        .strip_prefix("sha256:")
        .or_else(|| (value.len() == 64).then_some(value))
}

fn safe_filename(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(180)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use super::*;

    #[test]
    fn filename_removes_path_and_shell_characters() {
        assert_eq!(safe_filename("../../evil font.zip"), ".._.._evil_font.zip");
        assert_eq!(safe_filename("font;$x.zip"), "font__x.zip");
    }

    #[tokio::test]
    async fn retries_an_interrupted_response_and_replaces_partial_file()
    -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let server = thread::spawn(move || -> std::io::Result<()> {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept()?;
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request)?;
                if attempt == 0 {
                    stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc",
                    )?;
                } else {
                    stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789",
                    )?;
                }
            }
            Ok(())
        });
        let client = Client::builder().build()?;
        let directory = tempfile::tempdir()?;
        let path = download_with_retries(
            &client,
            &Url::parse(&format!("http://{address}/font.zip"))?,
            "font.zip",
            None,
            directory.path(),
        )
        .await?;
        assert_eq!(tokio::fs::read(path).await?, b"0123456789");
        assert!(!directory.path().join("font.zip.part").exists());
        server
            .join()
            .map_err(|_| std::io::Error::other("test server panicked"))??;
        Ok(())
    }
}
