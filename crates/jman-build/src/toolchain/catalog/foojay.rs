use std::collections::BTreeMap;

#[cfg(test)]
use std::collections::BTreeSet;

use reqwest::{header, Client, StatusCode, Url};
use serde::Deserialize;

use crate::{toolchain::version_key, BuildError};

use super::{
    CatalogRelease, CatalogValidators, JdkCatalogProvider, Platform, ProviderCatalog,
    ProviderFuture, ResolvedArtifact,
};

const PROVIDER_ID: &str = "foojay";
const API_ROOT: &str = "https://api.foojay.io/disco/v3.0";

pub(crate) struct FoojayProvider {
    api_root: Url,
}

impl FoojayProvider {
    pub(crate) fn new() -> Result<Self, BuildError> {
        Self::with_api_root(API_ROOT)
    }

    #[cfg(test)]
    pub(crate) fn with_api_root(api_root: &str) -> Result<Self, BuildError> {
        let api_root = Url::parse(api_root)
            .map_err(|error| BuildError::Invalid(format!("invalid Foojay API URL: {error}")))?;
        Ok(Self { api_root })
    }

    #[cfg(not(test))]
    fn with_api_root(api_root: &str) -> Result<Self, BuildError> {
        let api_root = Url::parse(api_root)
            .map_err(|error| BuildError::Invalid(format!("invalid Foojay API URL: {error}")))?;
        Ok(Self { api_root })
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, BuildError> {
        let mut url = self.api_root.clone();
        url.path_segments_mut()
            .map_err(|()| BuildError::Invalid("invalid Foojay API root".to_owned()))?
            .extend(segments);
        Ok(url)
    }

    fn package_request(
        &self,
        client: &Client,
        platform: Platform<'_>,
    ) -> Result<reqwest::RequestBuilder, BuildError> {
        Ok(client.get(self.endpoint(&["packages"])?).query(&[
            ("package_type", "jdk"),
            ("directly_downloadable", "true"),
            ("operating_system", provider_os(platform.os)),
            ("architecture", provider_arch(platform.architecture)),
            ("archive_type", platform.archive_type),
            ("release_status", "ga"),
            ("javafx_bundled", "false"),
            ("libc_type", platform.libc),
        ]))
    }
}

impl JdkCatalogProvider for FoojayProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn fetch_catalog<'a>(
        &'a self,
        client: &'a Client,
        platform: Platform<'a>,
        validators: Option<&'a CatalogValidators>,
    ) -> ProviderFuture<'a, ProviderCatalog> {
        Box::pin(async move {
            let mut request = self
                .package_request(client, platform)?
                .query(&[("latest", "available")]);
            if let Some(etag) = validators.and_then(|value| value.etag.as_deref()) {
                request = request.header(header::IF_NONE_MATCH, etag);
            }
            if let Some(modified) = validators.and_then(|value| value.last_modified.as_deref()) {
                request = request.header(header::IF_MODIFIED_SINCE, modified);
            }
            let response = request.send().await.map_err(metadata_request_error)?;
            if response.status() == StatusCode::NOT_MODIFIED {
                return Ok(ProviderCatalog::NotModified);
            }
            let response = response
                .error_for_status()
                .map_err(metadata_request_error)?;
            let validators = CatalogValidators {
                etag: response_header(&response, header::ETAG),
                last_modified: response_header(&response, header::LAST_MODIFIED),
            };
            let packages = response
                .json::<FoojayResponse<FoojayPackage>>()
                .await
                .map_err(|error| {
                    BuildError::Invalid(format!("invalid Foojay catalog metadata: {error}"))
                })?;
            Ok(ProviderCatalog::Modified {
                validators,
                releases: translate_packages(packages.result, platform),
            })
        })
    }

    fn resolve<'a>(
        &'a self,
        client: &'a Client,
        platform: Platform<'a>,
        vendor: &'a str,
        version: &'a str,
        exact: bool,
    ) -> ProviderFuture<'a, ResolvedArtifact> {
        Box::pin(async move {
            let distribution = provider_distribution(vendor).ok_or_else(|| {
                BuildError::Invalid(format!("unsupported JDK distribution `{vendor}`"))
            })?;
            let provider_version = provider_version(version);
            let request = self.package_request(client, platform)?.query(&[
                ("distro", distribution),
                ("version", provider_version.as_str()),
                ("latest", "available"),
            ]);
            let response = request
                .send()
                .await
                .map_err(metadata_request_error)?
                .error_for_status()
                .map_err(metadata_request_error)?
                .json::<FoojayResponse<FoojayPackage>>()
                .await
                .map_err(|error| {
                    BuildError::Invalid(format!("invalid Foojay package metadata: {error}"))
                })?;
            let mut releases = translate_packages(response.result, platform)
                .into_iter()
                .filter(|release| {
                    release.vendor == vendor
                        && if exact {
                            release.version == version
                        } else {
                            release.major.to_string() == version
                        }
                })
                .collect::<Vec<_>>();
            releases.sort_by(|left, right| {
                version_key(&right.version).cmp(&version_key(&left.version))
            });
            let release = releases.into_iter().next().ok_or_else(|| {
                BuildError::Invalid(format!("no {vendor} JDK found for {version}"))
            })?;

            let detail = client
                .get(self.endpoint(&["ids", &release.source_id])?)
                .send()
                .await
                .map_err(metadata_request_error)?
                .error_for_status()
                .map_err(metadata_request_error)?
                .json::<FoojayResponse<FoojayPackageInfo>>()
                .await
                .map_err(|error| {
                    BuildError::Invalid(format!("invalid Foojay download metadata: {error}"))
                })?
                .result
                .into_iter()
                .next()
                .ok_or_else(|| {
                    BuildError::Invalid(format!(
                        "Foojay returned no download metadata for {}",
                        release.source_id
                    ))
                })?;
            if !detail.checksum_type.eq_ignore_ascii_case("sha256")
                || detail.checksum.len() != 64
                || !detail
                    .checksum
                    .chars()
                    .all(|value| value.is_ascii_hexdigit())
            {
                return Err(BuildError::Invalid(format!(
                    "Foojay returned no valid SHA-256 checksum for {}",
                    release.source_id
                )));
            }
            let download_url = Url::parse(&detail.direct_download_uri).map_err(|error| {
                BuildError::Invalid(format!("invalid Foojay JDK download URL: {error}"))
            })?;
            Ok(ResolvedArtifact {
                release,
                download_url,
                checksum_sha256: detail.checksum.to_ascii_lowercase(),
            })
        })
    }
}

#[derive(Debug, Deserialize)]
struct FoojayResponse<T> {
    result: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct FoojayPackage {
    id: String,
    archive_type: String,
    distribution: String,
    major_version: u16,
    java_version: String,
    release_status: String,
    term_of_support: String,
    operating_system: String,
    lib_c_type: String,
    architecture: String,
    package_type: String,
    javafx_bundled: bool,
    directly_downloadable: bool,
    filename: String,
    free_use_in_production: bool,
}

#[derive(Debug, Deserialize)]
struct FoojayPackageInfo {
    direct_download_uri: String,
    checksum: String,
    checksum_type: String,
}

fn translate_packages(packages: Vec<FoojayPackage>, platform: Platform<'_>) -> Vec<CatalogRelease> {
    let mut releases = BTreeMap::new();
    for package in packages {
        let Some(vendor) = canonical_distribution(&package.distribution) else {
            continue;
        };
        if package.package_type != "jdk"
            || package.release_status != "ga"
            || !package.directly_downloadable
            || package.javafx_bundled
            || !package.free_use_in_production
            || package.archive_type != platform.archive_type
            || normalize_os(&package.operating_system) != Some(platform.os)
            || normalize_arch(&package.architecture) != Some(platform.architecture)
            || !libc_matches(platform.libc, &package.lib_c_type)
        {
            continue;
        }
        let release = CatalogRelease {
            vendor: vendor.to_owned(),
            version: normalize_version(&package.java_version, package.major_version),
            major: package.major_version,
            lts: package.term_of_support == "lts",
            os: platform.os.to_owned(),
            architecture: platform.architecture.to_owned(),
            archive_type: package.archive_type,
            filename: package.filename,
            source_id: package.id,
        };
        let key = (release.vendor.clone(), release.version.clone());
        releases.entry(key).or_insert(release);
    }
    let mut releases = releases.into_values().collect::<Vec<_>>();
    releases.sort_by(|left, right| {
        left.vendor.cmp(&right.vendor).then_with(|| {
            (right.major, version_key(&right.version))
                .cmp(&(left.major, version_key(&left.version)))
        })
    });
    releases
}

fn normalize_version(version: &str, major: u16) -> String {
    if major == 8 {
        let numbers = version_key(version);
        if numbers.len() >= 3 && numbers[0] == 8 && numbers[1] == 0 {
            let build = numbers.get(4).copied();
            return build.map_or_else(
                || format!("8u{}", numbers[2]),
                |build| format!("8u{}-b{build:02}", numbers[2]),
            );
        }
    }
    version.to_owned()
}

fn provider_version(version: &str) -> String {
    let Some(update) = version.strip_prefix("8u") else {
        return version.to_owned();
    };
    let (update, build) = update.split_once("-b").unwrap_or((update, ""));
    if build.is_empty() {
        format!("8.0.{update}")
    } else {
        let build = build.trim_start_matches('0');
        format!(
            "8.0.{update}+{}",
            if build.is_empty() { "0" } else { build }
        )
    }
}

fn canonical_distribution(value: &str) -> Option<&'static str> {
    Some(match value {
        "aoj" => "adoptopenjdk",
        "aoj_openj9" => "adoptopenjdk-openj9",
        "bisheng" => "bisheng",
        "corretto" => "corretto",
        "dragonwell" => "dragonwell",
        "eliya" => "eliya",
        "gluon_graalvm" => "gluon-graalvm",
        "graalvm" => "graalvm",
        "graalvm_community" => "graalvm-community",
        "graalvm_ce8" => "graalvm-ce-8",
        "graalvm_ce11" => "graalvm-ce-11",
        "graalvm_ce16" => "graalvm-ce-16",
        "graalvm_ce17" => "graalvm-ce-17",
        "graalvm_ce19" => "graalvm-ce-19",
        "jetbrains" => "jetbrains",
        "kona" => "kona",
        "liberica" => "liberica",
        "liberica_native" => "liberica-native",
        "mandrel" => "mandrel",
        "microsoft" => "microsoft",
        "openlogic" => "openlogic",
        "oracle" => "oracle",
        "oracle_open_jdk" => "openjdk",
        "sap_machine" => "sapmachine",
        "semeru" => "semeru",
        "semeru_certified" => "semeru-certified",
        "temurin" => "temurin",
        "trava" => "trava",
        "zulu" => "zulu",
        "zulu_prime" => "zulu-prime",
        _ => return None,
    })
}

fn provider_distribution(value: &str) -> Option<&'static str> {
    Some(match value {
        "adoptopenjdk" => "aoj",
        "adoptopenjdk-openj9" => "aoj_openj9",
        "bisheng" => "bisheng",
        "corretto" => "corretto",
        "dragonwell" => "dragonwell",
        "eliya" => "eliya",
        "gluon-graalvm" => "gluon_graalvm",
        "graalvm" => "graalvm",
        "graalvm-community" => "graalvm_community",
        "graalvm-ce-8" => "graalvm_ce8",
        "graalvm-ce-11" => "graalvm_ce11",
        "graalvm-ce-16" => "graalvm_ce16",
        "graalvm-ce-17" => "graalvm_ce17",
        "graalvm-ce-19" => "graalvm_ce19",
        "jetbrains" => "jetbrains",
        "kona" => "kona",
        "liberica" => "liberica",
        "liberica-native" => "liberica_native",
        "mandrel" => "mandrel",
        "microsoft" => "microsoft",
        "openlogic" => "openlogic",
        "oracle" => "oracle",
        "openjdk" => "oracle_open_jdk",
        "sapmachine" => "sap_machine",
        "semeru" => "semeru",
        "semeru-certified" => "semeru_certified",
        "temurin" => "temurin",
        "trava" => "trava",
        "zulu" => "zulu",
        "zulu-prime" => "zulu_prime",
        _ => return None,
    })
}

fn provider_os(value: &str) -> &str {
    if value == "mac" {
        "macos"
    } else {
        value
    }
}

fn provider_arch(value: &str) -> &str {
    value
}

fn normalize_os(value: &str) -> Option<&str> {
    match value {
        "macos" => Some("mac"),
        "linux" | "windows" => Some(value),
        _ => None,
    }
}

fn normalize_arch(value: &str) -> Option<&str> {
    match value {
        "x64" | "amd64" | "x86-64" => Some("x64"),
        "aarch64" | "arm64" => Some("aarch64"),
        "arm" => Some("arm"),
        "ppc64le" | "ppc64el" => Some("ppc64le"),
        "s390x" => Some("s390x"),
        "riscv64" => Some("riscv64"),
        _ => None,
    }
}

fn libc_matches(expected: &str, actual: &str) -> bool {
    expected == actual
        || (expected == "glibc" && matches!(actual, "c_std_lib" | "libc"))
        || (expected == "libc" && matches!(actual, "c_std_lib" | "glibc"))
}

fn response_header(response: &reqwest::Response, name: header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn metadata_request_error(error: reqwest::Error) -> BuildError {
    let message = error.to_string();
    drop(error);
    BuildError::Invalid(format!("Foojay metadata request failed: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform() -> Platform<'static> {
        Platform {
            os: "linux",
            architecture: "x64",
            libc: "glibc",
            archive_type: "tar.gz",
        }
    }

    #[test]
    fn translates_foojay_names_into_provider_neutral_releases() {
        let packages = vec![FoojayPackage {
            id: "package-1".to_owned(),
            archive_type: "tar.gz".to_owned(),
            distribution: "oracle_open_jdk".to_owned(),
            major_version: 21,
            java_version: "21.0.8+9".to_owned(),
            release_status: "ga".to_owned(),
            term_of_support: "lts".to_owned(),
            operating_system: "linux".to_owned(),
            lib_c_type: "glibc".to_owned(),
            architecture: "amd64".to_owned(),
            package_type: "jdk".to_owned(),
            javafx_bundled: false,
            directly_downloadable: true,
            filename: "openjdk.tar.gz".to_owned(),
            free_use_in_production: true,
        }];

        let releases = translate_packages(packages, platform());

        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].vendor, "openjdk");
        assert_eq!(releases[0].version, "21.0.8+9");
        assert_eq!(releases[0].source_id, "package-1");
    }

    #[test]
    fn excludes_unknown_non_free_and_incompatible_packages() {
        let package = |distribution: &str, free: bool, libc: &str| FoojayPackage {
            id: distribution.to_owned(),
            archive_type: "tar.gz".to_owned(),
            distribution: distribution.to_owned(),
            major_version: 21,
            java_version: "21.0.8+9".to_owned(),
            release_status: "ga".to_owned(),
            term_of_support: "lts".to_owned(),
            operating_system: "linux".to_owned(),
            lib_c_type: libc.to_owned(),
            architecture: "x64".to_owned(),
            package_type: "jdk".to_owned(),
            javafx_bundled: false,
            directly_downloadable: true,
            filename: "jdk.tar.gz".to_owned(),
            free_use_in_production: free,
        };
        let releases = translate_packages(
            vec![
                package("future_provider_name", true, "glibc"),
                package("oracle", false, "glibc"),
                package("zulu", true, "musl"),
            ],
            platform(),
        );
        assert!(releases.is_empty());
    }

    #[test]
    fn every_supported_distribution_has_a_reversible_mapping() {
        let supported = BTreeSet::from([
            "adoptopenjdk",
            "adoptopenjdk-openj9",
            "bisheng",
            "corretto",
            "dragonwell",
            "eliya",
            "gluon-graalvm",
            "graalvm",
            "graalvm-community",
            "graalvm-ce-8",
            "graalvm-ce-11",
            "graalvm-ce-16",
            "graalvm-ce-17",
            "graalvm-ce-19",
            "jetbrains",
            "kona",
            "liberica",
            "liberica-native",
            "mandrel",
            "microsoft",
            "openlogic",
            "openjdk",
            "oracle",
            "sapmachine",
            "semeru",
            "semeru-certified",
            "temurin",
            "trava",
            "zulu",
            "zulu-prime",
        ]);
        for vendor in supported {
            let provider = provider_distribution(vendor).expect("provider mapping");
            assert_eq!(canonical_distribution(provider), Some(vendor));
        }
    }
}
