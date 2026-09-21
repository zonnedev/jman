use std::{
    fs::File,
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use flate2::read::GzDecoder;
use fs2::FileExt;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{fs, io::AsyncWriteExt};

use crate::{io_error, probe_javac, BuildError, Toolchain};

mod catalog;

use catalog::{
    validate_initial_download_url, validate_redirect_download_url, CatalogRelease,
    CatalogValidators, FoojayProvider, JdkCatalogProvider, Platform, ProviderCatalog,
    ResolvedArtifact,
};

const METADATA: &str = ".jman-toolchain.json";
const CATALOG_CACHE_SCHEMA: u32 = 2;
const CATALOG_CACHE_TTL: Duration = Duration::from_mins(15);
const KNOWN_LTS_MAJORS: &[u16] = &[8, 11, 17, 21, 25];

/// User-wide Java configuration stored under JMAN's configuration directory.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserJavaConfig {
    pub java: JavaSelections,
}

/// Java selections belonging to the current user.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct JavaSelections {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<GlobalJavaSelection>,
}

/// An exact JDK installation selected as the user-wide default.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalJavaSelection {
    pub jdk: String,
    pub vendor: String,
}

/// Return JMAN's shared cache directory.
#[must_use]
pub fn default_cache_dir() -> PathBuf {
    std::env::var_os("JMAN_CACHE_DIR").map_or_else(
        || {
            dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".jman-cache"))
                .join("jman")
        },
        PathBuf::from,
    )
}

/// Return JMAN's durable user-data directory.
#[must_use]
pub fn default_data_dir() -> PathBuf {
    std::env::var_os("JMAN_DATA_DIR").map_or_else(
        || {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from(".jman-data"))
                .join("jman")
        },
        PathBuf::from,
    )
}

/// Return JMAN's user configuration directory.
#[must_use]
pub fn default_config_dir() -> PathBuf {
    std::env::var_os("JMAN_CONFIG_DIR").map_or_else(
        || {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from(".jman-config"))
                .join("jman")
        },
        PathBuf::from,
    )
}

/// Return the user-wide configuration file.
#[must_use]
pub fn default_config_file() -> PathBuf {
    default_config_dir().join("config.toml")
}

/// Read a user Java configuration file. A missing file is an empty configuration.
///
/// # Errors
///
/// Returns an error when an existing file cannot be read or parsed.
pub fn read_user_config(path: &Path) -> Result<UserJavaConfig, BuildError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(UserJavaConfig::default());
        }
        Err(source) => return Err(io_error(path, source)),
    };
    toml::from_str(&text).map_err(|error| {
        BuildError::Invalid(format!(
            "could not parse user configuration {}: {error}",
            path.display()
        ))
    })
}

/// Read the configured user-wide Java selection.
///
/// # Errors
///
/// Returns an error when the user configuration cannot be read or parsed.
pub fn global_java_selection() -> Result<Option<GlobalJavaSelection>, BuildError> {
    Ok(read_user_config(&default_config_file())?.java.global)
}

/// Encode user Java configuration as deterministic TOML.
///
/// # Errors
///
/// Returns an error when serialization fails.
pub fn user_config_toml(config: &UserJavaConfig) -> Result<String, BuildError> {
    toml::to_string_pretty(config).map_err(|error| {
        BuildError::Invalid(format!("could not encode user configuration: {error}"))
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolchainRequest {
    pub version: String,
    pub vendor: String,
}

impl ToolchainRequest {
    /// Return the requested JDK major.
    ///
    /// # Errors
    ///
    /// Returns an error when the version has no numeric major.
    pub fn major(&self) -> Result<u16, BuildError> {
        self.version
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .ok()
            .ok_or_else(|| BuildError::Invalid(format!("invalid JDK version `{}`", self.version)))
    }

    fn exact(&self) -> bool {
        !self
            .version
            .chars()
            .all(|character| character.is_ascii_digit())
    }

    fn validate_vendor(&self) -> Result<(), BuildError> {
        if !self.vendor.is_empty()
            && self
                .vendor
                .bytes()
                .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'-')
            && !self.vendor.starts_with('-')
            && !self.vendor.ends_with('-')
        {
            Ok(())
        } else {
            Err(BuildError::Invalid(format!(
                "invalid JDK distribution `{}`",
                self.vendor
            )))
        }
    }
}

#[must_use]
/// Return whether a Java feature release is a currently known LTS line.
pub fn is_lts_major(major: u16) -> bool {
    KNOWN_LTS_MAJORS.contains(&major)
}

/// Normalize user-facing distribution aliases into JMAN's stable identifiers.
#[must_use]
pub fn normalize_vendor(vendor: &str) -> String {
    match vendor
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-")
        .as_str()
    {
        "amzn" | "amazon" | "cor" | "corretto" => "corretto".to_owned(),
        "lib" | "liberica" => "liberica".to_owned(),
        "ms" | "microsoft" => "microsoft".to_owned(),
        "open" | "ojdk" | "openjdk" | "oracle-open-jdk" => "openjdk".to_owned(),
        "sap" | "sap-machine" | "sapmachine" => "sapmachine".to_owned(),
        "tem" | "temurin" => "temurin".to_owned(),
        other => other.to_owned(),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedJdk {
    pub vendor: String,
    pub version: String,
    pub major: u16,
    pub os: String,
    pub architecture: String,
    pub checksum: String,
    pub home: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AvailableJdk {
    pub vendor: String,
    pub version: String,
    pub major: u16,
    pub lts: bool,
    pub os: String,
    pub architecture: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogSource {
    Network,
    Cache,
    Revalidated,
    StaleCache,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogStatus {
    pub provider: String,
    pub source: CatalogSource,
    pub fetched_at: u64,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvailableCatalog {
    pub jdks: Vec<AvailableJdk>,
    pub status: CatalogStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovalResult {
    pub removed: Vec<ManagedJdk>,
    pub reclaimed_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogCache {
    schema: u32,
    fetched_at: u64,
    provider: String,
    os: String,
    architecture: String,
    validators: CatalogValidators,
    releases: Vec<CatalogRelease>,
}

pub struct ToolchainManager {
    root: PathBuf,
    catalog_root: PathBuf,
    client: reqwest::Client,
    provider: Box<dyn JdkCatalogProvider>,
}

impl ToolchainManager {
    /// Create a manager using JMAN's durable JDK store and the supplied cache.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be initialized.
    pub fn new(cache_dir: &Path) -> Result<Self, BuildError> {
        Self::new_with_data(cache_dir, &default_data_dir())
    }

    /// Create a manager with explicit cache and durable-data roots.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be initialized.
    pub fn new_with_data(cache_dir: &Path, data_dir: &Path) -> Result<Self, BuildError> {
        Self::with_provider(cache_dir, data_dir, Box::new(FoojayProvider::new()?))
    }

    #[cfg(test)]
    fn with_api_root(cache_dir: &Path, api_root: &str) -> Result<Self, BuildError> {
        Self::with_provider(
            cache_dir,
            cache_dir,
            Box::new(FoojayProvider::with_api_root(api_root)?),
        )
    }

    fn with_provider(
        cache_dir: &Path,
        data_dir: &Path,
        provider: Box<dyn JdkCatalogProvider>,
    ) -> Result<Self, BuildError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("jman/", env!("CARGO_PKG_VERSION")))
            // Redirects are followed manually so every hop crosses the same
            // distribution-scoped domain trust policy.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| {
                BuildError::Invalid(format!("could not create JDK client: {error}"))
            })?;
        Ok(Self {
            root: data_dir.join("jdks"),
            catalog_root: cache_dir.join("catalog"),
            client,
            provider,
        })
    }

    /// Return the durable directory containing managed JDK installations.
    #[must_use]
    pub fn installation_root(&self) -> &Path {
        &self.root
    }

    /// List valid JMAN-managed JDK installations.
    ///
    /// # Errors
    ///
    /// Returns an error when the toolchain store cannot be read.
    pub async fn list(&self) -> Result<Vec<ManagedJdk>, BuildError> {
        if !self.root.is_dir() {
            return Ok(Vec::new());
        }
        let mut entries = fs::read_dir(&self.root)
            .await
            .map_err(|source| io_error(&self.root, source))?;
        let mut jdks = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|source| io_error(&self.root, source))?
        {
            let metadata = entry.path().join(METADATA);
            if !metadata.is_file() {
                continue;
            }
            let bytes = fs::read(&metadata)
                .await
                .map_err(|source| io_error(&metadata, source))?;
            if let Ok(mut jdk) = serde_json::from_slice::<ManagedJdk>(&bytes) {
                jdk.version = normalize_managed_version(&jdk.version, jdk.major);
                if jdk.home == entry.path() && javac_path(&jdk.home).is_file() {
                    jdks.push(jdk);
                }
            }
        }
        jdks.sort_by(|left, right| {
            (&left.vendor, left.major, version_key(&left.version)).cmp(&(
                &right.vendor,
                right.major,
                version_key(&right.version),
            ))
        });
        Ok(jdks)
    }

    /// List remotely installable JDK releases for this platform.
    ///
    /// Results are newest-first. A major filter limits the catalog to one Java
    /// feature release, while `lts_only` limits it to provider-advertised LTS
    /// releases.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured catalog cannot be queried or decoded.
    pub async fn available(
        &self,
        major: Option<u16>,
        lts_only: bool,
        refresh: bool,
    ) -> Result<AvailableCatalog, BuildError> {
        self.available_at(major, lts_only, refresh, unix_time())
            .await
    }

    async fn available_at(
        &self,
        major: Option<u16>,
        lts_only: bool,
        refresh: bool,
        now: u64,
    ) -> Result<AvailableCatalog, BuildError> {
        let cached = self.read_catalog_cache().await?;
        if !refresh
            && cached
                .as_ref()
                .is_some_and(|cache| catalog_is_fresh(cache, now))
        {
            return Ok(catalog_result(
                cached.as_ref().expect("fresh cache was checked"),
                major,
                lts_only,
                CatalogSource::Cache,
                None,
            ));
        }

        match self.fetch_catalog(cached.as_ref(), now).await {
            Ok(cache) => {
                let source = if cached.is_some() {
                    CatalogSource::Revalidated
                } else {
                    CatalogSource::Network
                };
                let warning = self
                    .write_catalog_cache(&cache)
                    .await
                    .err()
                    .map(|error| format!("could not update JDK catalog cache: {error}"));
                Ok(catalog_result(&cache, major, lts_only, source, warning))
            }
            Err(error) => {
                let Some(cache) = cached else {
                    return Err(error);
                };
                Ok(catalog_result(
                    &cache,
                    major,
                    lts_only,
                    CatalogSource::StaleCache,
                    Some(format!(
                        "remote JDK catalog unavailable; using cached data: {error}"
                    )),
                ))
            }
        }
    }

    async fn fetch_catalog(
        &self,
        cached: Option<&CatalogCache>,
        now: u64,
    ) -> Result<CatalogCache, BuildError> {
        match self
            .provider
            .fetch_catalog(
                &self.client,
                current_platform(),
                cached.map(|cache| &cache.validators),
            )
            .await?
        {
            ProviderCatalog::NotModified => {
                let mut cache = cached.cloned().ok_or_else(|| {
                    BuildError::Invalid(
                        "JDK catalog returned 304 without cached metadata".to_owned(),
                    )
                })?;
                cache.fetched_at = now;
                Ok(cache)
            }
            ProviderCatalog::Modified {
                validators,
                releases,
            } => Ok(CatalogCache {
                schema: CATALOG_CACHE_SCHEMA,
                fetched_at: now,
                provider: self.provider.id().to_owned(),
                os: platform_os().to_owned(),
                architecture: platform_arch().to_owned(),
                validators,
                releases,
            }),
        }
    }

    async fn read_catalog_cache(&self) -> Result<Option<CatalogCache>, BuildError> {
        let path = self.catalog_cache_path();
        let bytes = match fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io_error(&path, source)),
        };
        let Ok(cache) = serde_json::from_slice::<CatalogCache>(&bytes) else {
            return Ok(None);
        };
        if cache.schema != CATALOG_CACHE_SCHEMA
            || cache.provider != self.provider.id()
            || cache.os != platform_os()
            || cache.architecture != platform_arch()
        {
            return Ok(None);
        }
        Ok(Some(cache))
    }

    async fn write_catalog_cache(&self, cache: &CatalogCache) -> Result<(), BuildError> {
        fs::create_dir_all(&self.catalog_root)
            .await
            .map_err(|source| io_error(&self.catalog_root, source))?;
        let path = self.catalog_cache_path();
        let temporary = self
            .catalog_root
            .join(format!(".catalog-{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(cache).map_err(|error| {
            BuildError::Invalid(format!("could not encode JDK catalog: {error}"))
        })?;
        fs::write(&temporary, bytes)
            .await
            .map_err(|source| io_error(&temporary, source))?;
        if let Err(source) = fs::rename(&temporary, &path).await {
            if path.exists() {
                fs::remove_file(&path)
                    .await
                    .map_err(|remove| io_error(&path, remove))?;
                fs::rename(&temporary, &path)
                    .await
                    .map_err(|rename| io_error(&path, rename))?;
            } else {
                return Err(io_error(&path, source));
            }
        }
        Ok(())
    }

    fn catalog_cache_path(&self) -> PathBuf {
        self.catalog_root.join(format!(
            "{}-{}-{}.json",
            self.provider.id(),
            platform_os(),
            platform_arch()
        ))
    }

    /// Find an installed JDK satisfying a request.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid version or unreadable store.
    pub async fn find(&self, request: &ToolchainRequest) -> Result<Option<ManagedJdk>, BuildError> {
        request.validate_vendor()?;
        let major = request.major()?;
        Ok(self.list().await?.into_iter().rev().find(|jdk| {
            jdk.vendor == request.vendor
                && jdk.major == major
                && (!request.exact() || jdk.version == request.version)
        }))
    }

    /// Select the installations that a remove operation would affect.
    ///
    /// A major-only request selects the newest matching JDK unless `all` is
    /// true. An exact request selects only that exact installation.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid version or unreadable toolchain store.
    pub async fn removal_candidates(
        &self,
        request: &ToolchainRequest,
        all: bool,
    ) -> Result<Vec<ManagedJdk>, BuildError> {
        request.validate_vendor()?;
        let major = request.major()?;
        let mut candidates = self
            .list()
            .await?
            .into_iter()
            .filter(|jdk| {
                jdk.vendor == request.vendor
                    && jdk.major == major
                    && (!request.exact() || jdk.version == request.version)
            })
            .collect::<Vec<_>>();
        if !all && !request.exact() {
            Ok(candidates.pop().into_iter().collect())
        } else {
            Ok(candidates)
        }
    }

    /// Remove selected JMAN-managed JDK installations.
    ///
    /// Targets are renamed inside the toolchain store before deletion so a
    /// failure while staging multiple targets can be rolled back.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid selection, unsafe paths, lock failures, or
    /// filesystem failures.
    pub async fn remove(
        &self,
        request: &ToolchainRequest,
        all: bool,
    ) -> Result<RemovalResult, BuildError> {
        fs::create_dir_all(&self.root)
            .await
            .map_err(|source| io_error(&self.root, source))?;
        let _lock = acquire_install_lock(self.lock_path(request)?).await?;
        let candidates = self.removal_candidates(request, all).await?;
        if candidates.is_empty() {
            return Err(BuildError::Invalid(format!(
                "no JMAN-managed {} JDK {} installation was found",
                request.vendor, request.version
            )));
        }
        let mut reclaimed_bytes = 0_u64;
        for jdk in &candidates {
            if jdk.home.parent() != Some(self.root.as_path()) {
                return Err(BuildError::Invalid(format!(
                    "refusing to remove JDK outside {}",
                    self.root.display()
                )));
            }
            reclaimed_bytes = reclaimed_bytes.saturating_add(directory_size(&jdk.home).await?);
        }
        let mut staged = Vec::with_capacity(candidates.len());
        for (index, jdk) in candidates.iter().enumerate() {
            let temporary = self
                .root
                .join(format!(".remove-{}-{index}", std::process::id()));
            if temporary.exists() {
                rollback_staged(&staged).await;
                return Err(BuildError::Invalid(format!(
                    "stale JDK removal path exists at {}",
                    temporary.display()
                )));
            }
            if let Err(source) = fs::rename(&jdk.home, &temporary).await {
                rollback_staged(&staged).await;
                return Err(io_error(&jdk.home, source));
            }
            staged.push((temporary, jdk.home.clone()));
        }
        for (index, (temporary, _)) in staged.iter().enumerate() {
            if let Err(source) = fs::remove_dir_all(temporary).await {
                rollback_staged(&staged[index..]).await;
                return Err(io_error(temporary, source));
            }
        }
        Ok(RemovalResult {
            removed: candidates,
            reclaimed_bytes,
        })
    }

    /// Install the latest or exact requested JDK distribution.
    ///
    /// # Errors
    ///
    /// Returns an error for offline misses, network or checksum failures, and
    /// invalid archives.
    pub async fn install(
        &self,
        request: &ToolchainRequest,
        offline: bool,
    ) -> Result<ManagedJdk, BuildError> {
        self.install_with_progress(request, offline, None).await
    }

    /// Install a JDK while reporting downloaded and total bytes.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::install`].
    pub async fn install_with_progress(
        &self,
        request: &ToolchainRequest,
        offline: bool,
        progress: Option<&(dyn Fn(u64, Option<u64>) + Send + Sync)>,
    ) -> Result<ManagedJdk, BuildError> {
        if let Some(jdk) = self.find(request).await? {
            return Ok(jdk);
        }
        if offline {
            return Err(BuildError::Invalid(format!(
                "JDK {} is not installed and offline mode forbids downloading it",
                request.version
            )));
        }
        fs::create_dir_all(&self.root)
            .await
            .map_err(|source| io_error(&self.root, source))?;
        let lock_path = self.lock_path(request)?;
        let _lock = acquire_install_lock(lock_path).await?;
        if let Some(jdk) = self.find(request).await? {
            return Ok(jdk);
        }
        let artifact = self.resolve_asset(request).await?;
        let install_name = format!(
            "{}-{}-{}-{}",
            request.vendor,
            sanitize(&artifact.release.version),
            platform_os(),
            platform_arch()
        );
        let destination = self.root.join(install_name);
        let archive = self.root.join(format!(
            ".download-{}-{}-{}",
            artifact.release.vendor,
            sanitize(&artifact.release.source_id),
            sanitize(&artifact.release.filename)
        ));
        self.download_verified(&artifact, &archive, progress)
            .await?;
        let extraction = self.root.join(format!(
            ".extract-{}-{}-{}",
            std::process::id(),
            artifact.release.vendor,
            artifact.release.major
        ));
        if extraction.exists() {
            fs::remove_dir_all(&extraction)
                .await
                .map_err(|source| io_error(&extraction, source))?;
        }
        fs::create_dir_all(&extraction)
            .await
            .map_err(|source| io_error(&extraction, source))?;
        extract_archive(&archive, &extraction).await?;
        let mut home = single_root(&extraction).await?;
        if !javac_path(&home).is_file() && javac_path(&home.join("Contents/Home")).is_file() {
            home = home.join("Contents/Home");
        }
        if !javac_path(&home).is_file() {
            return Err(BuildError::Invalid(format!(
                "downloaded JDK archive has no compiler at {}",
                javac_path(&home).display()
            )));
        }
        if destination.exists() {
            fs::remove_dir_all(&destination)
                .await
                .map_err(|source| io_error(&destination, source))?;
        }
        fs::rename(&home, &destination)
            .await
            .map_err(|source| io_error(&destination, source))?;
        let _ = fs::remove_dir_all(&extraction).await;
        let _ = fs::remove_file(&archive).await;
        let jdk = ManagedJdk {
            vendor: request.vendor.clone(),
            version: artifact.release.version,
            major: artifact.release.major,
            os: platform_os().to_owned(),
            architecture: platform_arch().to_owned(),
            checksum: format!("sha256:{}", artifact.checksum_sha256),
            home: destination,
        };
        let metadata = serde_json::to_vec_pretty(&jdk)
            .map_err(|error| BuildError::Invalid(format!("could not record JDK: {error}")))?;
        fs::write(jdk.home.join(METADATA), metadata)
            .await
            .map_err(|source| io_error(&jdk.home.join(METADATA), source))?;
        Ok(jdk)
    }

    fn lock_path(&self, request: &ToolchainRequest) -> Result<PathBuf, BuildError> {
        Ok(self.root.join(format!(
            ".install-{}-{}-{}.lock",
            request.vendor,
            request.major()?,
            platform_arch()
        )))
    }

    async fn resolve_asset(
        &self,
        request: &ToolchainRequest,
    ) -> Result<ResolvedArtifact, BuildError> {
        self.provider
            .resolve(
                &self.client,
                current_platform(),
                &request.vendor,
                &request.version,
                request.exact(),
            )
            .await
    }

    async fn download_verified(
        &self,
        artifact: &ResolvedArtifact,
        destination: &Path,
        progress: Option<&(dyn Fn(u64, Option<u64>) + Send + Sync)>,
    ) -> Result<(), BuildError> {
        let mut url = artifact.download_url.clone();
        validate_initial_download_url(&artifact.release.vendor, &url)?;
        let mut redirects = 0_u8;
        let response = loop {
            let response =
                self.client.get(url.clone()).send().await.map_err(|error| {
                    BuildError::Invalid(format!("JDK download failed: {error}"))
                })?;
            if !response.status().is_redirection() {
                break response.error_for_status().map_err(|error| {
                    BuildError::Invalid(format!("JDK download failed: {error}"))
                })?;
            }
            redirects = redirects.saturating_add(1);
            if redirects > 10 {
                return Err(BuildError::Invalid(
                    "JDK download exceeded 10 redirects".to_owned(),
                ));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    BuildError::Invalid("JDK download redirect had no valid location".to_owned())
                })?;
            url = url.join(location).map_err(|error| {
                BuildError::Invalid(format!("invalid JDK download redirect: {error}"))
            })?;
            validate_redirect_download_url(&artifact.release.vendor, &url)?;
        };
        let total = response.content_length();
        if let Some(progress) = progress {
            progress(0, total);
        }
        let mut file = fs::File::create(destination)
            .await
            .map_err(|source| io_error(destination, source))?;
        let mut hash = Sha256::new();
        let mut downloaded = 0_u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|error| BuildError::Invalid(format!("JDK download failed: {error}")))?;
            hash.update(&chunk);
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            file.write_all(&chunk)
                .await
                .map_err(|source| io_error(destination, source))?;
            if let Some(progress) = progress {
                progress(downloaded, total);
            }
        }
        file.flush()
            .await
            .map_err(|source| io_error(destination, source))?;
        let actual = hex::encode(hash.finalize());
        if actual != artifact.checksum_sha256 {
            let _ = fs::remove_file(destination).await;
            return Err(BuildError::Invalid(format!(
                "JDK checksum mismatch: expected {}, received {actual}",
                artifact.checksum_sha256
            )));
        }
        Ok(())
    }
}

/// Resolve a managed toolchain, optionally installing it.
///
/// # Errors
///
/// Returns an error when no matching JDK exists or installation/probing fails.
pub async fn resolve(
    request: &ToolchainRequest,
    cache_dir: &Path,
    auto_install: bool,
    offline: bool,
) -> Result<Toolchain, BuildError> {
    let request = ToolchainRequest {
        version: request.version.clone(),
        vendor: normalize_vendor(&request.vendor),
    };
    let store = ToolchainManager::new(cache_dir)?;
    if let Some(jdk) = store.find(&request).await? {
        return managed_toolchain(jdk, request.major()?).await;
    }
    if auto_install {
        let jdk = store.install(&request, offline).await?;
        return managed_toolchain(jdk, request.major()?).await;
    }
    Err(BuildError::Invalid(format!(
        "{} JDK {} is not installed; run `jman java install {} --vendor {}`",
        request.vendor, request.version, request.version, request.vendor
    )))
}

/// Prefer a JMAN-managed JDK for the requested major, then use `JAVA_HOME`/PATH.
///
/// # Errors
///
/// Returns an error when the managed store or external compiler cannot be used.
pub async fn resolve_default(
    requested_release: u16,
    cache_dir: &Path,
) -> Result<Toolchain, BuildError> {
    if let Some(selection) = global_java_selection()? {
        let request = ToolchainRequest {
            version: selection.jdk,
            vendor: normalize_vendor(&selection.vendor),
        };
        let store = ToolchainManager::new(cache_dir)?;
        let Some(jdk) = store.find(&request).await? else {
            return Err(BuildError::Invalid(format!(
                "globally selected {} JDK {} is not installed; run `jman java install {} --vendor {} --global`",
                request.vendor, request.version, request.version, request.vendor
            )));
        };
        return managed_toolchain(jdk, requested_release).await;
    }
    let request = ToolchainRequest {
        version: requested_release.to_string(),
        vendor: "temurin".to_owned(),
    };
    let store = ToolchainManager::new(cache_dir)?;
    if let Some(jdk) = store.find(&request).await? {
        managed_toolchain(jdk, requested_release).await
    } else {
        crate::discover_toolchain(requested_release).await
    }
}

async fn managed_toolchain(
    jdk: ManagedJdk,
    requested_release: u16,
) -> Result<Toolchain, BuildError> {
    let mut toolchain = probe_javac(javac_path(&jdk.home), requested_release).await?;
    toolchain.managed = Some(jdk);
    Ok(toolchain)
}

#[must_use]
pub fn javac_path(home: &Path) -> PathBuf {
    home.join("bin")
        .join(if cfg!(windows) { "javac.exe" } else { "javac" })
}

async fn extract_archive(archive: &Path, destination: &Path) -> Result<(), BuildError> {
    let archive = archive.to_owned();
    let destination = destination.to_owned();
    tokio::task::spawn_blocking(move || {
        let name = archive.to_string_lossy();
        if name.ends_with(".zip") {
            extract_zip(&archive, &destination)
        } else {
            let file = File::open(&archive).map_err(|source| io_error(&archive, source))?;
            let mut archive = tar::Archive::new(GzDecoder::new(file));
            archive
                .unpack(&destination)
                .map_err(|source| io_error(&destination, source))
        }
    })
    .await
    .map_err(|error| BuildError::Invalid(format!("JDK extraction task failed: {error}")))?
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<(), BuildError> {
    let file = File::open(archive).map_err(|source| io_error(archive, source))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| BuildError::Invalid(format!("invalid JDK ZIP: {error}")))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| BuildError::Invalid(format!("invalid JDK ZIP entry: {error}")))?;
        let relative = entry.enclosed_name().ok_or_else(|| {
            BuildError::Invalid(format!("unsafe path in JDK ZIP: {}", entry.name()))
        })?;
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(BuildError::Invalid("unsafe path in JDK ZIP".to_owned()));
        }
        let output = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&output).map_err(|source| io_error(&output, source))?;
        } else {
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
            }
            let mut file = File::create(&output).map_err(|source| io_error(&output, source))?;
            io::copy(&mut entry, &mut file).map_err(|source| io_error(&output, source))?;
        }
    }
    Ok(())
}

async fn single_root(directory: &Path) -> Result<PathBuf, BuildError> {
    let mut entries = fs::read_dir(directory)
        .await
        .map_err(|source| io_error(directory, source))?;
    let first = entries
        .next_entry()
        .await
        .map_err(|source| io_error(directory, source))?
        .ok_or_else(|| BuildError::Invalid("JDK archive was empty".to_owned()))?;
    if entries
        .next_entry()
        .await
        .map_err(|source| io_error(directory, source))?
        .is_some()
    {
        return Err(BuildError::Invalid(
            "JDK archive must contain one root directory".to_owned(),
        ));
    }
    Ok(first.path())
}

fn platform_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn platform_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        "arm" => "arm",
        "powerpc64" => "ppc64le",
        "s390x" => "s390x",
        "riscv64" => "riscv64",
        other => other,
    }
}

fn platform_libc() -> &'static str {
    if cfg!(target_os = "linux") {
        if cfg!(target_env = "musl") {
            "musl"
        } else {
            "glibc"
        }
    } else {
        "libc"
    }
}

fn platform_archive_type() -> &'static str {
    if cfg!(windows) {
        "zip"
    } else {
        "tar.gz"
    }
}

fn current_platform() -> Platform<'static> {
    Platform {
        os: platform_os(),
        architecture: platform_arch(),
        libc: platform_libc(),
        archive_type: platform_archive_type(),
    }
}

fn catalog_result(
    cache: &CatalogCache,
    major: Option<u16>,
    lts_only: bool,
    source: CatalogSource,
    warning: Option<String>,
) -> AvailableCatalog {
    let mut jdks = cache
        .releases
        .iter()
        .filter(|release| major.is_none_or(|major| release.major == major))
        .filter(|release| !lts_only || release.lts)
        .map(|release| AvailableJdk {
            vendor: release.vendor.clone(),
            version: release.version.clone(),
            major: release.major,
            lts: release.lts,
            os: release.os.clone(),
            architecture: release.architecture.clone(),
        })
        .collect::<Vec<_>>();
    jdks.sort_by(|left, right| {
        left.vendor.cmp(&right.vendor).then_with(|| {
            (right.major, version_key(&right.version))
                .cmp(&(left.major, version_key(&left.version)))
        })
    });
    AvailableCatalog {
        jdks,
        status: CatalogStatus {
            provider: cache.provider.clone(),
            source,
            fetched_at: cache.fetched_at,
            warning,
        },
    }
}

fn catalog_is_fresh(cache: &CatalogCache, now: u64) -> bool {
    now.saturating_sub(cache.fetched_at) <= CATALOG_CACHE_TTL.as_secs()
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn normalize_managed_version(version: &str, major: u16) -> String {
    if major == 8 {
        let numbers = version_key(version);
        if numbers.len() >= 5 && numbers[0] == 8 {
            return format!("8u{}-b{:02}", numbers[2], numbers[4]);
        }
    }
    let Some(without_lts) = version.strip_suffix(".0.LTS") else {
        return version.to_owned();
    };
    let Some((core, encoded_build)) = without_lts.rsplit_once('+') else {
        return version.to_owned();
    };
    let Ok(encoded_build) = encoded_build.parse::<u32>() else {
        return version.to_owned();
    };
    if encoded_build >= 100 {
        format!("{core}.{}+{}", encoded_build / 100, encoded_build % 100)
    } else {
        format!("{core}+{encoded_build}")
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn version_key(value: &str) -> Vec<u32> {
    let (core, suffix) = value.split_once('+').unwrap_or((value, ""));
    let mut key = core
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect::<Vec<_>>();
    key.resize(4, 0);
    key.extend(
        suffix
            .split(|character: char| !character.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u32>().ok()),
    );
    key
}

async fn acquire_install_lock(path: PathBuf) -> Result<File, BuildError> {
    tokio::task::spawn_blocking(move || {
        let lock = File::create(&path).map_err(|source| io_error(&path, source))?;
        lock.lock_exclusive()
            .map_err(|source| io_error(&path, source))?;
        Ok(lock)
    })
    .await
    .map_err(|error| BuildError::Invalid(format!("JDK install lock task failed: {error}")))?
}

async fn directory_size(root: &Path) -> Result<u64, BuildError> {
    let mut pending = vec![root.to_owned()];
    let mut size = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory)
            .await
            .map_err(|source| io_error(&directory, source))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|source| io_error(&directory, source))?
        {
            let metadata = fs::symlink_metadata(entry.path())
                .await
                .map_err(|source| io_error(&entry.path(), source))?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                size = size.saturating_add(metadata.len());
            }
        }
    }
    Ok(size)
}

async fn rollback_staged(staged: &[(PathBuf, PathBuf)]) {
    for (temporary, original) in staged.iter().rev() {
        let _ = fs::rename(temporary, original).await;
    }
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
    fn parses_major_and_distinguishes_exact_versions() {
        let major = ToolchainRequest {
            version: "17".to_owned(),
            vendor: "temurin".to_owned(),
        };
        let exact = ToolchainRequest {
            version: "17.0.20+8".to_owned(),
            vendor: "temurin".to_owned(),
        };
        assert_eq!(major.major().expect("major"), 17);
        assert!(!major.exact());
        assert_eq!(exact.major().expect("major"), 17);
        assert!(exact.exact());

        let java_8 = ToolchainRequest {
            version: "8u504-b01".to_owned(),
            vendor: "temurin".to_owned(),
        };
        assert_eq!(java_8.major().expect("major"), 8);
        assert!(java_8.exact());
    }

    #[test]
    fn sanitizes_versions_for_install_directories() {
        assert_eq!(sanitize("17.0.20+8"), "17.0.20_8");
        assert!(version_key("17.0.20+8") > version_key("17.0.9+9"));
        assert!(version_key("21.0.12+8") > version_key("21+35"));
        assert_eq!(normalize_vendor("oracle_open_jdk"), "openjdk");
        assert_eq!(normalize_vendor("tem"), "temurin");
        assert!(ToolchainRequest {
            version: "21".to_owned(),
            vendor: "../../outside".to_owned(),
        }
        .validate_vendor()
        .is_err());
        assert_eq!(sanitize("../../evil.tar.gz"), ".._.._evil.tar.gz");
        assert_eq!(
            normalize_managed_version("21.0.12+101.0.LTS", 21),
            "21.0.12.1+1"
        );
        assert_eq!(
            normalize_managed_version("21.0.12+8.0.LTS", 21),
            "21.0.12+8"
        );
        assert_eq!(normalize_managed_version("8.0.504+1", 8), "8u504-b01");
    }

    struct StaticProvider;

    impl JdkCatalogProvider for StaticProvider {
        fn id(&self) -> &'static str {
            "static-test"
        }

        fn fetch_catalog<'a>(
            &'a self,
            _client: &'a reqwest::Client,
            platform: Platform<'a>,
            _validators: Option<&'a CatalogValidators>,
        ) -> catalog::ProviderFuture<'a, ProviderCatalog> {
            Box::pin(async move {
                Ok(ProviderCatalog::Modified {
                    validators: CatalogValidators {
                        etag: None,
                        last_modified: None,
                    },
                    releases: vec![CatalogRelease {
                        vendor: "test-jdk".to_owned(),
                        version: "21.0.1+1".to_owned(),
                        major: 21,
                        lts: true,
                        os: platform.os.to_owned(),
                        architecture: platform.architecture.to_owned(),
                        archive_type: platform.archive_type.to_owned(),
                        filename: "test-jdk.tar.gz".to_owned(),
                        source_id: "opaque-test-id".to_owned(),
                    }],
                })
            })
        }

        fn resolve<'a>(
            &'a self,
            _client: &'a reqwest::Client,
            _platform: Platform<'a>,
            _vendor: &'a str,
            _version: &'a str,
            _exact: bool,
        ) -> catalog::ProviderFuture<'a, ResolvedArtifact> {
            Box::pin(async {
                Err(BuildError::Invalid(
                    "static provider does not resolve downloads".to_owned(),
                ))
            })
        }
    }

    #[tokio::test]
    async fn manager_consumes_the_provider_neutral_catalog_contract() {
        let cache = tempfile::tempdir().expect("cache");
        let manager =
            ToolchainManager::with_provider(cache.path(), cache.path(), Box::new(StaticProvider))
                .expect("manager");

        let available = manager
            .available(None, false, false)
            .await
            .expect("catalog");

        assert_eq!(available.status.provider, "static-test");
        assert_eq!(available.jdks[0].vendor, "test-jdk");
        assert_eq!(available.jdks[0].version, "21.0.1+1");
        let stored = manager
            .read_catalog_cache()
            .await
            .expect("cache read")
            .expect("catalog cache");
        assert_eq!(stored.provider, "static-test");
        assert_eq!(stored.releases[0].source_id, "opaque-test-id");
    }

    #[tokio::test]
    async fn lists_provider_neutral_multivendor_lts_catalog() {
        let body = foojay_packages(&[
            foojay_package("tem-22", "temurin", 22, "22.0.1+8", "sts"),
            foojay_package("open-21", "oracle_open_jdk", 21, "21.0.2+13", "lts"),
            foojay_package("zulu-8", "zulu", 8, "8.0.402+6", "lts"),
        ]);
        let (api_root, server) = catalog_server(vec![MockHttpResponse {
            expected_path: "/v3/packages?",
            expected_if_none_match: None,
            expected_if_modified_since: None,
            status: 200,
            etag: Some("\"catalog-v1\""),
            last_modified: Some("Fri, 18 Sep 2026 10:00:00 GMT"),
            body,
        }]);
        let cache = tempfile::tempdir().expect("cache");
        let manager = ToolchainManager::with_api_root(cache.path(), &api_root).expect("manager");

        let available = manager.available(None, true, false).await.expect("catalog");

        assert_eq!(
            available
                .jdks
                .iter()
                .map(|jdk| (jdk.vendor.as_str(), jdk.version.as_str(), jdk.lts))
                .collect::<Vec<_>>(),
            [("openjdk", "21.0.2+13", true), ("zulu", "8u402-b06", true)]
        );
        assert_eq!(available.status.provider, "foojay");
        assert_eq!(available.status.source, CatalogSource::Network);
        server.join().expect("catalog server");
        assert!(manager.catalog_cache_path().is_file());
        let stored = manager
            .read_catalog_cache()
            .await
            .expect("read cache")
            .expect("stored cache");
        assert_eq!(stored.validators.etag.as_deref(), Some("\"catalog-v1\""));
        assert_eq!(stored.releases.len(), 3);

        let cached = manager
            .available_at(None, true, false, available.status.fetched_at + 1)
            .await
            .expect("fresh cached catalog");
        assert_eq!(cached.status.source, CatalogSource::Cache);
    }

    #[tokio::test]
    async fn refresh_revalidates_catalog_with_http_validators() {
        let (api_root, server) = catalog_server(vec![MockHttpResponse {
            expected_path: "/v3/packages?",
            expected_if_none_match: Some("\"catalog-v1\""),
            expected_if_modified_since: Some("Fri, 18 Sep 2026 10:00:00 GMT"),
            status: 304,
            etag: None,
            last_modified: None,
            body: String::new(),
        }]);
        let cache_directory = tempfile::tempdir().expect("cache");
        let manager =
            ToolchainManager::with_api_root(cache_directory.path(), &api_root).expect("manager");
        manager
            .write_catalog_cache(&cached_catalog(100))
            .await
            .expect("seed cache");

        let available = manager
            .available_at(Some(21), false, true, 101)
            .await
            .expect("revalidated catalog");

        assert_eq!(available.status.source, CatalogSource::Revalidated);
        assert_eq!(available.status.fetched_at, 101);
        assert_eq!(available.jdks[0].version, "21.0.2+13");
        server.join().expect("catalog server");
    }

    #[tokio::test]
    async fn network_failure_falls_back_to_stale_catalog() {
        let cache_directory = tempfile::tempdir().expect("cache");
        let listener = TcpListener::bind("127.0.0.1:0").expect("unused listener");
        let address = listener.local_addr().expect("unused address");
        drop(listener);
        let manager = ToolchainManager::with_api_root(
            cache_directory.path(),
            &format!("http://{address}/v3"),
        )
        .expect("manager");
        manager
            .write_catalog_cache(&cached_catalog(100))
            .await
            .expect("seed cache");

        let available = manager
            .available_at(None, false, false, 100 + CATALOG_CACHE_TTL.as_secs() + 1)
            .await
            .expect("stale fallback");

        assert_eq!(available.status.source, CatalogSource::StaleCache);
        assert!(available.status.warning.is_some());
        assert_eq!(available.jdks.len(), 2);
    }

    #[tokio::test]
    async fn corrupt_cache_is_ignored_and_replaced_from_network() {
        let (api_root, server) = catalog_server(vec![mock_response(
            "/v3/packages?",
            200,
            foojay_packages(&[foojay_package("tem-21", "temurin", 21, "21.0.2+13", "lts")]),
        )]);
        let cache_directory = tempfile::tempdir().expect("cache");
        let manager =
            ToolchainManager::with_api_root(cache_directory.path(), &api_root).expect("manager");
        fs::create_dir_all(&manager.catalog_root)
            .await
            .expect("catalog directory");
        fs::write(manager.catalog_cache_path(), b"not json")
            .await
            .expect("corrupt cache");

        let available = manager
            .available_at(None, false, false, 500)
            .await
            .expect("network catalog");

        assert_eq!(available.status.source, CatalogSource::Network);
        assert_eq!(available.jdks.len(), 1);
        assert!(manager.read_catalog_cache().await.expect("cache").is_some());
        server.join().expect("catalog server");
    }

    #[tokio::test]
    async fn resolves_latest_and_exact_foojay_packages_with_sha256() {
        let checksum = "a".repeat(64);
        let (api_root, server) = catalog_server(vec![
            mock_response(
                "/v3/packages?",
                200,
                foojay_packages(&[foojay_package(
                    "latest",
                    "temurin",
                    21,
                    "21.0.12.1+1",
                    "lts",
                )]),
            ),
            mock_response(
                "/v3/ids/latest",
                200,
                foojay_info(
                    "https://github.com/adoptium/temurin21-binaries/releases/download/jdk/archive.tar.gz",
                    &checksum,
                ),
            ),
            mock_response(
                "/v3/packages?",
                200,
                foojay_packages(&[foojay_package(
                    "exact",
                    "temurin",
                    8,
                    "8.0.504+1",
                    "lts",
                )]),
            ),
            mock_response(
                "/v3/ids/exact",
                200,
                foojay_info(
                    "https://github.com/adoptium/temurin8-binaries/releases/download/jdk/archive.tar.gz",
                    &checksum,
                ),
            ),
        ]);
        let cache = tempfile::tempdir().expect("cache");
        let manager = ToolchainManager::with_api_root(cache.path(), &api_root).expect("manager");

        let latest = manager
            .resolve_asset(&ToolchainRequest {
                version: "21".to_owned(),
                vendor: "temurin".to_owned(),
            })
            .await
            .expect("latest asset");
        let exact = manager
            .resolve_asset(&ToolchainRequest {
                version: "8u504-b01".to_owned(),
                vendor: "temurin".to_owned(),
            })
            .await
            .expect("exact asset");

        assert_eq!(
            (latest.release.version.as_str(), latest.release.major),
            ("21.0.12.1+1", 21)
        );
        assert_eq!(
            (exact.release.version.as_str(), exact.release.major),
            ("8u504-b01", 8)
        );
        assert_eq!(exact.checksum_sha256, checksum);
        server.join().expect("asset server");
    }

    #[tokio::test]
    async fn removes_only_newest_major_match_and_reports_size() {
        let cache = tempfile::tempdir().expect("cache");
        let manager = ToolchainManager::new_with_data(cache.path(), cache.path()).expect("manager");
        let older = fake_install(cache.path(), "17.0.9+9", 17, 7).await;
        let newer = fake_install(cache.path(), "17.0.20+8", 17, 11).await;
        let request = ToolchainRequest {
            version: "17".to_owned(),
            vendor: "temurin".to_owned(),
        };

        let result = manager.remove(&request, false).await.expect("remove");

        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].version, "17.0.20+8");
        assert!(result.reclaimed_bytes >= 11);
        assert!(older.is_dir());
        assert!(!newer.exists());
    }

    #[test]
    fn user_java_configuration_round_trips_exact_global_selection() {
        let directory = tempfile::tempdir().expect("configuration directory");
        let path = directory.path().join("config.toml");
        assert_eq!(
            read_user_config(&path).expect("missing configuration"),
            UserJavaConfig::default()
        );

        let config = UserJavaConfig {
            java: JavaSelections {
                global: Some(GlobalJavaSelection {
                    jdk: "21.0.8+9".to_owned(),
                    vendor: "temurin".to_owned(),
                }),
            },
        };
        let text = user_config_toml(&config).expect("configuration TOML");
        assert_eq!(
            text,
            "[java.global]\njdk = \"21.0.8+9\"\nvendor = \"temurin\"\n"
        );
        std::fs::write(&path, text).expect("write configuration");
        assert_eq!(read_user_config(&path).expect("read configuration"), config);
    }

    #[test]
    fn user_java_configuration_rejects_unknown_fields() {
        let directory = tempfile::tempdir().expect("configuration directory");
        let path = directory.path().join("config.toml");
        std::fs::write(
            &path,
            "[java.global]\njdk = \"21\"\nvendor = \"temurin\"\nunsafe = true\n",
        )
        .expect("write configuration");

        let error = read_user_config(&path).expect_err("unknown field must fail");
        assert!(error.to_string().contains("unknown field `unsafe`"));
    }

    async fn fake_install(cache: &Path, version: &str, major: u16, payload_size: usize) -> PathBuf {
        let home = cache
            .join("jdks")
            .join(format!("temurin-{}", sanitize(version)));
        fs::create_dir_all(home.join("bin")).await.expect("JDK bin");
        fs::write(javac_path(&home), b"compiler")
            .await
            .expect("javac");
        fs::write(home.join("payload"), vec![0; payload_size])
            .await
            .expect("payload");
        let metadata = ManagedJdk {
            vendor: "temurin".to_owned(),
            version: version.to_owned(),
            major,
            os: platform_os().to_owned(),
            architecture: platform_arch().to_owned(),
            checksum: "sha256:test".to_owned(),
            home: home.clone(),
        };
        fs::write(
            home.join(METADATA),
            serde_json::to_vec(&metadata).expect("metadata"),
        )
        .await
        .expect("metadata file");
        home
    }

    struct MockHttpResponse {
        expected_path: &'static str,
        expected_if_none_match: Option<&'static str>,
        expected_if_modified_since: Option<&'static str>,
        status: u16,
        etag: Option<&'static str>,
        last_modified: Option<&'static str>,
        body: String,
    }

    fn mock_response(
        expected_path: &'static str,
        status: u16,
        body: impl Into<String>,
    ) -> MockHttpResponse {
        MockHttpResponse {
            expected_path,
            expected_if_none_match: None,
            expected_if_modified_since: None,
            status,
            etag: None,
            last_modified: None,
            body: body.into(),
        }
    }

    fn foojay_package(
        id: &str,
        distribution: &str,
        major: u16,
        version: &str,
        support: &str,
    ) -> String {
        format!(
            r#"{{"id":"{id}","archive_type":"{}","distribution":"{distribution}","major_version":{major},"java_version":"{version}","release_status":"ga","term_of_support":"{support}","operating_system":"{}","lib_c_type":"{}","architecture":"{}","package_type":"jdk","javafx_bundled":false,"directly_downloadable":true,"filename":"{distribution}-{major}.{}","free_use_in_production":true}}"#,
            platform_archive_type(),
            if platform_os() == "mac" {
                "macos"
            } else {
                platform_os()
            },
            platform_libc(),
            platform_arch(),
            platform_archive_type(),
        )
    }

    fn foojay_packages(packages: &[String]) -> String {
        format!(r#"{{"result":[{}]}}"#, packages.join(","))
    }

    fn foojay_info(url: &str, checksum: &str) -> String {
        format!(
            r#"{{"result":[{{"direct_download_uri":"{url}","checksum":"{checksum}","checksum_type":"sha256"}}]}}"#
        )
    }

    fn cached_catalog(fetched_at: u64) -> CatalogCache {
        CatalogCache {
            schema: CATALOG_CACHE_SCHEMA,
            fetched_at,
            provider: "foojay".to_owned(),
            os: platform_os().to_owned(),
            architecture: platform_arch().to_owned(),
            validators: CatalogValidators {
                etag: Some("\"catalog-v1\"".to_owned()),
                last_modified: Some("Fri, 18 Sep 2026 10:00:00 GMT".to_owned()),
            },
            releases: vec![
                CatalogRelease {
                    vendor: "temurin".to_owned(),
                    version: "22.0.1+8".to_owned(),
                    major: 22,
                    lts: false,
                    os: platform_os().to_owned(),
                    architecture: platform_arch().to_owned(),
                    archive_type: platform_archive_type().to_owned(),
                    filename: "temurin-22.tar.gz".to_owned(),
                    source_id: "tem-22".to_owned(),
                },
                CatalogRelease {
                    vendor: "openjdk".to_owned(),
                    version: "21.0.2+13".to_owned(),
                    major: 21,
                    lts: true,
                    os: platform_os().to_owned(),
                    architecture: platform_arch().to_owned(),
                    archive_type: platform_archive_type().to_owned(),
                    filename: "openjdk-21.tar.gz".to_owned(),
                    source_id: "open-21".to_owned(),
                },
            ],
        }
    }

    fn catalog_server(responses: Vec<MockHttpResponse>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("catalog listener");
        let address = listener.local_addr().expect("catalog address");
        let server = thread::spawn(move || {
            for response in responses {
                let (mut connection, _) = listener.accept().expect("catalog connection");
                let mut request = [0_u8; 4096];
                let read = connection.read(&mut request).expect("catalog request");
                let request = String::from_utf8_lossy(&request[..read]);
                assert!(request.starts_with("GET "));
                assert!(
                    request.contains(response.expected_path),
                    "unexpected request: {request}"
                );
                if let Some(etag) = response.expected_if_none_match {
                    assert!(
                        request.to_ascii_lowercase().contains(&format!(
                            "\r\nif-none-match: {}\r\n",
                            etag.to_ascii_lowercase()
                        )),
                        "missing If-None-Match header: {request}"
                    );
                }
                if let Some(modified) = response.expected_if_modified_since {
                    assert!(
                        request.to_ascii_lowercase().contains(&format!(
                            "\r\nif-modified-since: {}\r\n",
                            modified.to_ascii_lowercase()
                        )),
                        "missing If-Modified-Since header: {request}"
                    );
                }
                let reason = match response.status {
                    200 => "OK",
                    304 => "Not Modified",
                    _ => "Not Found",
                };
                let etag = response
                    .etag
                    .map_or_else(String::new, |etag| format!("ETag: {etag}\r\n"));
                let last_modified = response
                    .last_modified
                    .map_or_else(String::new, |value| format!("Last-Modified: {value}\r\n"));
                write!(
                    connection,
                    "HTTP/1.1 {} {reason}\r\n{etag}{last_modified}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status,
                    response.body.len(),
                    response.body,
                )
                .expect("catalog response");
            }
        });
        (format!("http://{address}/v3"), server)
    }
}
