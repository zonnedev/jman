use std::{
    collections::BTreeSet,
    fs::File,
    io,
    path::{Component, Path, PathBuf},
};

use flate2::read::GzDecoder;
use fs2::FileExt;
use futures::StreamExt;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{fs, io::AsyncWriteExt};

use crate::{io_error, probe_javac, BuildError, Toolchain};

const API_ROOT: &str = "https://api.adoptium.net/v3";
const METADATA: &str = ".jman-toolchain.json";
const REMOTE_PAGE_SIZE: usize = 20;
const KNOWN_LTS_MAJORS: &[u16] = &[8, 11, 17, 21, 25];

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
}

#[must_use]
/// Return whether a Java feature release is a currently known LTS line.
pub fn is_lts_major(major: u16) -> bool {
    KNOWN_LTS_MAJORS.contains(&major)
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvailableJdk {
    pub vendor: String,
    pub version: String,
    pub major: u16,
    pub lts: bool,
    pub os: String,
    pub architecture: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovalResult {
    pub removed: Vec<ManagedJdk>,
    pub reclaimed_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct AvailableReleases {
    available_lts_releases: Vec<u16>,
}

#[derive(Debug, Deserialize)]
struct ReleaseNames {
    releases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LatestAsset {
    binary: Binary,
    release_name: String,
    version: Version,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    binaries: Vec<Binary>,
    release_name: String,
    version_data: Version,
}

#[derive(Debug, Deserialize)]
struct Binary {
    package: Package,
}

#[derive(Debug, Deserialize)]
struct Package {
    checksum: String,
    link: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Version {
    major: u16,
}

#[derive(Debug)]
struct ResolvedAsset {
    package: Package,
    version: String,
    major: u16,
}

pub struct ToolchainManager {
    root: PathBuf,
    client: reqwest::Client,
    api_root: Url,
}

impl ToolchainManager {
    /// Create a manager rooted in JMAN's shared cache.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be initialized.
    pub fn new(cache_dir: &Path) -> Result<Self, BuildError> {
        Self::with_api_root(cache_dir, API_ROOT)
    }

    fn with_api_root(cache_dir: &Path, api_root: &str) -> Result<Self, BuildError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("jman/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|error| {
                BuildError::Invalid(format!("could not create JDK client: {error}"))
            })?;
        let api_root = Url::parse(api_root)
            .map_err(|error| BuildError::Invalid(format!("invalid JDK API URL: {error}")))?;
        Ok(Self {
            root: cache_dir.join("jdks"),
            client,
            api_root,
        })
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

    /// List remotely installable Temurin JDK releases for this platform.
    ///
    /// Results are newest-first. A major filter limits the catalog to one Java
    /// feature release, while `lts_only` limits it to Adoptium's advertised LTS
    /// feature releases.
    ///
    /// # Errors
    ///
    /// Returns an error when the Adoptium catalog cannot be queried or decoded.
    pub async fn available(
        &self,
        major: Option<u16>,
        lts_only: bool,
    ) -> Result<Vec<AvailableJdk>, BuildError> {
        let mut info_url = self.api_root.clone();
        info_url
            .path_segments_mut()
            .map_err(|()| BuildError::Invalid("invalid JDK API root".to_owned()))?
            .extend(["info", "available_releases"]);
        let info = self
            .client
            .get(info_url)
            .send()
            .await
            .map_err(metadata_request_error)?
            .error_for_status()
            .map_err(metadata_request_error)?
            .json::<AvailableReleases>()
            .await
            .map_err(|error| BuildError::Invalid(format!("invalid JDK metadata: {error}")))?;
        let lts_majors = info
            .available_lts_releases
            .into_iter()
            .collect::<BTreeSet<_>>();

        let mut releases = Vec::new();
        for page in 0_usize.. {
            let names = self.release_names_page(page).await?;
            if names.is_empty() {
                break;
            }
            releases.extend(names);
        }
        let mut seen = BTreeSet::new();
        let mut jdks = releases
            .into_iter()
            .filter_map(|release| normalize_release_name(&release))
            .filter(|(_, release_major)| major.is_none_or(|major| *release_major == major))
            .filter(|(_, release_major)| !lts_only || lts_majors.contains(release_major))
            .filter(|(version, release_major)| seen.insert((*release_major, version.clone())))
            .map(|(version, release_major)| AvailableJdk {
                vendor: "temurin".to_owned(),
                version,
                major: release_major,
                lts: lts_majors.contains(&release_major),
                os: platform_os().to_owned(),
                architecture: platform_arch().to_owned(),
            })
            .collect::<Vec<_>>();
        jdks.sort_by(|left, right| {
            (right.major, version_key(&right.version))
                .cmp(&(left.major, version_key(&left.version)))
        });
        Ok(jdks)
    }

    async fn release_names_page(&self, page: usize) -> Result<Vec<String>, BuildError> {
        let mut url = self.api_root.clone();
        url.path_segments_mut()
            .map_err(|()| BuildError::Invalid("invalid JDK API root".to_owned()))?
            .extend(["info", "release_names"]);
        let page = page.to_string();
        let page_size = REMOTE_PAGE_SIZE.to_string();
        let response = self
            .client
            .get(url)
            .query(&[
                ("architecture", platform_arch()),
                ("heap_size", "normal"),
                ("image_type", "jdk"),
                ("jvm_impl", "hotspot"),
                ("os", platform_os()),
                ("page", page.as_str()),
                ("page_size", page_size.as_str()),
                ("project", "jdk"),
                ("release_type", "ga"),
                ("sort_method", "DATE"),
                ("sort_order", "DESC"),
                ("vendor", "eclipse"),
            ])
            .send()
            .await
            .map_err(metadata_request_error)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        let response = response
            .error_for_status()
            .map_err(metadata_request_error)?;
        response
            .json::<ReleaseNames>()
            .await
            .map(|response| response.releases)
            .map_err(|error| BuildError::Invalid(format!("invalid JDK metadata: {error}")))
    }

    /// Find an installed JDK satisfying a request.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid version or unreadable store.
    pub async fn find(&self, request: &ToolchainRequest) -> Result<Option<ManagedJdk>, BuildError> {
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

    /// Install the latest or exact requested Temurin JDK.
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
        if request.vendor != "temurin" {
            return Err(BuildError::Invalid(format!(
                "unsupported JDK vendor `{}`",
                request.vendor
            )));
        }
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
        let asset = self.resolve_asset(request).await?;
        let install_name = format!(
            "{}-{}-{}-{}",
            request.vendor,
            sanitize(&asset.version),
            platform_os(),
            platform_arch()
        );
        let destination = self.root.join(install_name);
        let archive = self.root.join(format!(".download-{}", asset.package.name));
        self.download_verified(&asset.package, &archive, progress)
            .await?;
        let extraction = self
            .root
            .join(format!(".extract-{}-{}", std::process::id(), asset.major));
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
            version: asset.version,
            major: asset.major,
            os: platform_os().to_owned(),
            architecture: platform_arch().to_owned(),
            checksum: format!("sha256:{}", asset.package.checksum),
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

    async fn resolve_asset(&self, request: &ToolchainRequest) -> Result<ResolvedAsset, BuildError> {
        let mut endpoint = self.api_root.clone();
        if request.exact() {
            endpoint
                .path_segments_mut()
                .map_err(|()| BuildError::Invalid("invalid JDK API root".to_owned()))?
                .extend(["assets", "version", &request.version]);
        } else {
            let major = request.major()?.to_string();
            endpoint
                .path_segments_mut()
                .map_err(|()| BuildError::Invalid("invalid JDK API root".to_owned()))?
                .extend(["assets", "latest", &major, "hotspot"]);
        }
        let response = self
            .client
            .get(endpoint)
            .query(&[
                ("architecture", platform_arch()),
                ("image_type", "jdk"),
                ("os", platform_os()),
                ("vendor", "eclipse"),
            ])
            .send()
            .await
            .map_err(metadata_request_error)?
            .error_for_status()
            .map_err(metadata_request_error)?;
        if request.exact() {
            let releases = response
                .json::<Vec<ReleaseAsset>>()
                .await
                .map_err(|error| BuildError::Invalid(format!("invalid JDK metadata: {error}")))?;
            let release = releases.into_iter().next().ok_or_else(|| {
                BuildError::Invalid(format!("no Temurin JDK found for {}", request.version))
            })?;
            let package = release.binaries.into_iter().next().ok_or_else(|| {
                BuildError::Invalid(format!("no Temurin JDK found for {}", request.version))
            })?;
            let (version, major) =
                normalize_release_name(&release.release_name).ok_or_else(|| {
                    BuildError::Invalid(format!(
                        "invalid JDK release name `{}`",
                        release.release_name
                    ))
                })?;
            debug_assert_eq!(major, release.version_data.major);
            Ok(ResolvedAsset {
                package: package.package,
                version,
                major,
            })
        } else {
            let assets = response
                .json::<Vec<LatestAsset>>()
                .await
                .map_err(|error| BuildError::Invalid(format!("invalid JDK metadata: {error}")))?;
            let asset = assets.into_iter().next().ok_or_else(|| {
                BuildError::Invalid(format!("no Temurin JDK found for {}", request.version))
            })?;
            let (version, major) =
                normalize_release_name(&asset.release_name).ok_or_else(|| {
                    BuildError::Invalid(format!(
                        "invalid JDK release name `{}`",
                        asset.release_name
                    ))
                })?;
            debug_assert_eq!(major, asset.version.major);
            Ok(ResolvedAsset {
                package: asset.binary.package,
                version,
                major,
            })
        }
    }

    async fn download_verified(
        &self,
        package: &Package,
        destination: &Path,
        progress: Option<&(dyn Fn(u64, Option<u64>) + Send + Sync)>,
    ) -> Result<(), BuildError> {
        let response = self
            .client
            .get(&package.link)
            .send()
            .await
            .map_err(|error| BuildError::Invalid(format!("JDK download failed: {error}")))?
            .error_for_status()
            .map_err(|error| BuildError::Invalid(format!("JDK download failed: {error}")))?;
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
        if actual != package.checksum {
            let _ = fs::remove_file(destination).await;
            return Err(BuildError::Invalid(format!(
                "JDK checksum mismatch: expected {}, received {actual}",
                package.checksum
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
    let store = ToolchainManager::new(cache_dir)?;
    if let Some(jdk) = store.find(request).await? {
        return managed_toolchain(jdk, request.major()?).await;
    }
    if auto_install {
        let jdk = store.install(request, offline).await?;
        return managed_toolchain(jdk, request.major()?).await;
    }
    Err(BuildError::Invalid(format!(
        "JDK {} is not installed; run `jman java install {}`",
        request.version, request.version
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

fn normalize_release_name(release: &str) -> Option<(String, u16)> {
    let version = release
        .strip_prefix("jdk-")
        .or_else(|| release.strip_prefix("jdk"))?;
    let major = version
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()?;
    Some((version.to_owned(), major))
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

fn metadata_request_error(error: reqwest::Error) -> BuildError {
    let message = error.to_string();
    drop(error);
    BuildError::Invalid(format!("JDK metadata request failed: {message}"))
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
        assert_eq!(
            normalize_release_name("jdk-21.0.12+8"),
            Some(("21.0.12+8".to_owned(), 21))
        );
        assert_eq!(
            normalize_release_name("jdk8u504-b01"),
            Some(("8u504-b01".to_owned(), 8))
        );
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

    #[tokio::test]
    async fn lists_remote_lts_catalog_and_stops_on_missing_page() {
        let (api_root, server) = catalog_server(vec![
            (
                "/v3/info/available_releases",
                200,
                r#"{"available_lts_releases":[8,21]}"#,
            ),
            (
                "/v3/info/release_names?",
                200,
                r#"{"releases":["jdk-22.0.1+8","jdk-21.0.2+13","jdk8u402-b06"]}"#,
            ),
            ("/v3/info/release_names?", 404, r#"{"error":"page"}"#),
        ]);
        let cache = tempfile::tempdir().expect("cache");
        let manager = ToolchainManager::with_api_root(cache.path(), &api_root).expect("manager");

        let available = manager.available(None, true).await.expect("catalog");

        assert_eq!(
            available
                .iter()
                .map(|jdk| (jdk.version.as_str(), jdk.major, jdk.lts))
                .collect::<Vec<_>>(),
            [("21.0.2+13", 21, true), ("8u402-b06", 8, true)]
        );
        server.join().expect("catalog server");
    }

    #[tokio::test]
    async fn resolves_latest_and_exact_adoptium_response_shapes() {
        let (api_root, server) = catalog_server(vec![
            (
                "/v3/assets/latest/21/hotspot?",
                200,
                r#"[{"binary":{"package":{"checksum":"latest","link":"https://example.test/latest.tar.gz","name":"latest.tar.gz"}},"release_name":"jdk-21.0.12.1+1","version":{"major":21}}]"#,
            ),
            (
                "/v3/assets/version/8u504-b01?",
                200,
                r#"[{"binaries":[{"package":{"checksum":"exact","link":"https://example.test/exact.tar.gz","name":"exact.tar.gz"}}],"release_name":"jdk8u504-b01","version_data":{"major":8}}]"#,
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

        assert_eq!((latest.version.as_str(), latest.major), ("21.0.12.1+1", 21));
        assert_eq!((exact.version.as_str(), exact.major), ("8u504-b01", 8));
        server.join().expect("asset server");
    }

    #[tokio::test]
    async fn removes_only_newest_major_match_and_reports_size() {
        let cache = tempfile::tempdir().expect("cache");
        let manager = ToolchainManager::new(cache.path()).expect("manager");
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

    fn catalog_server(
        responses: Vec<(&'static str, u16, &'static str)>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("catalog listener");
        let address = listener.local_addr().expect("catalog address");
        let server = thread::spawn(move || {
            for (expected_path, status, body) in responses {
                let (mut connection, _) = listener.accept().expect("catalog connection");
                let mut request = [0_u8; 4096];
                let read = connection.read(&mut request).expect("catalog request");
                let request = String::from_utf8_lossy(&request[..read]);
                assert!(request.starts_with("GET "));
                assert!(
                    request.contains(expected_path),
                    "unexpected request: {request}"
                );
                let reason = if status == 200 { "OK" } else { "Not Found" };
                write!(
                    connection,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .expect("catalog response");
            }
        });
        (format!("http://{address}/v3"), server)
    }
}
