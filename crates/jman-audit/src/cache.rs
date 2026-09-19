use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AuditError, AuditPackage, AuditResult, AuditSource, Finding, VulnerabilityProvider};

const CACHE_SCHEMA_VERSION: u32 = 1;
const DEFAULT_TTL: Duration = Duration::from_hours(1);
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct AuditOptions {
    pub cache_dir: PathBuf,
    pub offline: bool,
    pub refresh: bool,
    pub ttl: Duration,
}

impl AuditOptions {
    #[must_use]
    pub fn platform_default(offline: bool, refresh: bool) -> Self {
        let cache_dir = std::env::var_os("JMAN_CACHE_DIR").map_or_else(
            || {
                dirs::cache_dir()
                    .unwrap_or_else(|| PathBuf::from(".jman-cache"))
                    .join("jman")
            },
            PathBuf::from,
        );
        Self {
            cache_dir,
            offline,
            refresh,
            ttl: DEFAULT_TTL,
        }
    }
}

#[derive(Debug)]
pub struct Auditor<P> {
    provider: P,
    options: AuditOptions,
}

impl<P: VulnerabilityProvider> Auditor<P> {
    #[must_use]
    pub fn new(provider: P, options: AuditOptions) -> Self {
        Self { provider, options }
    }

    /// Audit an exact set of resolved packages.
    ///
    /// # Errors
    ///
    /// Returns an error when neither the provider nor a valid cache can supply
    /// a result.
    pub async fn audit(&self, packages: &[AuditPackage]) -> Result<AuditResult, AuditError> {
        let mut packages = packages.to_vec();
        packages.sort();
        packages.dedup();
        let cache_path = self.cache_path(&packages);
        let cached = read_cache(&cache_path).await;
        if self.options.offline {
            let cache = cached?.ok_or(AuditError::OfflineMiss)?;
            return Ok(cache.into_result(AuditSource::Cache));
        }
        if !self.options.refresh {
            if let Ok(Some(cache)) = &cached {
                if cache.is_fresh(self.options.ttl) {
                    return Ok(cache.clone().into_result(AuditSource::Cache));
                }
            }
        }
        match self.provider.query(&packages).await {
            Ok(mut findings) => {
                findings.sort_by(|left, right| {
                    (&left.package, &left.advisory.id).cmp(&(&right.package, &right.advisory.id))
                });
                let cache = CachedAudit {
                    schema_version: CACHE_SCHEMA_VERSION,
                    fetched_at: unix_timestamp(),
                    provider: self.provider.name().to_owned(),
                    findings,
                };
                write_cache(&cache_path, &cache).await?;
                Ok(cache.into_result(AuditSource::Network))
            }
            Err(error) => match cached {
                Ok(Some(cache)) => Ok(cache.into_result(AuditSource::StaleCache)),
                _ => Err(error),
            },
        }
    }

    fn cache_path(&self, packages: &[AuditPackage]) -> PathBuf {
        let mut digest = Sha256::new();
        digest.update(self.provider.cache_identity().as_bytes());
        for package in packages {
            digest.update([0]);
            digest.update(package.group.as_bytes());
            digest.update([0]);
            digest.update(package.artifact.as_bytes());
            digest.update([0]);
            digest.update(package.version.as_bytes());
        }
        self.options
            .cache_dir
            .join("audit")
            .join(cache_directory_name(self.provider.name()))
            .join(format!("{}.json", hex::encode(digest.finalize())))
    }
}

fn cache_directory_name(provider: &str) -> String {
    let value = provider
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() || value.chars().all(|character| character == '.') {
        "provider".to_owned()
    } else {
        value
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CachedAudit {
    schema_version: u32,
    fetched_at: u64,
    provider: String,
    findings: Vec<Finding>,
}

impl CachedAudit {
    fn is_fresh(&self, ttl: Duration) -> bool {
        let now = unix_timestamp();
        self.schema_version == CACHE_SCHEMA_VERSION
            && self.fetched_at <= now.saturating_add(300)
            && now.saturating_sub(self.fetched_at) <= ttl.as_secs()
    }

    fn into_result(self, source: AuditSource) -> AuditResult {
        AuditResult {
            provider: self.provider,
            source,
            findings: self.findings,
        }
    }
}

async fn read_cache(path: &PathBuf) -> Result<Option<CachedAudit>, AuditError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(AuditError::Cache {
                path: path.clone(),
                source,
            });
        }
    };
    let cache: CachedAudit =
        serde_json::from_slice(&bytes).map_err(|error| AuditError::InvalidCache {
            path: path.clone(),
            message: error.to_string(),
        })?;
    if cache.schema_version != CACHE_SCHEMA_VERSION {
        return Err(AuditError::InvalidCache {
            path: path.clone(),
            message: format!("unsupported schema version {}", cache.schema_version),
        });
    }
    Ok(Some(cache))
}

async fn write_cache(path: &PathBuf, cache: &CachedAudit) -> Result<(), AuditError> {
    let parent = path.parent().expect("audit cache has a parent");
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|source| AuditError::Cache {
            path: parent.to_owned(),
            source,
        })?;
    let bytes = serde_json::to_vec(cache).expect("cached audit is serializable");
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    tokio::fs::write(&temporary, bytes)
        .await
        .map_err(|source| AuditError::Cache {
            path: temporary.clone(),
            source,
        })?;
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|source| AuditError::Cache {
            path: path.clone(),
            source,
        })
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use super::*;
    use crate::{Advisory, Severity};

    #[derive(Clone)]
    struct FakeProvider {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl VulnerabilityProvider for FakeProvider {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn query<'a>(
            &'a self,
            packages: &'a [AuditPackage],
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Finding>, AuditError>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::Relaxed);
                if self.fail {
                    return Err(AuditError::InvalidResponse(
                        "provider unavailable".to_owned(),
                    ));
                }
                Ok(vec![Finding {
                    package: packages[0].clone(),
                    advisory: Advisory {
                        id: "TEST-1".to_owned(),
                        aliases: Vec::new(),
                        summary: "fixture".to_owned(),
                        severity: Severity::High,
                        fixed_versions: vec!["2".to_owned()],
                        references: Vec::new(),
                        modified: None,
                    },
                }])
            })
        }
    }

    fn package() -> AuditPackage {
        AuditPackage {
            group: "org.example".to_owned(),
            artifact: "library".to_owned(),
            version: "1".to_owned(),
        }
    }

    #[test]
    fn provider_names_cannot_escape_the_cache_directory() {
        assert_eq!(
            cache_directory_name("../../custom/provider"),
            "______custom_provider"
        );
        assert_eq!(cache_directory_name(""), "provider");
    }

    #[tokio::test]
    async fn caches_provider_results_for_online_and_offline_reuse() {
        let directory = tempfile::tempdir().expect("cache");
        let calls = Arc::new(AtomicUsize::new(0));
        let options = AuditOptions {
            cache_dir: directory.path().to_owned(),
            offline: false,
            refresh: false,
            ttl: Duration::from_mins(1),
        };
        let first = Auditor::new(
            FakeProvider {
                calls: calls.clone(),
                fail: false,
            },
            options.clone(),
        )
        .audit(&[package()])
        .await
        .expect("network audit");
        assert_eq!(first.source, AuditSource::Network);

        let cached = Auditor::new(
            FakeProvider {
                calls: calls.clone(),
                fail: true,
            },
            options,
        )
        .audit(&[package()])
        .await
        .expect("fresh cache");
        assert_eq!(cached.source, AuditSource::Cache);
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        let offline = Auditor::new(
            FakeProvider {
                calls: calls.clone(),
                fail: true,
            },
            AuditOptions {
                cache_dir: directory.path().to_owned(),
                offline: true,
                refresh: false,
                ttl: Duration::ZERO,
            },
        )
        .audit(&[package()])
        .await
        .expect("offline cache");
        assert_eq!(offline.source, AuditSource::Cache);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn falls_back_to_stale_cache_when_refresh_fails() {
        let directory = tempfile::tempdir().expect("cache");
        let calls = Arc::new(AtomicUsize::new(0));
        let options = AuditOptions {
            cache_dir: directory.path().to_owned(),
            offline: false,
            refresh: false,
            ttl: Duration::from_mins(1),
        };
        Auditor::new(
            FakeProvider {
                calls: calls.clone(),
                fail: false,
            },
            options.clone(),
        )
        .audit(&[package()])
        .await
        .expect("populate cache");

        let stale = Auditor::new(
            FakeProvider { calls, fail: true },
            AuditOptions {
                refresh: true,
                ..options
            },
        )
        .audit(&[package()])
        .await
        .expect("stale fallback");
        assert_eq!(stale.source, AuditSource::StaleCache);
    }
}
