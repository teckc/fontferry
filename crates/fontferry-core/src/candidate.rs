use std::collections::BTreeSet;

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ArtifactProvider, FontDefinition, FontFerryError, Release, ReleaseAsset, Result};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallCandidate {
    pub version: String,
    pub asset_names: Vec<String>,
    pub identity: String,
}

pub(crate) fn install_candidate(
    font: &FontDefinition,
    release: &Release,
    variants: &[String],
) -> Result<InstallCandidate> {
    let provider = font
        .artifact_provider
        .as_ref()
        .ok_or(FontFerryError::ReminderOnly)?;
    let (asset_names, assets) = match provider {
        ArtifactProvider::GitHubAsset { .. } => {
            let selected = select_release_assets(font, release, variants)?;
            let names = selected.iter().map(|asset| asset.name.clone()).collect();
            let identities = selected
                .into_iter()
                .map(|asset| {
                    // GitHub asset IDs represent the underlying object independently of signed redirects.
                    let url = asset.id.is_none().then_some(asset.url.as_str());
                    (
                        asset.id,
                        asset.name.as_str(),
                        asset.size,
                        asset.digest.as_deref(),
                        url,
                    )
                })
                .collect::<Vec<_>>();
            (names, identities)
        }
        ArtifactProvider::DirectUrl { url } => {
            let name = url
                .path_segments()
                .and_then(Iterator::last)
                .filter(|name| !name.is_empty())
                .unwrap_or("font-download");
            (vec![name.to_owned()], Vec::new())
        }
    };
    let mut variants = variants.to_vec();
    variants.sort();
    variants.dedup();
    let bytes = serde_json::to_vec(&(font, &release.version, variants, assets))
        .map_err(|error| FontFerryError::State(error.to_string()))?;
    Ok(InstallCandidate {
        version: release.version.clone(),
        asset_names,
        identity: hex::encode(Sha256::digest(bytes)),
    })
}

pub fn select_release_assets<'a>(
    font: &FontDefinition,
    release: &'a Release,
    requested_variants: &[String],
) -> Result<Vec<&'a ReleaseAsset>> {
    let requested: BTreeSet<_> = requested_variants.iter().map(String::as_str).collect();
    if requested
        .iter()
        .any(|id| !font.variants.iter().any(|v| v.id == *id))
    {
        return Err(FontFerryError::DownloadRejected(
            "unknown variant ID".into(),
        ));
    }
    let variants: Vec<_> = font
        .variants
        .iter()
        .filter(|v| requested.contains(v.id.as_str()))
        .collect();
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
