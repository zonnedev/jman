use std::{
    io::Write,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

pub(crate) fn cache_root() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .map(|root| root.join("io.github.zonnedev.jman.lsp/cache"))
        .unwrap_or_else(|| std::env::temp_dir().join("io.github.zonnedev.jman.lsp/cache"))
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
    #[cfg(target_os = "linux")]
    if let Some(owner) = owner {
        return !Path::new("/proc").join(owner.to_string()).exists();
    }
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > std::time::Duration::from_secs(60 * 60))
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
        let acquired = FileLock::acquire(&cache).unwrap();
        assert!(lock.is_file());
        drop(acquired);
        assert!(!lock.exists());
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
