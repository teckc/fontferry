use std::cmp::Ordering;

use semver::Version;
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};

use crate::{VersionPolicy, catalog::ReleaseChannel};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub published_at: OffsetDateTime,
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub digest: Option<String>,
}

pub fn select_latest<'a>(
    releases: &'a [Release],
    channel: ReleaseChannel,
    policy: &VersionPolicy,
) -> Option<&'a Release> {
    releases
        .iter()
        .filter(|release| {
            channel == ReleaseChannel::Prerelease || !release.prerelease
        })
        .filter(|release| {
            policy
                .updates_through
                .is_none_or(|date| release.published_at.date() <= date)
        })
        .filter(|release| {
            policy.major.is_none_or(|major| {
                parse_version(&release.version).is_some_and(|version| version.major == major)
            })
        })
        .filter(|release| {
            policy.maximum_version.as_ref().is_none_or(|maximum| {
                compare_versions(&release.version, maximum) != Ordering::Greater
            })
        })
        .max_by(|left, right| compare_releases(left, right))
}

pub fn is_update_available(current: &str, available: &str) -> bool {
    compare_versions(available, current) == Ordering::Greater
}

fn compare_releases(left: &Release, right: &Release) -> Ordering {
    let version_order = compare_versions(&left.version, &right.version);
    if version_order == Ordering::Equal {
        left.published_at.cmp(&right.published_at)
    } else {
        version_order
    }
}

pub fn compare_versions(left: &str, right: &str) -> Ordering {
    match (parse_version(left), parse_version(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

pub fn parse_version(value: &str) -> Option<Version> {
    Version::parse(value.trim().trim_start_matches(['v', 'V'])).ok()
}

pub fn date_from_iso(value: &str) -> Option<Date> {
    Date::parse(
        value,
        &time::format_description::well_known::Iso8601::DEFAULT,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    fn release(version: &str, date: OffsetDateTime, prerelease: bool) -> Release {
        Release {
            version: version.into(),
            published_at: date,
            prerelease,
            assets: Vec::new(),
        }
    }

    #[test]
    fn selects_latest_semver_in_major_and_date_entitlement() {
        let releases = vec![
            release("6.7.2", datetime!(2024-12-12 0:00 UTC), false),
            release("7.2.0", datetime!(2026-02-04 0:00 UTC), false),
            release("7.3.1", datetime!(2026-07-07 0:00 UTC), false),
        ];
        let policy = VersionPolicy {
            major: Some(7),
            maximum_version: None,
            updates_through: Some(time::macros::date!(2026 - 03 - 01)),
        };
        let selected = select_latest(&releases, ReleaseChannel::Stable, &policy);
        assert_eq!(selected.map(|item| item.version.as_str()), Some("7.2.0"));
    }

    #[test]
    fn excludes_prerelease_from_stable_channel() {
        let releases = vec![
            release("1.0.0", datetime!(2026-01-01 0:00 UTC), false),
            release("1.1.0-beta.1", datetime!(2026-02-01 0:00 UTC), true),
        ];
        let selected = select_latest(
            &releases,
            ReleaseChannel::Stable,
            &VersionPolicy::default(),
        );
        assert_eq!(selected.map(|item| item.version.as_str()), Some("1.0.0"));
    }

    #[test]
    fn falls_back_to_lexical_for_non_semver_versions() {
        assert!(is_update_available("release-2025-12", "release-2026-01"));
    }
}

