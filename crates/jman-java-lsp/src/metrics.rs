use serde_json::Value;

pub(crate) fn emit(event: &str, fields: Value) {
    if std::env::var_os("JMAN_JAVA_LSP_TELEMETRY").is_none() {
        return;
    }
    eprintln!(
        "{}",
        serde_json::json!({
            "jman.javaMetric": event,
            "fields": fields
        })
    );
}
