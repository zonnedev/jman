use std::{future::Future, pin::Pin};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

use crate::BuildError;

mod foojay;

pub(super) use foojay::FoojayProvider;

pub(super) type ProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, BuildError>> + Send + 'a>>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CatalogValidators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CatalogRelease {
    /// Stable JMAN distribution identifier, independent of provider naming.
    pub vendor: String,
    pub version: String,
    pub major: u16,
    pub lts: bool,
    pub os: String,
    pub architecture: String,
    pub archive_type: String,
    pub filename: String,
    /// Opaque reference interpreted only by the provider that emitted it.
    pub source_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResolvedArtifact {
    pub release: CatalogRelease,
    pub download_url: Url,
    pub checksum_sha256: String,
}

pub(super) enum ProviderCatalog {
    NotModified,
    Modified {
        validators: CatalogValidators,
        releases: Vec<CatalogRelease>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Platform<'a> {
    pub os: &'a str,
    pub architecture: &'a str,
    pub libc: &'a str,
    pub archive_type: &'a str,
}

/// Boundary between JMAN's catalog model and an external discovery service.
///
/// Provider response types, parameter names, distribution aliases, and opaque
/// identifiers must be translated before crossing this interface.
pub(super) trait JdkCatalogProvider: Send + Sync {
    fn id(&self) -> &'static str;

    fn fetch_catalog<'a>(
        &'a self,
        client: &'a Client,
        platform: Platform<'a>,
        validators: Option<&'a CatalogValidators>,
    ) -> ProviderFuture<'a, ProviderCatalog>;

    fn resolve<'a>(
        &'a self,
        client: &'a Client,
        platform: Platform<'a>,
        vendor: &'a str,
        version: &'a str,
        exact: bool,
    ) -> ProviderFuture<'a, ResolvedArtifact>;
}

#[derive(Clone, Copy)]
enum UrlStage {
    Initial,
    Redirect,
}

pub(super) fn validate_initial_download_url(vendor: &str, url: &Url) -> Result<(), BuildError> {
    validate_download_url(vendor, url, UrlStage::Initial)
}

pub(super) fn validate_redirect_download_url(vendor: &str, url: &Url) -> Result<(), BuildError> {
    validate_download_url(vendor, url, UrlStage::Redirect)
}

fn validate_download_url(vendor: &str, url: &Url, stage: UrlStage) -> Result<(), BuildError> {
    if url.scheme() != "https" {
        return Err(untrusted_url(vendor, url, "HTTPS is required"));
    }
    if url.port_or_known_default() != Some(443) {
        return Err(untrusted_url(vendor, url, "HTTPS must use port 443"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(untrusted_url(vendor, url, "credentials are not allowed"));
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return Err(untrusted_url(vendor, url, "URL has no host"));
    };
    let path = url.path().to_ascii_lowercase();

    let github_org = match vendor {
        "temurin" => Some("adoptium"),
        "adoptopenjdk" | "adoptopenjdk-openj9" => Some("adoptopenjdk"),
        "dragonwell" => Some("dragonwell-project"),
        "eliya" => Some("asymmsystems"),
        "gluon-graalvm" => Some("gluonhq"),
        "graalvm-community" | "graalvm-ce-8" | "graalvm-ce-11" | "graalvm-ce-16"
        | "graalvm-ce-17" | "graalvm-ce-19" | "mandrel" => Some("graalvm"),
        "kona" => Some("tencent"),
        "liberica" | "liberica-native" => Some("bell-sw"),
        "sapmachine" => Some("sap"),
        "semeru" | "semeru-certified" => Some("ibmruntimes"),
        "trava" => Some("travaopenjdk"),
        _ => None,
    };
    if let Some(org) = github_org {
        if host == "github.com" && path.starts_with(&format!("/{org}/")) {
            return Ok(());
        }
        if matches!(stage, UrlStage::Redirect)
            && matches!(
                host.as_str(),
                "release-assets.githubusercontent.com" | "objects.githubusercontent.com"
            )
        {
            return Ok(());
        }
    }

    let trusted = match vendor {
        "bisheng" => host == "mirror.iscas.ac.cn",
        "corretto" => host == "corretto.aws" || host.ends_with(".amazonaws.com"),
        "graalvm" => {
            host == "gds.oracle.com"
                || host == "download.oracle.com"
                || host == "objectstorage.oraclecloud.com"
                || (host.starts_with("objectstorage.") && host.ends_with(".oraclecloud.com"))
        }
        "jetbrains" => {
            host == "cache-redirector.jetbrains.com"
                || host == "download.jetbrains.com"
                || (matches!(stage, UrlStage::Redirect) && host.ends_with(".cloudfront.net"))
        }
        "microsoft" => {
            host == "aka.ms"
                || host == "download.visualstudio.microsoft.com"
                || host.ends_with(".blob.core.windows.net")
        }
        "openlogic" => host == "builds.openlogic.com",
        "openjdk" => host == "download.java.net",
        "oracle" => host == "download.oracle.com",
        "zulu" | "zulu-prime" => host == "cdn.azul.com",
        _ => false,
    };
    if trusted {
        Ok(())
    } else {
        Err(untrusted_url(
            vendor,
            url,
            "host is not approved for this distribution",
        ))
    }
}

fn untrusted_url(vendor: &str, url: &Url, reason: &str) -> BuildError {
    BuildError::Invalid(format!(
        "refusing untrusted {vendor} JDK download URL `{url}`: {reason}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_trust_is_scoped_to_distribution_and_github_organization() {
        let adoptium = Url::parse(
            "https://github.com/adoptium/temurin21-binaries/releases/download/jdk/file.tar.gz",
        )
        .expect("URL");
        assert!(validate_initial_download_url("temurin", &adoptium).is_ok());
        assert!(validate_initial_download_url("dragonwell", &adoptium).is_err());

        let malicious =
            Url::parse("https://github.com/attacker/jdk/releases/file.tar.gz").expect("URL");
        assert!(validate_initial_download_url("temurin", &malicious).is_err());
    }

    #[test]
    fn download_trust_rejects_plain_http_and_unapproved_redirects() {
        let insecure = Url::parse("http://cdn.azul.com/zulu/jdk.tar.gz").expect("URL");
        assert!(validate_initial_download_url("zulu", &insecure).is_err());
        let unusual_port = Url::parse("https://cdn.azul.com:8443/zulu/jdk.tar.gz").expect("URL");
        assert!(validate_initial_download_url("zulu", &unusual_port).is_err());

        let github_asset = Url::parse(
            "https://release-assets.githubusercontent.com/github-production-release-asset",
        )
        .expect("URL");
        assert!(validate_redirect_download_url("temurin", &github_asset).is_ok());
        assert!(validate_redirect_download_url("zulu", &github_asset).is_err());

        let unknown = Url::parse("https://downloads.example.test/jdk.tar.gz").expect("URL");
        assert!(validate_initial_download_url("temurin", &unknown).is_err());
    }

    #[test]
    fn download_trust_accepts_audited_vendor_cdn_redirects() {
        let microsoft = Url::parse(
            "https://download.visualstudio.microsoft.com/download/pr/id/microsoft-jdk.tar.gz",
        )
        .expect("URL");
        assert!(validate_redirect_download_url("microsoft", &microsoft).is_ok());

        let oracle = Url::parse(
            "https://objectstorage.uk-london-1.oraclecloud.com/p/token/n/namespace/jdk.tar.gz",
        )
        .expect("URL");
        assert!(validate_redirect_download_url("graalvm", &oracle).is_ok());

        let jetbrains =
            Url::parse("https://d2xrhe97vsfxuc.cloudfront.net/jbrsdk.tar.gz").expect("URL");
        assert!(validate_redirect_download_url("jetbrains", &jetbrains).is_ok());
        assert!(validate_initial_download_url("jetbrains", &jetbrains).is_err());
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_supported_distribution_has_an_audited_initial_domain() {
        let trusted = [
            (
                "adoptopenjdk",
                "https://github.com/AdoptOpenJDK/jdk/releases/file.tar.gz",
            ),
            (
                "adoptopenjdk-openj9",
                "https://github.com/AdoptOpenJDK/jdk/releases/file.tar.gz",
            ),
            (
                "bisheng",
                "https://mirror.iscas.ac.cn/kunpeng/bisheng-jdk.tar.gz",
            ),
            ("corretto", "https://corretto.aws/downloads/jdk.tar.gz"),
            (
                "dragonwell",
                "https://github.com/dragonwell-project/jdk/releases/file.tar.gz",
            ),
            (
                "eliya",
                "https://github.com/asymmsystems/eliya-jdk/releases/file.tar.gz",
            ),
            (
                "gluon-graalvm",
                "https://github.com/gluonhq/graal/releases/file.tar.gz",
            ),
            ("graalvm", "https://gds.oracle.com/api/artifacts/id/content"),
            (
                "graalvm-community",
                "https://github.com/graalvm/graalvm-ce-builds/releases/file.tar.gz",
            ),
            (
                "graalvm-ce-8",
                "https://github.com/graalvm/graalvm-ce-builds/releases/file.tar.gz",
            ),
            (
                "graalvm-ce-11",
                "https://github.com/graalvm/graalvm-ce-builds/releases/file.tar.gz",
            ),
            (
                "graalvm-ce-16",
                "https://github.com/graalvm/graalvm-ce-builds/releases/file.tar.gz",
            ),
            (
                "graalvm-ce-17",
                "https://github.com/graalvm/graalvm-ce-builds/releases/file.tar.gz",
            ),
            (
                "graalvm-ce-19",
                "https://github.com/graalvm/graalvm-ce-builds/releases/file.tar.gz",
            ),
            (
                "jetbrains",
                "https://cache-redirector.jetbrains.com/intellij-jbr/jdk.tar.gz",
            ),
            (
                "kona",
                "https://github.com/Tencent/TencentKona/releases/file.tar.gz",
            ),
            (
                "liberica",
                "https://github.com/bell-sw/Liberica/releases/file.tar.gz",
            ),
            (
                "liberica-native",
                "https://github.com/bell-sw/LibericaNIK/releases/file.tar.gz",
            ),
            (
                "mandrel",
                "https://github.com/graalvm/mandrel/releases/file.tar.gz",
            ),
            ("microsoft", "https://aka.ms/download-jdk/jdk.tar.gz"),
            (
                "openlogic",
                "https://builds.openlogic.com/downloadJDK/jdk.tar.gz",
            ),
            ("openjdk", "https://download.java.net/java/GA/jdk.tar.gz"),
            ("oracle", "https://download.oracle.com/java/jdk.tar.gz"),
            (
                "sapmachine",
                "https://github.com/SAP/SapMachine/releases/file.tar.gz",
            ),
            (
                "semeru",
                "https://github.com/ibmruntimes/semeru-binaries/releases/file.tar.gz",
            ),
            (
                "semeru-certified",
                "https://github.com/ibmruntimes/semeru-certified/releases/file.tar.gz",
            ),
            (
                "temurin",
                "https://github.com/adoptium/temurin-binaries/releases/file.tar.gz",
            ),
            (
                "trava",
                "https://github.com/TravaOpenJDK/trava-jdk/releases/file.tar.gz",
            ),
            ("zulu", "https://cdn.azul.com/zulu/bin/jdk.tar.gz"),
            ("zulu-prime", "https://cdn.azul.com/prime/bin/jdk.tar.gz"),
        ];
        for (vendor, value) in trusted {
            let url = Url::parse(value).expect("URL");
            assert!(
                validate_initial_download_url(vendor, &url).is_ok(),
                "missing trust policy for {vendor}: {url}"
            );
        }
    }
}
