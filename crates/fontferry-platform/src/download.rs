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
use sha2::{Digest, Sha256};
use tokio::{fs::File, io::AsyncWriteExt};
use url::Url;

use crate::{HttpClient, validate_public_https};

const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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
        font.variants.iter().filter(|variant| variant.default).collect()
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
    let response = client
        .raw()
        .get(url.clone())
        .header(reqwest::header::USER_AGENT, "FontFerry/0.2")
        .send()
        .await
        .map_err(|error| FontFerryError::Network(error.to_string()))?
        .error_for_status()
        .map_err(|error| FontFerryError::Network(error.to_string()))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD_BYTES)
    {
        return Err(FontFerryError::DownloadRejected(
            "download exceeds the 2 GiB limit".into(),
        ));
    }

    let partial = staging_directory.join(format!("{filename}.part"));
    let completed = staging_directory.join(filename);
    let mut file = File::create(&partial)
        .await
        .map_err(|error| FontFerryError::State(error.to_string()))?;
    let mut stream = response.bytes_stream();
    let mut size = 0_u64;
    let mut hash = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| FontFerryError::Network(error.to_string()))?;
        size = size.saturating_add(chunk.len() as u64);
        if size > MAX_DOWNLOAD_BYTES {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(FontFerryError::DownloadRejected(
                "download exceeds the 2 GiB limit".into(),
            ));
        }
        hash.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| FontFerryError::State(error.to_string()))?;
    }
    file.flush()
        .await
        .map_err(|error| FontFerryError::State(error.to_string()))?;
    let actual = hex::encode(hash.finalize());
    if let Some(expected) = expected_digest.and_then(normalize_sha256)
        && !actual.eq_ignore_ascii_case(expected)
    {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(FontFerryError::DownloadRejected(format!(
            "SHA-256 mismatch for '{filename}'"
        )));
    }
    tokio::fs::rename(&partial, &completed)
        .await
        .map_err(|error| FontFerryError::State(error.to_string()))?;
    Ok(completed)
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
    use super::*;

    #[test]
    fn filename_removes_path_and_shell_characters() {
        assert_eq!(safe_filename("../../evil font.zip"), ".._.._evil_font.zip");
        assert_eq!(safe_filename("font;$x.zip"), "font__x.zip");
    }
}

