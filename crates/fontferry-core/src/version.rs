use std::cmp::Ordering;

use semver::Version;
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};

use crate::{VersionPolicy, catalog::ReleaseChannel};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    #[serde(with = "time::serde::rfc3339")]
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

#[must_use]
pub fn select_latest<'a>(
    releases: &'a [Release],
    channel: ReleaseChannel,
    policy: &VersionPolicy,
) -> Option<&'a Release> {
    let eligible: Vec<_> = releases
        .iter()
        .filter(|release| release_eligible(release, channel, policy))
        .collect();
    // Select one ordering for the whole set; pairwise fallback is non-transitive.
    let numeric = eligible
        .iter()
        .all(|release| parse_version(&release.version).is_some());
    eligible.into_iter().max_by(|left, right| {
        let version_order = if numeric {
            compare_versions(&left.version, &right.version).unwrap_or(Ordering::Equal)
        } else {
            Ordering::Equal
        };
        version_order.then_with(|| left.published_at.cmp(&right.published_at))
            // Deterministic tie-break only, not a claim of opaque version precedence.
            .then_with(|| left.version.cmp(&right.version))
    })
}

#[must_use]
pub fn release_eligible(
    release: &Release,
    channel: ReleaseChannel,
    policy: &VersionPolicy,
) -> bool {
    (channel == ReleaseChannel::Prerelease
        || (!release.prerelease
            && parse_version(&release.version).is_none_or(|v| v.pre.is_empty())))
        && policy
            .updates_through
            .is_none_or(|date| release.published_at.date() <= date)
        && policy
            .major
            .is_none_or(|major| parse_version(&release.version).is_some_and(|v| v.major == major))
        && policy.maximum_version.as_ref().is_none_or(|maximum| {
            compare_versions(&release.version, maximum).is_some_and(|o| o != Ordering::Greater)
        })
}

#[must_use]
pub fn is_update_available(current: &str, available: &str) -> bool {
    compare_versions(available, current) == Some(Ordering::Greater)
}

#[must_use]
pub fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    match (parse_version(left), parse_version(right)) {
        (Some(left), Some(right)) => Some(left.cmp(&right)),
        _ if left == right => Some(Ordering::Equal),
        _ => None,
    }
}

#[must_use]
pub fn parse_version(value: &str) -> Option<Version> {
    let value = value
        .trim()
        .strip_prefix(['v', 'V'])
        .unwrap_or(value.trim());
    if let Ok(version) = Version::parse(value) {
        return Some(version);
    }
    let numeric = value.strip_suffix('R').unwrap_or(value);
    let parts: Vec<_> = numeric.split('.').collect();
    if (parts.len() == 2 || parts.len() == 3)
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    {
        return Some(Version::new(
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            if parts.len() == 3 {
                parts[2].parse().ok()?
            } else {
                0
            },
        ));
    }
    // Date tags sort chronologically. Calendar validation prevents accepting arbitrary labels.
    let date = value.strip_prefix("release-").unwrap_or(value);
    let parts: Vec<_> = date.split('-').collect();
    if parts.len() == 3 && parts[0].len() == 4 {
        let year: i32 = parts[0].parse().ok()?;
        let month: u8 = parts[1].parse().ok()?;
        let day: u8 = parts[2].parse().ok()?;
        Date::from_calendar_date(year, time::Month::try_from(month).ok()?, day).ok()?;
        return Some(Version::new(
            year.try_into().ok()?,
            month.into(),
            day.into(),
        ));
    }
    None
}

#[must_use]
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
        let selected = select_latest(&releases, ReleaseChannel::Stable, &VersionPolicy::default());
        assert_eq!(selected.map(|item| item.version.as_str()), Some("1.0.0"));
    }

    #[test]
    fn mixed_release_selection_is_invariant_under_permutation() {
        let candidates = [
            release("2.0", datetime!(2026-01-01 0:00 UTC), false),
            release("custom", datetime!(2026-02-01 0:00 UTC), false),
            release("1.0", datetime!(2026-03-01 0:00 UTC), false),
        ];
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let releases: Vec<_> = order.iter().map(|i| candidates[*i].clone()).collect();
            assert_eq!(
                select_latest(&releases, ReleaseChannel::Stable, &VersionPolicy::default())
                    .map(|r| r.version.as_str()),
                Some("1.0")
            );
        }
    }

    #[test]
    fn refuses_to_order_unknown_labels() {
        assert_eq!(compare_versions("custom-z", "custom-a"), None);
        assert!(!is_update_available("custom-z", "custom-a"));
    }

    #[test]
    fn compares_numeric_and_date_versions() {
        for (old, new) in [
            ("1.9", "1.10"),
            ("v1.9", "v1.10"),
            ("2.004R", "2.005R"),
            ("v0.108", "v0.210"),
            ("1.0.0-beta.1", "1.0.0"),
            ("2025-12-31", "2026-01-01"),
        ] {
            assert!(is_update_available(old, new));
        }
    }
}
