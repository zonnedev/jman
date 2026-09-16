use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

use sha2::{Digest, Sha256};

use crate::{Coordinate, PomSource, ResolverError};

const CENTRAL: &str = "https://repo.maven.apache.org/maven2";

#[derive(Clone, Debug)]
pub struct CachedFile {
    pub path: PathBuf,
    pub checksum: String,
    pub bytes: Vec<u8>,
    pub source: String,
}

#[derive(Clone)]
pub struct RepositoryClient {
    client: reqwest::Client,
    cache_dir: PathBuf,
    repositories: Vec<String>,
    offline: bool,
    memory_cache: Arc<tokio::sync::RwLock<HashMap<String, CachedFile>>>,
    fetch_locks: Arc<tokio::sync::RwLock<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl RepositoryClient {
    /// Create a repository client with an explicit cache and ordered repositories.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be initialized.
    pub fn new(cache_dir: PathBuf, repositories: Vec<String>) -> Result<Self, ResolverError> {
        Self::new_with_mode(cache_dir, repositories, false)
    }

    /// Create a repository client with explicit offline behavior.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be initialized.
    pub fn new_with_mode(
        cache_dir: PathBuf,
        repositories: Vec<String>,
        offline: bool,
    ) -> Result<Self, ResolverError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("jman/", env!("CARGO_PKG_VERSION")))
            .pool_max_idle_per_host(16)
            .build()
            .map_err(|source| ResolverError::Request {
                url: "<client initialization>".to_owned(),
                source,
            })?;
        let repositories = if repositories.is_empty() {
            vec![CENTRAL.to_owned()]
        } else {
            repositories
        };
        Ok(Self {
            client,
            cache_dir,
            repositories,
            offline,
            memory_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            fetch_locks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        })
    }

    /// Create a repository client using the platform cache directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be initialized.
    pub fn with_default_cache(repositories: Vec<String>) -> Result<Self, ResolverError> {
        Self::with_default_cache_mode(repositories, false)
    }

    /// Create a platform-cache client with explicit offline behavior.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository client cannot be initialized.
    pub fn with_default_cache_mode(
        repositories: Vec<String>,
        offline: bool,
    ) -> Result<Self, ResolverError> {
        let cache_dir = std::env::var_os("JMAN_CACHE_DIR").map_or_else(
            || {
                dirs::cache_dir()
                    .unwrap_or_else(|| PathBuf::from(".jman-cache"))
                    .join("jman")
            },
            PathBuf::from,
        );
        Self::new_with_mode(cache_dir, repositories, offline)
    }

    #[must_use]
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Fetch and verify an artifact through the local and content-addressed caches.
    ///
    /// # Errors
    ///
    /// Returns an error for repository, HTTP, or cache failures.
    pub async fn fetch(&self, coordinate: &Coordinate) -> Result<CachedFile, ResolverError> {
        let repository_path = coordinate.repository_path();
        if let Some(cached) = self.memory_cache.read().await.get(&repository_path) {
            return Ok(cached.clone());
        }
        let fetch_lock = {
            let mut locks = self.fetch_locks.write().await;
            locks
                .entry(repository_path.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _fetch_guard = fetch_lock.lock().await;
        if let Some(cached) = self.memory_cache.read().await.get(&repository_path) {
            return Ok(cached.clone());
        }
        let fetched = self.fetch_uncached(coordinate, &repository_path).await?;
        self.memory_cache
            .write()
            .await
            .insert(repository_path, fetched.clone());
        Ok(fetched)
    }

    async fn fetch_uncached(
        &self,
        coordinate: &Coordinate,
        repository_path: &str,
    ) -> Result<CachedFile, ResolverError> {
        let metadata_path = self.cache_dir.join("repository").join(repository_path);
        if let Ok(bytes) = tokio::fs::read(&metadata_path).await {
            let checksum = sha256(&bytes);
            let path = self
                .content_addressed_path(coordinate, &checksum, &bytes)
                .await?;
            return Ok(CachedFile {
                checksum,
                bytes,
                path,
                source: self
                    .repositories
                    .first()
                    .map_or_else(String::new, Clone::clone),
            });
        }

        for repository in &self.repositories {
            let mut bytes = self
                .fetch_repository_bytes(repository, repository_path)
                .await?;
            if bytes.is_none() && coordinate.version.ends_with("-SNAPSHOT") {
                if let Some(snapshot_path) = self
                    .snapshot_repository_path(repository, coordinate)
                    .await?
                {
                    bytes = self
                        .fetch_repository_bytes(repository, &snapshot_path)
                        .await?;
                }
            }
            if let Some(bytes) = bytes {
                write_atomic(&metadata_path, &bytes).await?;
                let checksum = sha256(&bytes);
                let path = self
                    .content_addressed_path(coordinate, &checksum, &bytes)
                    .await?;
                return Ok(CachedFile {
                    checksum,
                    bytes,
                    path,
                    source: repository.clone(),
                });
            }
        }

        if self.offline {
            return Err(ResolverError::OfflineMiss(coordinate.to_string()));
        }
        Err(ResolverError::HttpStatus {
            url: coordinate.repository_path(),
            status: reqwest::StatusCode::NOT_FOUND,
        })
    }

    async fn fetch_repository_bytes(
        &self,
        repository: &str,
        repository_path: &str,
    ) -> Result<Option<Vec<u8>>, ResolverError> {
        if let Some(root) = repository.strip_prefix("file://") {
            let source_path = Path::new(root).join(repository_path);
            return match tokio::fs::read(&source_path).await {
                Ok(bytes) => Ok(Some(bytes)),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(source) => Err(ResolverError::Cache {
                    path: source_path,
                    source,
                }),
            };
        }
        if self.offline {
            return Ok(None);
        }
        let url = format!("{}/{}", repository.trim_end_matches('/'), repository_path);
        for attempt in 0..3 {
            match self.client.get(&url).send().await {
                Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
                    return Ok(None);
                }
                Ok(response) if response.status().is_success() => {
                    return response
                        .bytes()
                        .await
                        .map(|bytes| Some(bytes.to_vec()))
                        .map_err(|source| ResolverError::Request {
                            url: url.clone(),
                            source,
                        });
                }
                Ok(response)
                    if (response.status().is_server_error()
                        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS)
                        && attempt < 2 =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(50 << attempt)).await;
                }
                Ok(response) => {
                    return Err(ResolverError::HttpStatus {
                        url,
                        status: response.status(),
                    });
                }
                Err(_source) if attempt < 2 => {
                    tokio::time::sleep(std::time::Duration::from_millis(50 << attempt)).await;
                }
                Err(source) => return Err(ResolverError::Request { url, source }),
            }
        }
        unreachable!("the retry loop returns on its final attempt")
    }

    async fn snapshot_repository_path(
        &self,
        repository: &str,
        coordinate: &Coordinate,
    ) -> Result<Option<String>, ResolverError> {
        let group_path = coordinate.group.replace('.', "/");
        let metadata_relative = format!(
            "{group_path}/{}/{}/maven-metadata.xml",
            coordinate.artifact, coordinate.version
        );
        let cached_metadata = self.cache_dir.join("repository").join(&metadata_relative);
        let bytes = if let Ok(bytes) = tokio::fs::read(&cached_metadata).await {
            bytes
        } else if let Some(bytes) = self
            .fetch_repository_bytes(repository, &metadata_relative)
            .await?
        {
            write_atomic(&cached_metadata, &bytes).await?;
            bytes
        } else {
            return Ok(None);
        };
        let xml = std::str::from_utf8(&bytes)
            .map_err(|error| ResolverError::InvalidPom(error.to_string()))?;
        let value = snapshot_value(xml, coordinate)?;
        Ok(value.map(|value| {
            let classifier = coordinate
                .classifier
                .as_ref()
                .map_or_else(String::new, |classifier| format!("-{classifier}"));
            format!(
                "{group_path}/{artifact}/{version}/{artifact}-{value}{classifier}.{extension}",
                artifact = coordinate.artifact,
                version = coordinate.version,
                extension = coordinate.extension
            )
        }))
    }

    async fn content_addressed_path(
        &self,
        coordinate: &Coordinate,
        checksum: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, ResolverError> {
        if coordinate.extension == "pom" {
            return Ok(self
                .cache_dir
                .join("repository")
                .join(coordinate.repository_path()));
        }
        let digest = checksum.strip_prefix("sha256:").unwrap_or(checksum);
        let path = self
            .cache_dir
            .join("artifacts")
            .join(format!("{digest}.{}", coordinate.extension));
        let valid = match tokio::fs::read(&path).await {
            Ok(existing) => sha256(&existing) == checksum,
            Err(_) => false,
        };
        if !valid {
            write_atomic(&path, bytes).await?;
        }
        Ok(path)
    }
}

impl PomSource for RepositoryClient {
    fn load_pom(
        &self,
        coordinate: &Coordinate,
    ) -> Pin<Box<dyn Future<Output = Result<String, ResolverError>> + Send + '_>> {
        let coordinate = Coordinate::pom(
            coordinate.group.clone(),
            coordinate.artifact.clone(),
            coordinate.version.clone(),
        );
        Box::pin(async move {
            let cached = self.fetch(&coordinate).await?;
            String::from_utf8(cached.bytes)
                .map_err(|error| ResolverError::InvalidPom(error.to_string()))
        })
    }
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ResolverError> {
    let parent = path.parent().ok_or_else(|| ResolverError::Cache {
        path: path.to_owned(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "cache path has no parent"),
    })?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|source| ResolverError::Cache {
            path: parent.to_owned(),
            source,
        })?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    tokio::fs::write(&temporary, bytes)
        .await
        .map_err(|source| ResolverError::Cache {
            path: temporary.clone(),
            source,
        })?;
    match tokio::fs::rename(&temporary, path).await {
        Ok(()) => Ok(()),
        Err(_source) if path.exists() => {
            let _ = tokio::fs::remove_file(&temporary).await;
            Ok(())
        }
        Err(source) => Err(ResolverError::Cache {
            path: path.to_owned(),
            source,
        }),
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn snapshot_value(xml: &str, coordinate: &Coordinate) -> Result<Option<String>, ResolverError> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| ResolverError::InvalidPom(error.to_string()))?;
    for snapshot_version in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "snapshotVersion")
    {
        let extension = xml_child_text(snapshot_version, "extension");
        let classifier = xml_child_text(snapshot_version, "classifier");
        if extension.as_deref() == Some(&coordinate.extension)
            && classifier.as_deref() == coordinate.classifier.as_deref()
        {
            return Ok(xml_child_text(snapshot_version, "value"));
        }
    }
    let snapshot = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "snapshot");
    let timestamp = snapshot.and_then(|node| xml_child_text(node, "timestamp"));
    let build_number = snapshot.and_then(|node| xml_child_text(node, "buildNumber"));
    Ok(timestamp.zip(build_number).map(|(timestamp, build)| {
        format!(
            "{}-{timestamp}-{build}",
            coordinate.version.trim_end_matches("-SNAPSHOT")
        )
    }))
}

fn xml_child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
        .and_then(|child| child.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[tokio::test]
    async fn concurrent_fetches_share_one_content_addressed_artifact() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let repository = temporary.path().join("repository");
        let artifact = repository.join("org/example/library/1/library-1.jar");
        fs::create_dir_all(artifact.parent().expect("artifact parent"))
            .expect("repository directories");
        fs::write(&artifact, b"library").expect("artifact fixture");
        let client = RepositoryClient::new(
            temporary.path().join("cache"),
            vec![format!("file://{}", repository.display())],
        )
        .expect("repository client");
        let coordinate = Coordinate::jar("org.example", "library", "1");

        let (first, second) = tokio::join!(client.fetch(&coordinate), client.fetch(&coordinate));
        let first = first.expect("first fetch");
        let second = second.expect("second fetch");

        assert_eq!(first.checksum, second.checksum);
        assert_eq!(first.path, second.path);
        assert_eq!(fs::read(first.path).expect("cached artifact"), b"library");
    }

    #[tokio::test]
    async fn offline_mode_reports_a_coordinate_specific_cache_miss() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let client = RepositoryClient::new_with_mode(
            temporary.path().join("cache"),
            vec!["https://repo.invalid".to_owned()],
            true,
        )
        .expect("repository client");

        let error = client
            .fetch(&Coordinate::jar("org.example", "missing", "1"))
            .await
            .expect_err("offline cache miss");

        assert!(matches!(
            error,
            ResolverError::OfflineMiss(coordinate)
                if coordinate == "org.example:missing:1"
        ));
    }

    #[tokio::test]
    async fn resolves_timestamped_snapshot_artifacts_from_maven_metadata() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let repository = temporary
            .path()
            .join("repository/org/example/library/1-SNAPSHOT");
        fs::create_dir_all(&repository).expect("snapshot directory");
        fs::write(
            repository.join("maven-metadata.xml"),
            "<metadata><versioning><snapshotVersions><snapshotVersion>\
             <extension>jar</extension><value>1-20260727.120000-4</value>\
             </snapshotVersion></snapshotVersions></versioning></metadata>",
        )
        .expect("snapshot metadata");
        fs::write(
            repository.join("library-1-20260727.120000-4.jar"),
            b"snapshot",
        )
        .expect("snapshot artifact");
        let client = RepositoryClient::new(
            temporary.path().join("cache"),
            vec![format!(
                "file://{}",
                temporary.path().join("repository").display()
            )],
        )
        .expect("repository client");

        let artifact = client
            .fetch(&Coordinate::jar("org.example", "library", "1-SNAPSHOT"))
            .await
            .expect("timestamped snapshot");

        assert_eq!(artifact.bytes, b"snapshot");
        assert!(artifact.path.is_file());
    }

    #[tokio::test]
    async fn repairs_a_corrupted_content_addressed_artifact_from_repository_cache() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let repository = temporary.path().join("repository");
        let artifact = repository.join("org/example/library/1/library-1.jar");
        fs::create_dir_all(artifact.parent().expect("artifact parent"))
            .expect("repository directories");
        fs::write(&artifact, b"library").expect("artifact fixture");
        let cache = temporary.path().join("cache");
        let repositories = vec![format!("file://{}", repository.display())];
        let coordinate = Coordinate::jar("org.example", "library", "1");
        let first = RepositoryClient::new(cache.clone(), repositories.clone())
            .expect("repository client")
            .fetch(&coordinate)
            .await
            .expect("initial fetch");
        fs::write(&first.path, b"corrupt").expect("corrupt CAS entry");
        fs::remove_file(&artifact).expect("remove source repository artifact");

        let repaired = RepositoryClient::new_with_mode(cache, repositories, true)
            .expect("offline repository client")
            .fetch(&coordinate)
            .await
            .expect("offline repair");

        assert_eq!(repaired.bytes, b"library");
        assert_eq!(fs::read(repaired.path).expect("repaired CAS"), b"library");
    }
}
