use std::{
    collections::BTreeSet,
    ffi::OsString,
    fmt, fs,
    path::{Path, PathBuf},
};

const KNOWN_LTS_JAVA_MAJORS: &[u16] = &[8, 11, 17, 21, 25];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRuntime {
    pub build_tool: &'static str,
    pub build_tool_version: Option<String>,
    pub java_home: PathBuf,
    pub java_version: String,
    pub java_major: u16,
    pub source: &'static str,
}

impl BuildRuntime {
    pub fn summary(&self) -> String {
        let tool = self.build_tool_version.as_ref().map_or_else(
            || self.build_tool.to_owned(),
            |version| format!("{} {version}", self.build_tool),
        );
        format!(
            "{tool} using Java {} from {} ({})",
            self.java_major,
            self.java_home.display(),
            self.source
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GradleVersion {
    major: u16,
    minor: u16,
    patch: u16,
}

impl GradleVersion {
    const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for GradleVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.patch == 0 {
            write!(formatter, "{}.{}", self.major, self.minor)
        } else {
            write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
        }
    }
}

#[derive(Clone, Debug)]
struct JavaCandidate {
    home: PathBuf,
    version: String,
    version_key: Vec<u32>,
    major: u16,
    source: &'static str,
    source_priority: u8,
}

#[derive(Clone, Debug)]
struct RuntimeEnvironment {
    explicit_home: Option<PathBuf>,
    cache_dir: PathBuf,
    sdkman_java_dir: Option<PathBuf>,
    java_home: Option<PathBuf>,
    path: Option<OsString>,
}

impl RuntimeEnvironment {
    fn from_process() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let cache_dir = std::env::var_os("JMAN_CACHE_DIR").map_or_else(
            || {
                dirs::cache_dir()
                    .unwrap_or_else(|| PathBuf::from(".jman-cache"))
                    .join("jman")
            },
            PathBuf::from,
        );
        let sdkman_java_dir = std::env::var_os("SDKMAN_CANDIDATES_DIR")
            .map(PathBuf::from)
            .map(|directory| directory.join("java"))
            .or_else(|| home.map(|directory| directory.join(".sdkman/candidates/java")));
        Self {
            explicit_home: std::env::var_os("JAVA_LSP_BUILD_JAVA_HOME").map(PathBuf::from),
            cache_dir,
            sdkman_java_dir,
            java_home: std::env::var_os("JAVA_HOME").map(PathBuf::from),
            path: std::env::var_os("PATH"),
        }
    }
}

pub fn select_gradle_runtime(root: &Path) -> Result<Option<BuildRuntime>, String> {
    select_gradle_runtime_with(root, &RuntimeEnvironment::from_process())
}

fn select_gradle_runtime_with(
    root: &Path,
    environment: &RuntimeEnvironment,
) -> Result<Option<BuildRuntime>, String> {
    let gradle_version = gradle_wrapper_version(root)?;

    if let Some(home) = &environment.explicit_home {
        return runtime_from_home(home, "jman.java.buildJavaHome", gradle_version).map(Some);
    }
    if let Some(home) = project_gradle_java_home(root)? {
        return runtime_from_home(&home, "org.gradle.java.home", gradle_version).map(Some);
    }

    let requested_major = project_gradle_java_major(root)?;
    if gradle_version.is_none() && requested_major.is_none() {
        return Ok(None);
    }

    let mut candidates = Vec::new();
    collect_directory_candidates(
        &environment.cache_dir.join("jdks"),
        "JMAN-managed",
        4,
        &mut candidates,
    );
    if let Some(directory) = &environment.sdkman_java_dir {
        collect_directory_candidates(directory, "SDKMAN", 3, &mut candidates);
    }
    if let Some(home) = &environment.java_home {
        add_candidate(home, "JAVA_HOME", 2, &mut candidates);
    }
    collect_path_candidates(environment.path.as_deref(), &mut candidates);
    deduplicate_candidates(&mut candidates);

    if let Some(major) = requested_major {
        if let Some(version) = gradle_version
            && !gradle_supports_java(version, major)
        {
            return Err(format!(
                "Gradle {version} cannot run on the project-requested Java {major}; update the Gradle daemon JVM criteria or wrapper"
            ));
        }
        candidates.retain(|candidate| candidate.major == major);
    } else if let Some(version) = gradle_version {
        candidates.retain(|candidate| gradle_supports_java(version, candidate.major));
    }

    candidates.sort_by(|left, right| {
        let left_lts = KNOWN_LTS_JAVA_MAJORS.contains(&left.major);
        let right_lts = KNOWN_LTS_JAVA_MAJORS.contains(&right.major);
        (
            right_lts,
            right.major,
            &right.version_key,
            right.source_priority,
        )
            .cmp(&(
                left_lts,
                left.major,
                &left.version_key,
                left.source_priority,
            ))
    });

    let Some(candidate) = candidates.into_iter().next() else {
        let required_major = requested_major.or_else(|| {
            gradle_version.and_then(|version| {
                KNOWN_LTS_JAVA_MAJORS
                    .iter()
                    .rev()
                    .copied()
                    .find(|major| gradle_supports_java(version, *major))
            })
        });
        let tool = gradle_version.map_or_else(
            || "Gradle".to_owned(),
            |version| format!("Gradle {version}"),
        );
        let install = required_major.map_or_else(
            || "install a compatible JDK".to_owned(),
            |major| format!("run `jman java install {major}`"),
        );
        return Err(format!(
            "no installed Java runtime is compatible with {tool}; {install} or configure `jman.java.buildJavaHome`"
        ));
    };

    Ok(Some(BuildRuntime {
        build_tool: "Gradle",
        build_tool_version: gradle_version.map(|version| version.to_string()),
        java_home: candidate.home,
        java_version: candidate.version,
        java_major: candidate.major,
        source: candidate.source,
    }))
}

fn runtime_from_home(
    home: &Path,
    source: &'static str,
    gradle_version: Option<GradleVersion>,
) -> Result<BuildRuntime, String> {
    let candidate = java_candidate(home, source, 5).ok_or_else(|| {
        format!(
            "{source} points to {}, which is not a JDK with a readable `release` file and `bin/java`",
            home.display()
        )
    })?;
    if let Some(version) = gradle_version
        && !gradle_supports_java(version, candidate.major)
    {
        return Err(format!(
            "Gradle {version} cannot run on Java {}; configure `jman.java.buildJavaHome` with a compatible JDK",
            candidate.major
        ));
    }
    Ok(BuildRuntime {
        build_tool: "Gradle",
        build_tool_version: gradle_version.map(|version| version.to_string()),
        java_home: candidate.home,
        java_version: candidate.version,
        java_major: candidate.major,
        source: candidate.source,
    })
}

fn gradle_wrapper_version(root: &Path) -> Result<Option<GradleVersion>, String> {
    let properties = root.join("gradle/wrapper/gradle-wrapper.properties");
    if !properties.is_file() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&properties)
        .map_err(|error| format!("cannot read {}: {error}", properties.display()))?;
    let Some(url) = property(&contents, "distributionUrl") else {
        return Ok(None);
    };
    let archive = url.rsplit('/').next().unwrap_or(url);
    let Some(version) = archive.strip_prefix("gradle-").and_then(|name| {
        name.strip_suffix("-bin.zip")
            .or_else(|| name.strip_suffix("-all.zip"))
    }) else {
        return Ok(None);
    };
    Ok(parse_gradle_version(version))
}

fn parse_gradle_version(version: &str) -> Option<GradleVersion> {
    let numeric = version
        .split_once('-')
        .map_or(version, |(numeric, _)| numeric);
    let mut components = numeric.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components.next().unwrap_or("0").parse().ok()?;
    let patch = components.next().unwrap_or("0").parse().ok()?;
    Some(GradleVersion::new(major, minor, patch))
}

fn gradle_supports_java(gradle: GradleVersion, java: u16) -> bool {
    let minimum = match java {
        8 => GradleVersion::new(2, 0, 0),
        9 => GradleVersion::new(4, 3, 0),
        10 => GradleVersion::new(4, 7, 0),
        11 => GradleVersion::new(5, 0, 0),
        12 => GradleVersion::new(5, 4, 0),
        13 => GradleVersion::new(6, 0, 0),
        14 => GradleVersion::new(6, 3, 0),
        15 => GradleVersion::new(6, 7, 0),
        16 => GradleVersion::new(7, 0, 0),
        17 => GradleVersion::new(7, 3, 0),
        18 => GradleVersion::new(7, 5, 0),
        19 => GradleVersion::new(7, 6, 0),
        20 => GradleVersion::new(8, 3, 0),
        21 => GradleVersion::new(8, 5, 0),
        22 => GradleVersion::new(8, 8, 0),
        23 => GradleVersion::new(8, 10, 0),
        24 => GradleVersion::new(8, 14, 0),
        25 => GradleVersion::new(9, 1, 0),
        26 => GradleVersion::new(9, 4, 0),
        _ => return false,
    };
    gradle >= minimum && !(java <= 16 && gradle >= GradleVersion::new(9, 0, 0))
}

fn project_gradle_java_home(root: &Path) -> Result<Option<PathBuf>, String> {
    let properties = root.join("gradle.properties");
    if !properties.is_file() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&properties)
        .map_err(|error| format!("cannot read {}: {error}", properties.display()))?;
    Ok(property(&contents, "org.gradle.java.home").map(|home| {
        let home = PathBuf::from(home);
        if home.is_absolute() {
            home
        } else {
            root.join(home)
        }
    }))
}

fn project_gradle_java_major(root: &Path) -> Result<Option<u16>, String> {
    let properties = root.join("gradle/gradle-daemon-jvm.properties");
    if !properties.is_file() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&properties)
        .map_err(|error| format!("cannot read {}: {error}", properties.display()))?;
    property(&contents, "toolchainVersion")
        .map(|version| {
            version.parse().map_err(|_| {
                format!(
                    "invalid Gradle daemon toolchainVersion `{version}` in {}",
                    properties.display()
                )
            })
        })
        .transpose()
}

fn property<'a>(contents: &'a str, name: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('!') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        (key.trim() == name).then(|| value.trim())
    })
}

fn collect_directory_candidates(
    directory: &Path,
    source: &'static str,
    priority: u8,
    candidates: &mut Vec<JavaCandidate>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut homes: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    homes.sort();
    for home in homes {
        add_candidate(&home, source, priority, candidates);
    }
}

fn collect_path_candidates(path: Option<&std::ffi::OsStr>, candidates: &mut Vec<JavaCandidate>) {
    let Some(path) = path else {
        return;
    };
    for directory in std::env::split_paths(path) {
        let executable = directory.join(if cfg!(windows) { "java.exe" } else { "java" });
        let Ok(executable) = fs::canonicalize(executable) else {
            continue;
        };
        let Some(home) = executable.parent().and_then(Path::parent) else {
            continue;
        };
        add_candidate(home, "PATH", 1, candidates);
    }
}

fn add_candidate(
    home: &Path,
    source: &'static str,
    priority: u8,
    candidates: &mut Vec<JavaCandidate>,
) {
    if let Some(candidate) = java_candidate(home, source, priority) {
        candidates.push(candidate);
    }
}

fn java_candidate(home: &Path, source: &'static str, priority: u8) -> Option<JavaCandidate> {
    let executable = home
        .join("bin")
        .join(if cfg!(windows) { "java.exe" } else { "java" });
    if !executable.is_file() {
        return None;
    }
    let release = fs::read_to_string(home.join("release")).ok()?;
    let version = property(&release, "JAVA_VERSION")?
        .trim_matches('"')
        .to_owned();
    let major = java_major(&version)?;
    Some(JavaCandidate {
        home: fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf()),
        version_key: java_version_key(&version),
        version,
        major,
        source,
        source_priority: priority,
    })
}

fn java_major(version: &str) -> Option<u16> {
    let numeric = version.strip_prefix("1.").unwrap_or(version);
    numeric
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

fn java_version_key(version: &str) -> Vec<u32> {
    version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .filter_map(|component| component.parse().ok())
        .collect()
}

fn deduplicate_candidates(candidates: &mut Vec<JavaCandidate>) {
    let mut homes = BTreeSet::new();
    candidates.retain(|candidate| homes.insert(candidate.home.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        tempfile::tempdir().expect("fixture")
    }

    fn wrapper(root: &Path, version: &str) {
        let directory = root.join("gradle/wrapper");
        fs::create_dir_all(&directory).expect("wrapper directory");
        fs::write(
            directory.join("gradle-wrapper.properties"),
            format!(
                "distributionUrl=https\\://services.gradle.org/distributions/gradle-{version}-bin.zip\n"
            ),
        )
        .expect("wrapper properties");
    }

    fn jdk(root: &Path, name: &str, version: &str) -> PathBuf {
        let home = root.join(name);
        fs::create_dir_all(home.join("bin")).expect("JDK bin");
        fs::write(
            home.join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" }),
            b"runtime",
        )
        .expect("java executable");
        fs::write(
            home.join("release"),
            format!("JAVA_VERSION=\"{version}\"\n"),
        )
        .expect("JDK release");
        home
    }

    fn environment(root: &Path) -> RuntimeEnvironment {
        RuntimeEnvironment {
            explicit_home: None,
            cache_dir: root.join("cache"),
            sdkman_java_dir: Some(root.join("sdkman")),
            java_home: None,
            path: None,
        }
    }

    #[test]
    fn parses_standard_wrapper_versions() {
        let root = fixture();
        wrapper(root.path(), "8.7");
        assert_eq!(
            gradle_wrapper_version(root.path()).expect("version"),
            Some(GradleVersion::new(8, 7, 0))
        );
        wrapper(root.path(), "9.1.0-rc-1");
        assert_eq!(
            gradle_wrapper_version(root.path()).expect("version"),
            Some(GradleVersion::new(9, 1, 0))
        );
    }

    #[test]
    fn models_gradle_java_runtime_compatibility_boundaries() {
        assert!(gradle_supports_java(GradleVersion::new(8, 7, 0), 21));
        assert!(!gradle_supports_java(GradleVersion::new(8, 7, 0), 22));
        assert!(!gradle_supports_java(GradleVersion::new(8, 7, 0), 25));
        assert!(gradle_supports_java(GradleVersion::new(9, 1, 0), 25));
        assert!(!gradle_supports_java(GradleVersion::new(9, 0, 0), 11));
        assert!(gradle_supports_java(GradleVersion::new(9, 4, 0), 26));
    }

    #[test]
    fn gradle_8_7_uses_installed_java_21_instead_of_server_java_25() {
        let root = fixture();
        wrapper(root.path(), "8.7");
        let environment = environment(root.path());
        let java_21 = jdk(
            environment.sdkman_java_dir.as_deref().expect("SDKMAN"),
            "21.0.2-open",
            "21.0.2",
        );
        let java_25 = jdk(root.path(), "server-25", "25.0.4");
        let environment = RuntimeEnvironment {
            java_home: Some(java_25),
            ..environment
        };

        let selected = select_gradle_runtime_with(root.path(), &environment)
            .expect("selection")
            .expect("runtime");

        assert_eq!(selected.java_major, 21);
        assert_eq!(selected.java_home, java_21);
        assert_eq!(selected.source, "SDKMAN");
        assert_eq!(selected.build_tool_version.as_deref(), Some("8.7"));
    }

    #[test]
    fn gradle_9_1_can_reuse_server_java_25() {
        let root = fixture();
        wrapper(root.path(), "9.1.0");
        let java_25 = jdk(root.path(), "server-25", "25.0.4");
        let environment = RuntimeEnvironment {
            java_home: Some(java_25.clone()),
            ..environment(root.path())
        };

        let selected = select_gradle_runtime_with(root.path(), &environment)
            .expect("selection")
            .expect("runtime");

        assert_eq!(selected.java_major, 25);
        assert_eq!(selected.java_home, java_25);
    }

    #[test]
    fn explicit_incompatible_runtime_is_rejected() {
        let root = fixture();
        wrapper(root.path(), "8.7");
        let java_25 = jdk(root.path(), "explicit-25", "25.0.4");
        let environment = RuntimeEnvironment {
            explicit_home: Some(java_25),
            ..environment(root.path())
        };

        let error = select_gradle_runtime_with(root.path(), &environment).unwrap_err();

        assert!(error.contains("Gradle 8.7 cannot run on Java 25"));
        assert!(error.contains("jman.java.buildJavaHome"));
    }

    #[test]
    fn project_java_home_has_priority_over_automatic_selection() {
        let root = fixture();
        wrapper(root.path(), "8.7");
        let java_17 = jdk(root.path(), "project-17", "17.0.20");
        let environment = environment(root.path());
        jdk(
            environment.sdkman_java_dir.as_deref().expect("SDKMAN"),
            "21.0.2-open",
            "21.0.2",
        );
        fs::write(
            root.path().join("gradle.properties"),
            "org.gradle.java.home=project-17\n",
        )
        .expect("Gradle properties");

        let selected = select_gradle_runtime_with(root.path(), &environment)
            .expect("selection")
            .expect("runtime");

        assert_eq!(selected.java_home, java_17);
        assert_eq!(selected.java_major, 17);
        assert_eq!(selected.source, "org.gradle.java.home");
    }

    #[test]
    fn daemon_jvm_criteria_selects_the_requested_major() {
        let root = fixture();
        wrapper(root.path(), "9.1.0");
        let environment = environment(root.path());
        let java_21 = jdk(
            environment.sdkman_java_dir.as_deref().expect("SDKMAN"),
            "21.0.2-open",
            "21.0.2",
        );
        jdk(
            environment.sdkman_java_dir.as_deref().expect("SDKMAN"),
            "25.0.4-open",
            "25.0.4",
        );
        fs::create_dir_all(root.path().join("gradle")).expect("Gradle directory");
        fs::write(
            root.path().join("gradle/gradle-daemon-jvm.properties"),
            "toolchainVersion=21\n",
        )
        .expect("daemon criteria");

        let selected = select_gradle_runtime_with(root.path(), &environment)
            .expect("selection")
            .expect("runtime");

        assert_eq!(selected.java_home, java_21);
        assert_eq!(selected.java_major, 21);
    }

    #[test]
    fn missing_compatible_runtime_has_an_actionable_error() {
        let root = fixture();
        wrapper(root.path(), "8.7");
        let java_25 = jdk(root.path(), "server-25", "25.0.4");
        let environment = RuntimeEnvironment {
            java_home: Some(java_25),
            ..environment(root.path())
        };

        let error = select_gradle_runtime_with(root.path(), &environment).unwrap_err();

        assert!(error.contains("no installed Java runtime is compatible with Gradle 8.7"));
        assert!(error.contains("jman java install 21"));
    }

    #[test]
    fn managed_jdk_wins_over_sdkman_for_the_same_java_release() {
        let root = fixture();
        wrapper(root.path(), "8.7");
        let environment = environment(root.path());
        let managed = jdk(
            &environment.cache_dir.join("jdks"),
            "temurin-21.0.2",
            "21.0.2",
        );
        jdk(
            environment.sdkman_java_dir.as_deref().expect("SDKMAN"),
            "21.0.2-open",
            "21.0.2",
        );

        let selected = select_gradle_runtime_with(root.path(), &environment)
            .expect("selection")
            .expect("runtime");

        assert_eq!(selected.java_home, managed);
        assert_eq!(selected.source, "JMAN-managed");
    }

    #[test]
    fn newest_patch_wins_before_source_priority() {
        let root = fixture();
        wrapper(root.path(), "8.7");
        let environment = environment(root.path());
        jdk(
            &environment.cache_dir.join("jdks"),
            "temurin-21.0.2",
            "21.0.2+13",
        );
        let newer = jdk(
            environment.sdkman_java_dir.as_deref().expect("SDKMAN"),
            "21.0.10-open",
            "21.0.10+7",
        );

        let selected = select_gradle_runtime_with(root.path(), &environment)
            .expect("selection")
            .expect("runtime");

        assert_eq!(selected.java_home, newer);
        assert_eq!(selected.java_version, "21.0.10+7");
    }

    #[test]
    fn custom_wrapper_distribution_is_left_to_an_explicit_override() {
        let root = fixture();
        fs::create_dir_all(root.path().join("gradle/wrapper")).expect("wrapper directory");
        fs::write(
            root.path().join("gradle/wrapper/gradle-wrapper.properties"),
            "distributionUrl=https\\://example.test/company-gradle.zip\n",
        )
        .expect("wrapper properties");
        let java_25 = jdk(root.path(), "server-25", "25.0.4");
        let environment = RuntimeEnvironment {
            java_home: Some(java_25),
            ..environment(root.path())
        };

        assert_eq!(
            select_gradle_runtime_with(root.path(), &environment).expect("selection"),
            None
        );
    }
}
