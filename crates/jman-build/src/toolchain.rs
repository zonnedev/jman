use std::{
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
            .split('.')
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| BuildError::Invalid(format!("invalid JDK version `{}`", self.version)))
    }

    fn exact(&self) -> bool {
        self.version.contains('.') || self.version.contains('+')
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovalResult {
    pub removed: Vec<ManagedJdk>,
    pub reclaimed_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct Asset {
    binary: Binary,
    version: Version,
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
    semver: String,
}

pub struct ToolchainManager {
    root: PathBuf,
    client: reqwest::Client,
}

impl ToolchainManager {
    /// Create a manager rooted in JMAN's shared cache.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be initialized.
    pub fn new(cache_dir: &Path) -> Result<Self, BuildError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("jman/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|error| {
                BuildError::Invalid(format!("could not create JDK client: {error}"))
            })?;
        Ok(Self {
            root: cache_dir.join("jdks"),
            client,
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
            if let Ok(jdk) = serde_json::from_slice::<ManagedJdk>(&bytes) {
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
            sanitize(&asset.version.semver),
            platform_os(),
            platform_arch()
        );
        let destination = self.root.join(install_name);
        let archive = self
            .root
            .join(format!(".download-{}", asset.binary.package.name));
        self.download_verified(&asset.binary.package, &archive, progress)
            .await?;
        let extraction = self.root.join(format!(
            ".extract-{}-{}",
            std::process::id(),
            asset.version.major
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
            version: asset.version.semver,
            major: asset.version.major,
            os: platform_os().to_owned(),
            architecture: platform_arch().to_owned(),
            checksum: format!("sha256:{}", asset.binary.package.checksum),
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

    async fn resolve_asset(&self, request: &ToolchainRequest) -> Result<Asset, BuildError> {
        let endpoint = if request.exact() {
            let mut url = Url::parse(API_ROOT)
                .map_err(|error| BuildError::Invalid(format!("invalid JDK API URL: {error}")))?;
            url.path_segments_mut()
                .map_err(|()| BuildError::Invalid("invalid JDK API root".to_owned()))?
                .extend(["assets", "version", &request.version]);
            url
        } else {
            Url::parse(&format!(
                "{API_ROOT}/assets/latest/{}/hotspot",
                request.major()?
            ))
            .map_err(|error| BuildError::Invalid(format!("invalid JDK API URL: {error}")))?
        };
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
            .map_err(|error| BuildError::Invalid(format!("JDK metadata request failed: {error}")))?
            .error_for_status()
            .map_err(|error| {
                BuildError::Invalid(format!("JDK metadata request failed: {error}"))
            })?;
        let assets = response
            .json::<Vec<Asset>>()
            .await
            .map_err(|error| BuildError::Invalid(format!("invalid JDK metadata: {error}")))?;
        assets.into_iter().next().ok_or_else(|| {
            BuildError::Invalid(format!("no Temurin JDK found for {}", request.version))
        })
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
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
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
    }

    #[test]
    fn sanitizes_versions_for_install_directories() {
        assert_eq!(sanitize("17.0.20+8"), "17.0.20_8");
        assert!(version_key("17.0.20+8") > version_key("17.0.9+9"));
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
}
