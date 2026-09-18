use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use base64::Engine;
use javac_frontend::{
    EditorQueryResult, SemanticResult, decode_editor_query_result, decode_semantic_result,
};

const PROTOCOL_VERSION: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessedSemanticRequest {
    pub file_name: String,
    pub source: String,
    pub release: u8,
    pub classpath: Vec<PathBuf>,
    pub module_path: Vec<PathBuf>,
    pub source_path: Vec<PathBuf>,
    pub processor_path: Vec<PathBuf>,
    pub processor_options: Vec<String>,
    pub compiler_options: Vec<String>,
}

pub(crate) struct ProcessedSemanticWorker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl ProcessedSemanticWorker {
    pub fn start(java: &Path, worker_classpath: &Path) -> Result<Self, String> {
        let mut child = Command::new(java)
            .arg("-cp")
            .arg(worker_classpath)
            .arg("io.github.zonnedev.jman.javac.ProcessedSemanticWorker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("cannot start processed semantic worker: {error}"))?;
        let input = child
            .stdin
            .take()
            .ok_or("processed semantic worker has no stdin")?;
        let mut output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or("processed semantic worker has no stdout")?,
        );
        let mut ready = String::new();
        output
            .read_line(&mut ready)
            .map_err(|error| format!("cannot read processed semantic worker handshake: {error}"))?;
        if ready.trim() != format!("READY\t{PROTOCOL_VERSION}") {
            return Err(format!(
                "invalid processed semantic worker handshake: {}",
                ready.trim()
            ));
        }
        Ok(Self {
            child,
            input,
            output,
        })
    }

    pub fn analyze(
        &mut self,
        request: &ProcessedSemanticRequest,
        request_file: &Path,
    ) -> Result<SemanticResult, String> {
        let payload = self.execute("analyze", None, request, request_file)?;
        decode_semantic_result(&payload)
            .map_err(|error| format!("invalid processed semantic result: {error:?}"))
    }

    pub fn editor_query(
        &mut self,
        request: &ProcessedSemanticRequest,
        cursor: u32,
        request_file: &Path,
    ) -> Result<EditorQueryResult, String> {
        let payload = self.execute("query", Some(cursor), request, request_file)?;
        decode_editor_query_result(&payload)
            .map_err(|error| format!("invalid processed editor result: {error:?}"))
    }

    fn execute(
        &mut self,
        operation: &str,
        cursor: Option<u32>,
        request: &ProcessedSemanticRequest,
        request_file: &Path,
    ) -> Result<Vec<u8>, String> {
        let parent = request_file.parent().ok_or_else(|| {
            format!(
                "processed request has no parent: {}",
                request_file.display()
            )
        })?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create processed request directory: {error}"))?;
        let encoded = request.to_properties(operation, cursor);
        let temporary = request_file.with_extension("properties.tmp");
        std::fs::write(&temporary, encoded)
            .and_then(|()| std::fs::rename(&temporary, request_file))
            .map_err(|error| format!("cannot write processed semantic request: {error}"))?;
        writeln!(self.input, "{}", request_file.display())
            .and_then(|()| self.input.flush())
            .map_err(|error| format!("cannot send processed semantic request: {error}"))?;
        let mut response = String::new();
        let response_read = self
            .output
            .read_line(&mut response)
            .map_err(|error| format!("cannot read processed semantic response: {error}"));
        // Requests can contain unsaved editor text. They are transport files,
        // not cache entries, and must not persist after the worker consumes them.
        let _ = std::fs::remove_file(request_file);
        response_read?;
        let response = response.trim();
        if let Some(encoded) = response.strip_prefix("OK\t") {
            return base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| format!("invalid processed semantic payload: {error}"));
        }
        if let Some(encoded) = response.strip_prefix("ERROR\t")
            && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded)
            && let Ok(message) = String::from_utf8(bytes)
        {
            return Err(format!("processed semantic worker failed: {message}"));
        }
        Err(format!("processed semantic worker failed: {response}"))
    }
}

impl Drop for ProcessedSemanticWorker {
    fn drop(&mut self) {
        let _ = writeln!(self.input, "STOP");
        let _ = self.input.flush();
        let _ = self.child.wait();
    }
}

impl ProcessedSemanticRequest {
    fn to_properties(&self, operation: &str, cursor: Option<u32>) -> String {
        let mut output = String::new();
        property(
            &mut output,
            "protocol.version",
            &PROTOCOL_VERSION.to_string(),
        );
        property(&mut output, "operation", operation);
        property(&mut output, "file.name", &self.file_name);
        property(
            &mut output,
            "source.base64",
            &base64::engine::general_purpose::STANDARD.encode(self.source.as_bytes()),
        );
        property(&mut output, "release", &self.release.to_string());
        if let Some(cursor) = cursor {
            property(&mut output, "cursor", &cursor.to_string());
        }
        path_list(&mut output, "classpath", &self.classpath);
        path_list(&mut output, "module.path", &self.module_path);
        path_list(&mut output, "source.path", &self.source_path);
        path_list(&mut output, "processor.path", &self.processor_path);
        string_list(&mut output, "processor.option", &self.processor_options);
        string_list(&mut output, "compiler.option", &self.compiler_options);
        output
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_the_exact_processor_and_compiler_model() {
        let request = ProcessedSemanticRequest {
            file_name: "Example.java".to_owned(),
            source: "class Example {}".to_owned(),
            release: 21,
            classpath: vec![PathBuf::from("/work/classes")],
            module_path: vec![PathBuf::from("/work/modules")],
            source_path: vec![PathBuf::from("/work/src")],
            processor_path: vec![PathBuf::from("/work/lombok.jar")],
            processor_options: vec!["-Amode=strict".to_owned()],
            compiler_options: vec!["--enable-preview".to_owned()],
        };

        let encoded = request.to_properties("query", Some(7));

        assert!(encoded.contains("protocol.version=1"));
        assert!(encoded.contains("operation=query"));
        assert!(encoded.contains("cursor=7"));
        assert!(encoded.contains("processor.path.0=/work/lombok.jar"));
        assert!(encoded.contains("processor.option.0=-Amode\\=strict"));
        assert!(encoded.contains("compiler.option.0=--enable-preview"));
    }
}
