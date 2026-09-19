use std::cmp::Ordering;

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionChange {
    Patch,
    Minor,
    Major,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionUpdates {
    pub patch: Option<String>,
    pub minor: Option<String>,
    pub major: Option<String>,
    pub latest: Option<String>,
    pub change: Option<VersionChange>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NumericVersion {
    major: u64,
    minor: u64,
}

/// Find the newest stable patch, minor, major, and overall update.
#[must_use]
pub fn analyze_versions(
    current: &str,
    available: &[String],
    include_prerelease: bool,
) -> VersionUpdates {
    let current_numeric = numeric_version(current);
    let mut patch = None;
    let mut minor = None;
    let mut major = None;
    let mut other = None;
    for candidate in available {
        if candidate == current
            || (!include_prerelease && is_prerelease(candidate))
            || compare_versions(candidate, current) != Ordering::Greater
        {
            continue;
        }
        match (current_numeric, numeric_version(candidate)) {
            (Some(current), Some(candidate_numeric))
                if candidate_numeric.major == current.major
                    && candidate_numeric.minor == current.minor =>
            {
                select_newer(&mut patch, candidate);
            }
            (Some(current), Some(candidate_numeric))
                if candidate_numeric.major == current.major
                    && candidate_numeric.minor > current.minor =>
            {
                select_newer(&mut minor, candidate);
            }
            (Some(current), Some(candidate_numeric)) if candidate_numeric.major > current.major => {
                select_newer(&mut major, candidate);
            }
            _ => select_newer(&mut other, candidate),
        }
    }
    let (latest, change) = [
        (&patch, VersionChange::Patch),
        (&minor, VersionChange::Minor),
        (&major, VersionChange::Major),
        (&other, VersionChange::Other),
    ]
    .into_iter()
    .filter_map(|(version, change)| version.as_ref().map(|version| (version, change)))
    .max_by(|(left, _), (right, _)| compare_versions(left, right))
    .map_or((None, None), |(version, change)| {
        (Some(version.clone()), Some(change))
    });
    VersionUpdates {
        patch,
        minor,
        major,
        latest,
        change,
    }
}

fn select_newer(selected: &mut Option<String>, candidate: &str) {
    if selected
        .as_ref()
        .is_none_or(|current| compare_versions(candidate, current) == Ordering::Greater)
    {
        *selected = Some(candidate.to_owned());
    }
}

fn numeric_version(version: &str) -> Option<NumericVersion> {
    let mut components = version.split(['.', '-', '_', '+']);
    let major = components.next()?.parse().ok()?;
    let minor = components
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    Some(NumericVersion { major, minor })
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left_numbers = numeric_components(left);
    let right_numbers = numeric_components(right);
    for index in 0..left_numbers.len().max(right_numbers.len()) {
        let ordering = left_numbers
            .get(index)
            .copied()
            .unwrap_or_default()
            .cmp(&right_numbers.get(index).copied().unwrap_or_default());
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    qualifier_key(left).cmp(&qualifier_key(right))
}

fn numeric_components(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '_', '+'])
        .map_while(|value| value.parse().ok())
        .collect()
}

fn qualifier_key(version: &str) -> (i8, u64, String) {
    let normalized = version.to_ascii_lowercase();
    let qualifier = normalized
        .split(['.', '-', '_', '+'])
        .skip_while(|value| value.chars().all(|character| character.is_ascii_digit()))
        .collect::<Vec<_>>()
        .join(".");
    if qualifier.is_empty() || matches!(qualifier.as_str(), "final" | "ga" | "release") {
        return (0, 0, String::new());
    }
    let number = qualifier
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or_default();
    let rank = if qualifier.starts_with("snapshot") {
        -60
    } else if qualifier_marker(&qualifier, "alpha", "a") {
        -50
    } else if qualifier_marker(&qualifier, "beta", "b") {
        -40
    } else if qualifier_marker(&qualifier, "milestone", "m") {
        -30
    } else if qualifier_marker(&qualifier, "rc", "rc") || qualifier_marker(&qualifier, "cr", "cr") {
        -20
    } else if qualifier_marker(&qualifier, "ea", "ea")
        || qualifier_marker(&qualifier, "preview", "preview")
    {
        -10
    } else if qualifier.starts_with("sp") {
        10
    } else {
        0
    };
    (rank, number, qualifier)
}

fn qualifier_marker(qualifier: &str, long: &str, short: &str) -> bool {
    [long, short].into_iter().any(|marker| {
        qualifier.strip_prefix(marker).is_some_and(|suffix| {
            suffix.is_empty()
                || suffix.starts_with('.')
                || suffix.starts_with(|character: char| character.is_ascii_digit())
        })
    })
}

fn is_prerelease(version: &str) -> bool {
    let qualifier = qualifier_key(version);
    qualifier.0 < 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_latest_patch_minor_and_major_updates() {
        let versions = ["1.2.3", "1.2.4", "1.3.0", "2.0.0", "2.1.0"].map(str::to_owned);
        let updates = analyze_versions("1.2.3", &versions, false);
        assert_eq!(updates.patch.as_deref(), Some("1.2.4"));
        assert_eq!(updates.minor.as_deref(), Some("1.3.0"));
        assert_eq!(updates.major.as_deref(), Some("2.1.0"));
        assert_eq!(updates.latest.as_deref(), Some("2.1.0"));
        assert_eq!(updates.change, Some(VersionChange::Major));
    }

    #[test]
    fn excludes_prereleases_unless_requested() {
        let versions = ["1.0.0", "1.1.0-rc1", "2.0.0-M1"].map(str::to_owned);
        assert_eq!(analyze_versions("1.0.0", &versions, false).latest, None);
        let updates = analyze_versions("1.0.0", &versions, true);
        assert_eq!(updates.latest.as_deref(), Some("2.0.0-M1"));
        assert_eq!(updates.change, Some(VersionChange::Major));
    }

    #[test]
    fn orders_maven_qualifiers_around_the_final_release() {
        assert_eq!(
            compare_versions("1.0.0-rc2", "1.0.0-rc1"),
            Ordering::Greater
        );
        assert_eq!(compare_versions("1.0.0", "1.0.0-rc2"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0-sp1", "1.0.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0.Final", "1.0.0"), Ordering::Equal);
        assert!(!is_prerelease("1.0.0-android"));
    }

    #[test]
    fn reports_other_updates_for_non_numeric_versions() {
        let updates = analyze_versions("blue", &["green".to_owned()], false);
        assert_eq!(updates.latest.as_deref(), Some("green"));
        assert_eq!(updates.change, Some(VersionChange::Other));
    }
}
