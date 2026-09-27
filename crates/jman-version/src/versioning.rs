use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VersionInfo {
    pub release_version: String,
    pub build_version: String,
    pub git_commit: Option<String>,
    pub git_distance: Option<u64>,
    pub dirty: bool,
}

impl VersionInfo {
    #[must_use]
    pub fn override_version(release_version: &str, build_version: String) -> Self {
        Self {
            release_version: release_version.to_owned(),
            build_version,
            git_commit: None,
            git_distance: None,
            dirty: false,
        }
    }

    #[must_use]
    pub fn as_rust_source(&self) -> String {
        let commit = self
            .git_commit
            .as_deref()
            .map_or_else(|| "None".to_owned(), |value| format!("Some({value:?})"));
        let distance = self
            .git_distance
            .map_or_else(|| "None".to_owned(), |value| format!("Some({value})"));
        format!(
            "pub const RELEASE_VERSION: &str = {:?};\n\
             pub const BUILD_VERSION: &str = {:?};\n\
             pub const GIT_COMMIT: Option<&str> = {commit};\n\
             pub const GIT_DISTANCE: Option<u64> = {distance};\n\
             pub const DIRTY: bool = {};\n",
            self.release_version, self.build_version, self.dirty
        )
    }
}

#[must_use]
pub fn discover(repository: &Path, fallback: &str) -> VersionInfo {
    let fallback_info = || VersionInfo {
        release_version: fallback.to_owned(),
        build_version: fallback.to_owned(),
        git_commit: None,
        git_distance: None,
        dirty: false,
    };

    let tags = annotated_semver_tags(repository);
    if tags.is_empty() {
        return fallback_info();
    }

    let mut command = Command::new("git");
    command.arg("-C").arg(repository).args([
        "describe",
        "--long",
        "--dirty",
        "--first-parent",
        "--abbrev=12",
    ]);
    for tag in tags {
        command.arg("--match").arg(tag);
    }

    command
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|description| parse_git_description(description.trim()))
        .unwrap_or_else(fallback_info)
}

pub fn emit_git_rerun_directives(repository: &Path) {
    for name in ["HEAD", "index", "packed-refs", "refs/tags"] {
        if let Some(path) = git_path(repository, name) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    if let Some(reference) = git_output(repository, &["symbolic-ref", "--quiet", "HEAD"]) {
        if let Some(path) = git_path(repository, reference.trim()) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    if let Some(files) = git_output(repository, &["ls-files", "-z"]) {
        for file in files.split('\0').filter(|file| !file.is_empty()) {
            println!("cargo:rerun-if-changed={}", repository.join(file).display());
        }
    }
}

#[must_use]
pub fn parse_git_description(description: &str) -> Option<VersionInfo> {
    let (description, dirty) = description
        .strip_suffix("-dirty")
        .map_or((description, false), |clean| (clean, true));
    let mut components = description.rsplitn(3, '-');
    let commit = components.next()?;
    let distance = components.next()?.parse::<u64>().ok()?;
    let tag = components.next()?;
    let release_version = tag.strip_prefix('v')?;

    if !is_semver(release_version)
        || commit.len() < 2
        || !commit.starts_with('g')
        || !commit[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }

    let build_version = if distance == 0 && !dirty {
        release_version.to_owned()
    } else {
        let suffix = format!(
            "dev.{distance}.{commit}{}",
            if dirty { ".dirty" } else { "" }
        );
        append_build_metadata(release_version, &suffix)
    };

    Some(VersionInfo {
        release_version: release_version.to_owned(),
        build_version,
        git_commit: Some(commit.strip_prefix('g')?.to_owned()),
        git_distance: Some(distance),
        dirty,
    })
}

#[must_use]
pub fn is_semver(version: &str) -> bool {
    if version.is_empty() || version.starts_with('v') {
        return false;
    }

    let mut build_split = version.split('+');
    let Some(before_build) = build_split.next() else {
        return false;
    };
    let build = build_split.next();
    if build_split.next().is_some() || build.is_some_and(|value| !valid_identifiers(value, false)) {
        return false;
    }

    let (core, prerelease) = before_build
        .split_once('-')
        .map_or((before_build, None), |(core, prerelease)| {
            (core, Some(prerelease))
        });
    if prerelease.is_some_and(|value| !valid_identifiers(value, true)) {
        return false;
    }

    let mut numbers = core.split('.');
    let valid_core = (0..3).all(|_| numbers.next().is_some_and(valid_core_number));
    valid_core && numbers.next().is_none()
}

fn valid_core_number(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

fn valid_identifiers(value: &str, reject_numeric_leading_zero: bool) -> bool {
    !value.is_empty()
        && value.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && (!reject_numeric_leading_zero
                    || !identifier.bytes().all(|byte| byte.is_ascii_digit())
                    || identifier == "0"
                    || !identifier.starts_with('0'))
        })
}

fn append_build_metadata(version: &str, metadata: &str) -> String {
    if version.contains('+') {
        format!("{version}.{metadata}")
    } else {
        format!("{version}+{metadata}")
    }
}

fn annotated_semver_tags(repository: &Path) -> Vec<String> {
    git_output(
        repository,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(objecttype)",
            "refs/tags",
        ],
    )
    .into_iter()
    .flat_map(|output| {
        output
            .lines()
            .filter_map(|line| {
                let (tag, object_type) = line.split_once('\t')?;
                (object_type == "tag" && tag.strip_prefix('v').is_some_and(is_semver))
                    .then(|| tag.to_owned())
            })
            .collect::<Vec<_>>()
    })
    .collect()
}

fn git_path(repository: &Path, name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(git_output(repository, &["rev-parse", "--git-path", name])?.trim());
    Some(if path.is_absolute() {
        path
    } else {
        repository.join(path)
    })
}

fn git_output(repository: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use tempfile::tempdir;

    use super::{discover, is_semver, parse_git_description};

    #[test]
    fn converts_git_descriptions_to_semver_build_identity() {
        let exact = parse_git_description("v1.4.2-0-g8fa23cd12345").unwrap();
        assert_eq!(exact.release_version, "1.4.2");
        assert_eq!(exact.build_version, "1.4.2");
        assert_eq!(exact.git_distance, Some(0));
        assert!(!exact.dirty);

        let development = parse_git_description("v1.4.2-rc.1-3-g8fa23cd12345-dirty").unwrap();
        assert_eq!(development.release_version, "1.4.2-rc.1");
        assert_eq!(
            development.build_version,
            "1.4.2-rc.1+dev.3.g8fa23cd12345.dirty"
        );
        assert_eq!(development.git_commit.as_deref(), Some("8fa23cd12345"));
        assert_eq!(development.git_distance, Some(3));
        assert!(development.dirty);
    }

    #[test]
    fn validates_release_and_override_versions_as_semver() {
        for valid in [
            "0.8.2",
            "1.0.0-rc.1",
            "1.0.0+dev.3.g8fa23cd",
            "1.0.0-rc.1+dev.3.g8fa23cd.dirty",
        ] {
            assert!(is_semver(valid), "expected valid SemVer: {valid}");
        }
        for invalid in [
            "v1.0.0",
            "1.0",
            "01.0.0",
            "1.0.0-dev.01",
            "1.0.0+",
            "unknown",
        ] {
            assert!(!is_semver(invalid), "expected invalid SemVer: {invalid}");
        }
    }

    #[test]
    fn discovers_only_annotated_semver_tags_and_tracks_dirty_sources() {
        let repository = tempdir().unwrap();
        git(repository.path(), &["init", "-q"]);
        git(repository.path(), &["config", "user.name", "JMAN Test"]);
        git(
            repository.path(),
            &["config", "user.email", "jman@example.invalid"],
        );
        git(repository.path(), &["config", "commit.gpgSign", "false"]);
        git(repository.path(), &["config", "tag.gpgSign", "false"]);
        fs::write(repository.path().join("source.txt"), "one\n").unwrap();
        git(repository.path(), &["add", "source.txt"]);
        git(repository.path(), &["commit", "-q", "-m", "initial"]);
        git(repository.path(), &["tag", "-a", "v1.4.2", "-m", "v1.4.2"]);
        git(repository.path(), &["tag", "v9.0.0"]);

        assert_eq!(discover(repository.path(), "0.0.0").build_version, "1.4.2");

        fs::write(repository.path().join("source.txt"), "two\n").unwrap();
        git(repository.path(), &["add", "source.txt"]);
        git(repository.path(), &["commit", "-q", "-m", "second"]);
        git(
            repository.path(),
            &["tag", "-a", "v1.4", "-m", "malformed version"],
        );
        let development = discover(repository.path(), "0.0.0");
        assert!(development.build_version.starts_with("1.4.2+dev.1.g"));
        assert!(!development.dirty);

        fs::write(repository.path().join("source.txt"), "dirty\n").unwrap();
        let dirty = discover(repository.path(), "0.0.0");
        assert_eq!(dirty.build_version.rsplit('.').next(), Some("dirty"));
    }

    #[test]
    fn falls_back_when_git_or_release_tags_are_unavailable() {
        let repository = tempdir().unwrap();
        assert_eq!(discover(repository.path(), "2.0.0").build_version, "2.0.0");
    }

    #[test]
    fn follows_first_parent_instead_of_tags_from_merged_branches() {
        let repository = tempdir().unwrap();
        git(repository.path(), &["init", "-q", "-b", "main"]);
        git(repository.path(), &["config", "user.name", "JMAN Test"]);
        git(
            repository.path(),
            &["config", "user.email", "jman@example.invalid"],
        );
        git(repository.path(), &["config", "commit.gpgSign", "false"]);
        git(repository.path(), &["config", "tag.gpgSign", "false"]);
        fs::write(repository.path().join("base.txt"), "base\n").unwrap();
        git(repository.path(), &["add", "base.txt"]);
        git(repository.path(), &["commit", "-q", "-m", "base"]);
        git(repository.path(), &["tag", "-a", "v1.0.0", "-m", "v1.0.0"]);

        git(repository.path(), &["checkout", "-q", "-b", "topic"]);
        fs::write(repository.path().join("topic.txt"), "topic\n").unwrap();
        git(repository.path(), &["add", "topic.txt"]);
        git(repository.path(), &["commit", "-q", "-m", "topic"]);
        git(repository.path(), &["tag", "-a", "v9.0.0", "-m", "v9.0.0"]);

        git(repository.path(), &["checkout", "-q", "main"]);
        fs::write(repository.path().join("main.txt"), "main\n").unwrap();
        git(repository.path(), &["add", "main.txt"]);
        git(repository.path(), &["commit", "-q", "-m", "main"]);
        git(
            repository.path(),
            &["merge", "-q", "--no-ff", "topic", "-m", "merge topic"],
        );

        let version = discover(repository.path(), "0.0.0");
        assert!(version.build_version.starts_with("1.0.0+dev.2.g"));
    }

    fn git(repository: &std::path::Path, arguments: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success(), "git command failed: {arguments:?}");
    }
}
