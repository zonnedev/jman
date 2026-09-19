use std::collections::{BTreeMap, BTreeSet};

use futures::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Advisory, AuditError, AuditPackage, Finding, Severity, VulnerabilityProvider};

pub const OSV_API_URL: &str = "https://api.osv.dev";
const MAX_BATCH_SIZE: usize = 1_000;

#[derive(Clone, Debug)]
pub struct OsvProvider {
    client: reqwest::Client,
    base_url: String,
}

impl OsvProvider {
    /// Create the production OSV provider.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be initialized.
    pub fn new() -> Result<Self, AuditError> {
        Self::with_base_url(OSV_API_URL)
    }

    /// Create an OSV provider for an explicit compatible endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be initialized.
    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, AuditError> {
        let base_url = base_url.into();
        let parsed = reqwest::Url::parse(&base_url).map_err(|error| {
            AuditError::InvalidResponse(format!("invalid OSV base URL `{base_url}`: {error}"))
        })?;
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(AuditError::InvalidResponse(
                "OSV base URL cannot contain credentials, a query, or a fragment".to_owned(),
            ));
        }
        let loopback = parsed
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
        if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
            return Err(AuditError::InvalidResponse(
                "OSV base URL must use HTTPS (HTTP is allowed only for loopback testing)"
                    .to_owned(),
            ));
        }
        let client = reqwest::Client::builder()
            .user_agent(concat!("jman/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|source| AuditError::Request {
                url: "<OSV client initialization>".to_owned(),
                source,
            })?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }

    async fn query_inner(&self, packages: &[AuditPackage]) -> Result<Vec<Finding>, AuditError> {
        let mut identifiers = vec![BTreeSet::new(); packages.len()];
        let mut seen_page_tokens = BTreeSet::new();
        let mut pending = packages
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, package)| PendingQuery {
                index,
                package,
                page_token: None,
            })
            .collect::<Vec<_>>();
        while !pending.is_empty() {
            let mut next = Vec::new();
            for chunk in pending.chunks(MAX_BATCH_SIZE) {
                let response = self.query_batch(chunk).await?;
                if response.results.len() != chunk.len() {
                    return Err(AuditError::InvalidResponse(format!(
                        "OSV returned {} batch results for {} queries",
                        response.results.len(),
                        chunk.len()
                    )));
                }
                for (query, result) in chunk.iter().zip(response.results) {
                    for vulnerability in result.vulns {
                        if vulnerability.id.trim().is_empty() {
                            return Err(AuditError::InvalidResponse(
                                "OSV returned a vulnerability without an ID".to_owned(),
                            ));
                        }
                        identifiers[query.index].insert(vulnerability.id);
                    }
                    if let Some(page_token) = result.next_page_token {
                        if !seen_page_tokens.insert((query.index, page_token.clone())) {
                            return Err(AuditError::InvalidResponse(format!(
                                "OSV repeated a pagination token for {}",
                                query.package.coordinate()
                            )));
                        }
                        next.push(PendingQuery {
                            index: query.index,
                            package: query.package.clone(),
                            page_token: Some(page_token),
                        });
                    }
                }
            }
            pending = next;
        }

        let unique = identifiers
            .iter()
            .flat_map(BTreeSet::iter)
            .cloned()
            .collect::<BTreeSet<_>>();
        let details = stream::iter(unique.into_iter().map(|identifier| async move {
            let record = self.load_advisory(&identifier).await?;
            if record.id != identifier {
                return Err(AuditError::InvalidResponse(format!(
                    "OSV returned advisory `{}` for requested ID `{identifier}`",
                    record.id
                )));
            }
            Ok::<_, AuditError>((identifier, record))
        }))
        .buffer_unordered(8)
        .try_collect::<BTreeMap<_, _>>()
        .await?;
        let mut findings = Vec::new();
        for (index, package) in packages.iter().enumerate() {
            for identifier in &identifiers[index] {
                let record = details.get(identifier).ok_or_else(|| {
                    AuditError::InvalidResponse(format!(
                        "OSV details are missing for advisory {identifier}"
                    ))
                })?;
                if record.withdrawn.is_some() {
                    continue;
                }
                findings.push(Finding {
                    package: package.clone(),
                    advisory: translate_advisory(record, package),
                });
            }
        }
        Ok(findings)
    }

    async fn query_batch(&self, pending: &[PendingQuery]) -> Result<BatchResponse, AuditError> {
        let url = format!("{}/v1/querybatch", self.base_url);
        let request = BatchRequest {
            queries: pending
                .iter()
                .map(|query| OsvQuery {
                    version: &query.package.version,
                    package: OsvPackageQuery {
                        name: query.package.name(),
                        ecosystem: "Maven",
                    },
                    page_token: query.page_token.as_deref(),
                })
                .collect(),
        };
        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|source| AuditError::Request {
                url: url.clone(),
                source,
            })?;
        if !response.status().is_success() {
            return Err(AuditError::HttpStatus {
                url,
                status: response.status(),
            });
        }
        response
            .json()
            .await
            .map_err(|source| AuditError::Request { url, source })
    }

    async fn load_advisory(&self, identifier: &str) -> Result<OsvAdvisory, AuditError> {
        if !identifier
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_' | b'.'))
        {
            return Err(AuditError::InvalidResponse(format!(
                "OSV returned unsafe advisory ID `{identifier}`"
            )));
        }
        let url = format!("{}/v1/vulns/{identifier}", self.base_url);
        let response =
            self.client
                .get(&url)
                .send()
                .await
                .map_err(|source| AuditError::Request {
                    url: url.clone(),
                    source,
                })?;
        if !response.status().is_success() {
            return Err(AuditError::HttpStatus {
                url,
                status: response.status(),
            });
        }
        response
            .json()
            .await
            .map_err(|source| AuditError::Request { url, source })
    }
}

impl VulnerabilityProvider for OsvProvider {
    fn name(&self) -> &'static str {
        "osv"
    }

    fn cache_identity(&self) -> String {
        format!("osv:{}", self.base_url)
    }

    fn query<'a>(
        &'a self,
        packages: &'a [AuditPackage],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<Finding>, AuditError>> + Send + 'a>,
    > {
        Box::pin(self.query_inner(packages))
    }
}

#[derive(Clone, Debug)]
struct PendingQuery {
    index: usize,
    package: AuditPackage,
    page_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct BatchRequest<'a> {
    queries: Vec<OsvQuery<'a>>,
}

#[derive(Debug, Serialize)]
struct OsvQuery<'a> {
    version: &'a str,
    package: OsvPackageQuery,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_token: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct OsvPackageQuery {
    name: String,
    ecosystem: &'static str,
}

#[derive(Debug, Deserialize)]
struct BatchResponse {
    results: Vec<BatchResult>,
}

#[derive(Debug, Deserialize)]
struct BatchResult {
    #[serde(default)]
    vulns: Vec<BatchVulnerability>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BatchVulnerability {
    id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct OsvAdvisory {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    modified: Option<String>,
    #[serde(default)]
    withdrawn: Option<String>,
    #[serde(default)]
    severity: Vec<OsvSeverity>,
    #[serde(default)]
    affected: Vec<OsvAffected>,
    #[serde(default)]
    references: Vec<OsvReference>,
    #[serde(default)]
    database_specific: Value,
}

#[derive(Clone, Debug, Deserialize)]
struct OsvSeverity {
    score: String,
}

#[derive(Clone, Debug, Deserialize)]
struct OsvAffected {
    package: OsvAffectedPackage,
    #[serde(default)]
    severity: Vec<OsvSeverity>,
    #[serde(default)]
    ranges: Vec<OsvRange>,
    #[serde(default)]
    database_specific: Value,
    #[serde(default)]
    ecosystem_specific: Value,
}

#[derive(Clone, Debug, Deserialize)]
struct OsvAffectedPackage {
    ecosystem: String,
    name: String,
}

#[derive(Clone, Debug, Deserialize)]
struct OsvRange {
    #[serde(default)]
    events: Vec<OsvEvent>,
}

#[derive(Clone, Debug, Deserialize)]
struct OsvEvent {
    #[serde(default)]
    fixed: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct OsvReference {
    url: String,
}

fn translate_advisory(record: &OsvAdvisory, package: &AuditPackage) -> Advisory {
    let package_name = package.name();
    let affected = record.affected.iter().filter(|affected| {
        affected.package.ecosystem.eq_ignore_ascii_case("Maven")
            && affected.package.name == package_name
    });
    let mut severity = severity_from_database(&record.database_specific);
    for candidate in &record.severity {
        severity = severity.max(severity_from_score(&candidate.score));
    }
    let mut fixed_versions = Vec::new();
    for affected in affected {
        severity = severity.max(severity_from_database(&affected.database_specific));
        severity = severity.max(severity_from_database(&affected.ecosystem_specific));
        for candidate in &affected.severity {
            severity = severity.max(severity_from_score(&candidate.score));
        }
        fixed_versions.extend(
            affected
                .ranges
                .iter()
                .flat_map(|range| &range.events)
                .filter_map(|event| event.fixed.clone()),
        );
    }
    fixed_versions.sort();
    fixed_versions.dedup();
    let mut aliases = record.aliases.clone();
    aliases.sort();
    aliases.dedup();
    let mut references = record
        .references
        .iter()
        .map(|reference| reference.url.clone())
        .collect::<Vec<_>>();
    references.sort();
    references.dedup();
    let summary = record
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
        .or_else(|| {
            record
                .details
                .as_deref()
                .and_then(|details| details.lines().next())
        })
        .unwrap_or("No advisory summary supplied")
        .trim()
        .to_owned();
    Advisory {
        id: record.id.clone(),
        aliases,
        summary,
        severity,
        fixed_versions,
        references,
        modified: record.modified.clone(),
    }
}

fn severity_from_database(value: &Value) -> Severity {
    value
        .get("severity")
        .and_then(Value::as_str)
        .map_or(Severity::Unknown, Severity::from_label)
}

fn severity_from_score(value: &str) -> Severity {
    value
        .parse::<f64>()
        .ok()
        .or_else(|| cvss_v3_score(value))
        .map_or_else(|| Severity::from_label(value), Severity::from_score)
}

fn cvss_v3_score(vector: &str) -> Option<f64> {
    if !vector.starts_with("CVSS:3.0/") && !vector.starts_with("CVSS:3.1/") {
        return None;
    }
    let metrics = vector
        .split('/')
        .skip(1)
        .filter_map(|metric| metric.split_once(':'))
        .collect::<BTreeMap<_, _>>();
    let scope_changed = *metrics.get("S")? == "C";
    let attack_vector = metric(
        &metrics,
        "AV",
        &[("N", 0.85), ("A", 0.62), ("L", 0.55), ("P", 0.2)],
    )?;
    let complexity = metric(&metrics, "AC", &[("L", 0.77), ("H", 0.44)])?;
    let privileges = if scope_changed {
        metric(&metrics, "PR", &[("N", 0.85), ("L", 0.68), ("H", 0.5)])?
    } else {
        metric(&metrics, "PR", &[("N", 0.85), ("L", 0.62), ("H", 0.27)])?
    };
    let interaction = metric(&metrics, "UI", &[("N", 0.85), ("R", 0.62)])?;
    let confidentiality = metric(&metrics, "C", &[("N", 0.0), ("L", 0.22), ("H", 0.56)])?;
    let integrity = metric(&metrics, "I", &[("N", 0.0), ("L", 0.22), ("H", 0.56)])?;
    let availability = metric(&metrics, "A", &[("N", 0.0), ("L", 0.22), ("H", 0.56)])?;
    let impact_base = 1.0 - (1.0 - confidentiality) * (1.0 - integrity) * (1.0 - availability);
    let impact = if scope_changed {
        7.52 * (impact_base - 0.029) - 3.25 * (impact_base - 0.02).powi(15)
    } else {
        6.42 * impact_base
    };
    if impact <= 0.0 {
        return Some(0.0);
    }
    let exploitability = 8.22 * attack_vector * complexity * privileges * interaction;
    let base = if scope_changed {
        (1.08 * (impact + exploitability)).min(10.0)
    } else {
        (impact + exploitability).min(10.0)
    };
    Some((base * 10.0).ceil() / 10.0)
}

fn metric(metrics: &BTreeMap<&str, &str>, name: &str, values: &[(&str, f64)]) -> Option<f64> {
    let selected = *metrics.get(name)?;
    values
        .iter()
        .find_map(|(value, score)| (*value == selected).then_some(*score))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    use super::*;

    #[test]
    fn translates_osv_maven_advisories_into_provider_neutral_findings() {
        let record: OsvAdvisory = serde_json::from_str(
            r#"{
              "id":"GHSA-test-1234",
              "aliases":["CVE-2026-0001"],
              "modified":"2026-01-02T00:00:00Z",
              "summary":"Unsafe parsing",
              "severity":[{"type":"CVSS_V3","score":"CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"}],
              "affected":[{
                "package":{"ecosystem":"Maven","name":"org.example:library"},
                "ranges":[{"type":"ECOSYSTEM","events":[{"introduced":"0"},{"fixed":"1.2.4"}]}]
              }],
              "references":[{"type":"ADVISORY","url":"https://example.test/advisory"}]
            }"#,
        )
        .expect("OSV fixture");
        let advisory = translate_advisory(
            &record,
            &AuditPackage {
                group: "org.example".to_owned(),
                artifact: "library".to_owned(),
                version: "1.2.3".to_owned(),
            },
        );
        assert_eq!(advisory.id, "GHSA-test-1234");
        assert_eq!(advisory.severity, Severity::Critical);
        assert_eq!(advisory.fixed_versions, ["1.2.4"]);
        assert_eq!(advisory.aliases, ["CVE-2026-0001"]);
    }

    #[test]
    fn normalizes_database_labels_numeric_scores_and_cvss_vectors() {
        assert_eq!(Severity::from_label("MODERATE"), Severity::Medium);
        assert_eq!(severity_from_score("7.5"), Severity::High);
        assert_eq!(
            severity_from_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            Severity::Critical
        );
        assert_eq!(
            severity_from_score("CVSS:3.1/AV:L/AC:H/PR:H/UI:R/S:U/C:L/I:N/A:N"),
            Severity::Low
        );
        assert_eq!(severity_from_score("CVSS:4.0/AV:N"), Severity::Unknown);
        assert_eq!(severity_from_score("moderate"), Severity::Medium);
    }

    #[test]
    fn reads_package_severity_from_ecosystem_specific_metadata() {
        let record: OsvAdvisory = serde_json::from_str(
            r#"{
              "id":"TEST-1",
              "affected":[{
                "package":{"ecosystem":"Maven","name":"org.example:library"},
                "ecosystem_specific":{"severity":"HIGH"}
              }]
            }"#,
        )
        .expect("OSV fixture");
        let advisory = translate_advisory(
            &record,
            &AuditPackage {
                group: "org.example".to_owned(),
                artifact: "library".to_owned(),
                version: "1".to_owned(),
            },
        );
        assert_eq!(advisory.severity, Severity::High);
    }

    #[test]
    fn custom_endpoints_reject_credentials_and_insecure_remote_hosts() {
        assert!(OsvProvider::with_base_url("https://user:secret@example.test").is_err());
        assert!(OsvProvider::with_base_url("http://example.test").is_err());
        assert!(OsvProvider::with_base_url("http://127.0.0.1:1234").is_ok());
    }

    #[tokio::test]
    async fn queries_batches_then_loads_full_advisory_records() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("connection");
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).expect("request bytes");
                    request.extend_from_slice(&buffer[..read]);
                    let headers = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|index| index + 4);
                    let Some(headers) = headers else {
                        continue;
                    };
                    let header_text = String::from_utf8_lossy(&request[..headers]);
                    let content_length = header_text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or_default();
                    if request.len() >= headers + content_length {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let body = if text.starts_with("POST /v1/querybatch") {
                    assert!(text.contains("org.example:library"));
                    r#"{"results":[{"vulns":[{"id":"GHSA-test-1234"}]}]}"#
                } else {
                    assert!(text.starts_with("GET /v1/vulns/GHSA-test-1234"));
                    r#"{"id":"GHSA-test-1234","summary":"fixture","database_specific":{"severity":"HIGH"},"affected":[{"package":{"ecosystem":"Maven","name":"org.example:library"},"ranges":[{"events":[{"fixed":"2.0.0"}]}]}]}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .expect("response");
            }
        });
        let provider =
            OsvProvider::with_base_url(format!("http://{address}")).expect("fixture provider");
        let findings = provider
            .query(&[AuditPackage {
                group: "org.example".to_owned(),
                artifact: "library".to_owned(),
                version: "1.0.0".to_owned(),
            }])
            .await
            .expect("OSV audit");
        server.join().expect("server");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].advisory.severity, Severity::High);
        assert_eq!(findings[0].advisory.fixed_versions, ["2.0.0"]);
    }
}
