use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use base64::Engine;

const PROCESSOR_PROTOCOL_VERSION: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorRequest {
    pub java_executable: PathBuf,
    pub sources: Vec<PathBuf>,
    pub source_path: Vec<PathBuf>,
    pub classpath: Vec<PathBuf>,
    pub processor_path: Vec<PathBuf>,
    pub processor_options: Vec<String>,
    pub release: u8,
    pub generated_directory: PathBuf,
    pub classes_directory: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessorResult {
    pub generated_sources: u64,
    pub cache_hit: bool,
}

pub struct ProcessorWorker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    fingerprints: HashMap<String, u64>,
}

impl ProcessorWorker {
    pub fn start(java: &Path, worker_classpath: &Path) -> Result<Self, String> {
        let mut child = Command::new(java)
            .arg("-cp")
            .arg(worker_classpath)
            .arg("io.github.zonnedev.jman.processor.worker.ProcessorWorker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("cannot start processor worker: {error}"))?;
        let input = child.stdin.take().ok_or("processor worker has no stdin")?;
        let mut output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or("processor worker has no stdout")?,
        );
        let mut ready = String::new();
        output
            .read_line(&mut ready)
            .map_err(|error| format!("cannot read processor worker handshake: {error}"))?;
        if ready.trim() != format!("READY\t{PROCESSOR_PROTOCOL_VERSION}") {
            return Err(format!(
                "invalid processor worker handshake: {}",
                ready.trim()
            ));
        }
        Ok(Self {
            child,
            input,
            output,
            fingerprints: HashMap::new(),
        })
    }

    pub fn process(
        &mut self,
        key: &str,
        request: &ProcessorRequest,
        request_file: &Path,
    ) -> Result<ProcessorResult, String> {
        let fingerprint = request.fingerprint()?;
        let fingerprint_file = request_file.with_extension("fingerprint");
        let persisted_fingerprint = std::fs::read_to_string(&fingerprint_file)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok());
        if self.fingerprints.get(key) == Some(&fingerprint)
            || (persisted_fingerprint == Some(fingerprint) && request.generated_directory.is_dir())
        {
            self.fingerprints.insert(key.to_owned(), fingerprint);
            return Ok(ProcessorResult {
                generated_sources: count_java_sources(&request.generated_directory),
                cache_hit: true,
            });
        }
        std::fs::write(request_file, request.to_properties())
            .map_err(|error| format!("cannot write processor request: {error}"))?;
        writeln!(self.input, "{}", request_file.display())
            .and_then(|_| self.input.flush())
            .map_err(|error| format!("cannot send processor request: {error}"))?;
        let mut response = String::new();
        self.output
            .read_line(&mut response)
            .map_err(|error| format!("cannot read processor response: {error}"))?;
        let response = response.trim();
        if let Some(count) = response.strip_prefix("OK\t") {
            let generated_sources = count
                .parse()
                .map_err(|_| format!("invalid processor response: {response}"))?;
            self.fingerprints.insert(key.to_owned(), fingerprint);
            let temporary = fingerprint_file.with_extension("fingerprint.tmp");
            std::fs::write(&temporary, fingerprint.to_string())
                .and_then(|_| std::fs::rename(&temporary, &fingerprint_file))
                .map_err(|error| format!("cannot persist processor fingerprint: {error}"))?;
            let _ = std::fs::remove_file(request_file.with_extension("failed-fingerprint"));
            return Ok(ProcessorResult {
                generated_sources,
                cache_hit: false,
            });
        }
        if let Some(encoded) = response.strip_prefix("ERROR\t")
            && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded)
            && let Ok(message) = String::from_utf8(bytes)
        {
            persist_failed_fingerprint(request_file, fingerprint);
            return Err(format!("processor worker failed: {message}"));
        }
        persist_failed_fingerprint(request_file, fingerprint);
        Err(format!("processor worker failed: {response}"))
    }
}

impl Drop for ProcessorWorker {
    fn drop(&mut self) {
        let _ = writeln!(self.input, "STOP");
        let _ = self.input.flush();
        let _ = self.child.wait();
    }
}

impl ProcessorRequest {
    pub fn persistent_cache_hit(
        &self,
        request_file: &Path,
    ) -> Result<Option<ProcessorResult>, String> {
        let fingerprint = self.fingerprint()?;
        let persisted = std::fs::read_to_string(request_file.with_extension("fingerprint"))
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok());
        let failed = std::fs::read_to_string(request_file.with_extension("failed-fingerprint"))
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok());
        Ok(
            ((persisted == Some(fingerprint) && self.generated_directory.is_dir())
                || failed == Some(fingerprint))
            .then(|| ProcessorResult {
                generated_sources: count_java_sources(&self.generated_directory),
                cache_hit: true,
            }),
        )
    }

    fn fingerprint(&self) -> Result<u64, String> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        PROCESSOR_PROTOCOL_VERSION.hash(&mut hash);
        fingerprint_path(&self.java_executable, true, &mut hash)?;
        self.release.hash(&mut hash);
        self.processor_options.hash(&mut hash);
        self.generated_directory.hash(&mut hash);
        self.classes_directory.hash(&mut hash);
        for path in self.sources.iter().chain(&self.processor_path) {
            fingerprint_path(path, true, &mut hash)?;
        }
        for path in self.classpath.iter().chain(&self.source_path) {
            fingerprint_path(path, false, &mut hash)?;
        }
        Ok(hash.finish())
    }

    fn to_properties(&self) -> String {
        let mut output = format!("protocol.version={PROCESSOR_PROTOCOL_VERSION}\n");
        property(&mut output, "release", &self.release.to_string());
        property(
            &mut output,
            "generated.directory",
            &self.generated_directory.to_string_lossy(),
        );
        property(
            &mut output,
            "classes.directory",
            &self.classes_directory.to_string_lossy(),
        );
        path_list(&mut output, "source", &self.sources);
        path_list(&mut output, "source.path", &self.source_path);
        path_list(&mut output, "classpath", &self.classpath);
        path_list(&mut output, "processor.path", &self.processor_path);
        string_list(&mut output, "processor.option", &self.processor_options);
        output
    }
}

fn fingerprint_path(path: &Path, required: bool, hash: &mut impl Hasher) -> Result<(), String> {
    path.hash(hash);
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => {
            "missing-future-output".hash(hash);
            return Ok(());
        }
        Err(error) => {
            return Err(format!("cannot fingerprint {}: {error}", path.display()));
        }
    };
    metadata.len().hash(hash);
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .hash(hash);
    if metadata.is_file() && metadata.len() <= 16 * 1024 * 1024 {
        std::fs::read(path)
            .map_err(|error| format!("cannot fingerprint {}: {error}", path.display()))?
            .hash(hash);
    }
    Ok(())
}

fn persist_failed_fingerprint(request_file: &Path, fingerprint: u64) {
    let target = request_file.with_extension("failed-fingerprint");
    let temporary = request_file.with_extension("failed-fingerprint.tmp");
    let _ = std::fs::write(&temporary, fingerprint.to_string())
        .and_then(|_| std::fs::rename(temporary, target));
}

fn property(output: &mut String, name: &str, value: &str) {
    output.push_str(name);
    output.push('=');
    for character in value.chars() {
        if matches!(character, '\\' | ':' | '=' | '#' | '!') {
            output.push('\\');
        }
        output.push(character);
    }
    output.push('\n');
}

fn path_list(output: &mut String, name: &str, values: &[PathBuf]) {
    let values: Vec<_> = values
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    string_list(output, name, &values);
}

fn string_list(output: &mut String, name: &str, values: &[String]) {
    property(output, &format!("{name}.count"), &values.len().to_string());
    for (index, value) in values.iter().enumerate() {
        property(output, &format!("{name}.{index}"), value);
    }
}

fn count_java_sources(directory: &Path) -> u64 {
    std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .map(|path| {
            if path.is_dir() {
                count_java_sources(&path)
            } else {
                u64::from(path.extension().and_then(|value| value.to_str()) == Some("java"))
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_versioned_processor_requests() {
        let request = ProcessorRequest {
            java_executable: PathBuf::from("/jdk/bin/java"),
            sources: vec![PathBuf::from("/work/App.java")],
            source_path: vec![PathBuf::from("/work/src/main/java")],
            classpath: vec![PathBuf::from("/repo/api.jar")],
            processor_path: vec![PathBuf::from("/repo/processor.jar")],
            processor_options: vec!["-Amode=strict".to_owned()],
            release: 25,
            generated_directory: PathBuf::from("/work/generated"),
            classes_directory: PathBuf::from("/work/classes"),
        };
        let encoded = request.to_properties();
        assert!(encoded.contains("protocol.version=2"));
        assert!(encoded.contains("source.0=/work/App.java"));
        assert!(encoded.contains("source.path.0=/work/src/main/java"));
        assert!(encoded.contains("processor.option.0=-Amode\\=strict"));
    }

    #[test]
    fn fingerprint_changes_with_source_classpath_processor_and_options() {
        let root = std::env::temp_dir().join(format!(
            "jman-java-processor-fingerprint-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("App.java");
        let classpath = root.join("api.jar");
        let processor = root.join("processor.jar");
        let java = root.join("java");
        std::fs::write(&source, "class App {}").unwrap();
        std::fs::write(&classpath, "api-one").unwrap();
        std::fs::write(&processor, "processor-one").unwrap();
        std::fs::write(&java, "runtime-one").unwrap();
        let request = ProcessorRequest {
            java_executable: java.clone(),
            sources: vec![source.clone()],
            source_path: vec![root.join("src")],
            classpath: vec![classpath.clone()],
            processor_path: vec![processor.clone()],
            processor_options: vec!["-Amode=one".to_owned()],
            release: 25,
            generated_directory: root.join("generated"),
            classes_directory: root.join("classes"),
        };
        let baseline = request.fingerprint().unwrap();

        std::fs::write(&source, "class App { int value; }").unwrap();
        assert_ne!(baseline, request.fingerprint().unwrap());
        std::fs::write(&source, "class App {}").unwrap();
        std::fs::write(&classpath, "api-two").unwrap();
        assert_ne!(baseline, request.fingerprint().unwrap());
        std::fs::write(&classpath, "api-one").unwrap();
        std::fs::write(&processor, "processor-two").unwrap();
        assert_ne!(baseline, request.fingerprint().unwrap());
        let mut changed_options = request.clone();
        changed_options.processor_options = vec!["-Amode=two".to_owned()];
        assert_ne!(baseline, changed_options.fingerprint().unwrap());
        let mut changed_classes = request.clone();
        changed_classes.classes_directory = root.join("other-classes");
        assert_ne!(baseline, changed_classes.fingerprint().unwrap());
        std::fs::write(&java, "runtime-two").unwrap();
        assert_ne!(baseline, request.fingerprint().unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fingerprint_allows_missing_future_outputs_but_requires_inputs() {
        let root = std::env::temp_dir().join(format!(
            "jman-java-processor-optional-paths-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("App.java");
        let processor = root.join("processor.jar");
        let java = root.join("java");
        std::fs::write(&source, "class App {}").unwrap();
        std::fs::write(&processor, "processor").unwrap();
        std::fs::write(&java, "runtime").unwrap();
        let request = ProcessorRequest {
            java_executable: java,
            sources: vec![source.clone()],
            source_path: vec![root.join("future-generated-sources")],
            classpath: vec![root.join("future-classes")],
            processor_path: vec![processor.clone()],
            processor_options: vec![],
            release: 25,
            generated_directory: root.join("generated"),
            classes_directory: root.join("classes"),
        };

        request.fingerprint().unwrap();

        std::fs::remove_file(&source).unwrap();
        assert!(request.fingerprint().is_err());
        std::fs::write(&source, "class App {}").unwrap();
        std::fs::remove_file(&processor).unwrap();
        assert!(request.fingerprint().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_input_is_cached_without_discarding_last_good_generated_sources() {
        let root =
            std::env::temp_dir().join(format!("jman-java-processor-failed-{}", std::process::id()));
        let generated = root.join("generated/demo");
        std::fs::create_dir_all(&generated).unwrap();
        std::fs::write(generated.join("Generated.java"), "class Generated {}").unwrap();
        let source = root.join("App.java");
        let processor = root.join("processor.jar");
        let java = root.join("java");
        std::fs::write(&source, "class App {}").unwrap();
        std::fs::write(&processor, "processor").unwrap();
        std::fs::write(&java, "runtime").unwrap();
        let request = ProcessorRequest {
            java_executable: java,
            sources: vec![source],
            source_path: vec![root.join("src")],
            classpath: vec![],
            processor_path: vec![processor],
            processor_options: vec![],
            release: 25,
            generated_directory: root.join("generated"),
            classes_directory: root.join("classes"),
        };
        let request_file = root.join("request.properties");
        persist_failed_fingerprint(&request_file, request.fingerprint().unwrap());
        let hit = request
            .persistent_cache_hit(&request_file)
            .unwrap()
            .unwrap();
        assert!(hit.cache_hit);
        assert_eq!(hit.generated_sources, 1);
        assert!(generated.join("Generated.java").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
