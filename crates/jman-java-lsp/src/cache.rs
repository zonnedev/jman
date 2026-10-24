use std::{
    io::Write,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// Return the root containing all regenerable language-server cache data.
#[must_use]
pub fn cache_root() -> PathBuf {
    std::env::var_os("JMAN_CACHE_DIR")
        .map_or_else(
            || {
                dirs::cache_dir()
                    .unwrap_or_else(|| PathBuf::from(".jman-cache"))
                    .join("jman")
            },
            PathBuf::from,
        )
        .join("lsp")
}

/// Return whether an active language-server process owns a cache lock.
#[must_use]
pub fn cache_is_active() -> bool {
    cache_is_active_at(&cache_root())
}

fn cache_is_active_at(root: &Path) -> bool {
    cache_lock_directories(root)
        .into_iter()
        .any(|directory| directory_has_active_lock(&directory))
}

fn cache_lock_directories(root: &Path) -> [PathBuf; 3] {
    [
        root.join("projects"),
        root.join("decompiled"),
        root.join("external-sources"),
    ]
}

fn directory_has_active_lock(directory: &Path) -> bool {
    std::fs::read_dir(directory).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                return false;
            };
            if metadata.file_type().is_symlink() {
                return false;
            }
            if metadata.is_dir() {
                return directory_has_active_lock(&path);
            }
            path.extension().is_some_and(|value| value == "lock") && !lock_is_stale(&path)
        })
    })
}

pub(crate) fn project_cache_directory(root: &Path) -> PathBuf {
    cache_root().join("projects").join(stable_project_id(root))
}

pub(crate) fn stable_project_id(root: &Path) -> String {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut digest = Sha256::new();
    digest.update(canonical.to_string_lossy().as_bytes());
    for relative in ["pom.xml", "settings.gradle", "settings.gradle.kts"] {
        let path = canonical.join(relative);
        if path.exists() {
            digest.update(relative.as_bytes());
        }
    }
    hex::encode(digest.finalize())[..24].to_owned()
}

#[cfg(feature = "native-ffi")]
pub(crate) fn directory_size(root: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            entry.metadata().ok().map_or(0, |metadata| {
                if metadata.is_dir() {
                    directory_size(&entry.path())
                } else {
                    metadata.len()
                }
            })
        })
        .sum()
}

pub(crate) struct FileLock {
    path: PathBuf,
}

impl FileLock {
    pub(crate) fn acquire(cache: &Path) -> Result<Self, String> {
        let path = cache.with_extension("lock");
        // A real Gradle import can take minutes on the first run. Wait for the
        // owning editor instead of starting a second importer or failing a
        // second language-server instance after an arbitrary short timeout.
        for _ in 0..30_000 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path) {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
                Err(error) => return Err(format!("cannot lock {}: {error}", cache.display())),
            }
        }
        Err(format!("timed out locking {}", cache.display()))
    }
}

fn lock_is_stale(path: &Path) -> bool {
    let owner = std::fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok());
    #[cfg(unix)]
    if let Some(owner) = owner {
        return !process_is_alive(owner);
    }
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > std::time::Duration::from_secs(60 * 60))
}

#[cfg(unix)]
fn process_is_alive(owner: u32) -> bool {
    use rustix::{io::Errno, process::Pid};

    let Ok(owner) = i32::try_from(owner) else {
        return false;
    };
    let Some(owner) = Pid::from_raw(owner) else {
        return false;
    };
    match rustix::process::test_kill_process(owner) {
        Ok(()) | Err(Errno::PERM) => true,
        Err(_) => false,
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_identity_is_stable_for_symlinks_and_changes_with_build_identity() {
        let root = std::env::temp_dir().join(format!("jman-java-cache-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("pom.xml"), "<project/>").unwrap();
        let first = stable_project_id(&root);
        #[cfg(unix)]
        {
            let alias = root.with_extension("alias");
            let _ = std::fs::remove_file(&alias);
            std::os::unix::fs::symlink(&root, &alias).unwrap();
            assert_eq!(first, stable_project_id(&alias));
            std::fs::remove_file(alias).unwrap();
        }
        std::fs::write(
            root.join("pom.xml"),
            "<project><name>changed</name></project>",
        )
        .unwrap();
        assert_eq!(first, stable_project_id(&root));
        std::fs::remove_file(root.join("pom.xml")).unwrap();
        std::fs::write(root.join("settings.gradle"), "rootProject.name='changed'").unwrap();
        assert_ne!(first, stable_project_id(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn abandoned_lock_is_recovered() {
        let cache =
            std::env::temp_dir().join(format!("jman-java-abandoned-lock-{}", std::process::id()));
        let lock = cache.with_extension("lock");
        std::fs::write(&lock, "4294967295\n").unwrap();
        assert!(lock_is_stale(&lock));
        let acquired = FileLock::acquire(&cache).unwrap();
        assert!(lock.is_file());
        drop(acquired);
        assert!(!lock.exists());
    }

    #[cfg(unix)]
    #[test]
    fn lock_owned_by_current_process_is_not_abandoned() {
        let cache =
            std::env::temp_dir().join(format!("jman-java-live-lock-{}", std::process::id()));
        let lock = cache.with_extension("lock");
        std::fs::write(&lock, format!("{}\n", std::process::id())).unwrap();
        assert!(!lock_is_stale(&lock));
        std::fs::remove_file(lock).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn active_cache_detection_checks_language_server_lock_directories() {
        let root = tempfile::tempdir().expect("cache root");
        let decompiled = root.path().join("decompiled/class-hash");
        std::fs::create_dir_all(&decompiled).expect("decompiler cache directory");
        let lock = decompiled.join("decompiler-cache.lock");
        std::fs::write(&lock, std::process::id().to_string()).expect("active lock");
        assert!(cache_is_active_at(root.path()));

        std::fs::remove_file(lock).expect("remove active lock");
        assert!(!cache_is_active_at(root.path()));
    }

    #[test]
    fn cache_lock_serializes_writers_and_recovers_after_release() {
        let root =
            std::env::temp_dir().join(format!("jman-java-cache-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cache = root.join("index.json");
        let first = FileLock::acquire(&cache).unwrap();
        let target = cache.clone();
        let writer = std::thread::spawn(move || FileLock::acquire(&target).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(!writer.is_finished());
        drop(first);
        drop(writer.join().unwrap());
        assert!(!cache.with_extension("lock").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
