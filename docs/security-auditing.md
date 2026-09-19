# Security auditing

`jman audit` checks every external package in the current `jman.lock` graph,
including transitive dependencies. It is read-only and requires current
lockfiles; run `jman sync` first when the workspace manifests have changed.

```bash
jman audit
jman audit path/to/workspace
jman audit --severity high
jman audit --deny high
jman audit --refresh
jman audit --offline
jman audit --format json
```

The default report includes every known finding and exits successfully.
`--severity` hides findings below the selected display threshold without
changing policy evaluation. `--deny` makes the command exit unsuccessfully when
an active finding meets or exceeds its threshold, which provides an explicit
and reviewable CI policy. Supported levels are `unknown`, `low`, `medium`,
`high`, and `critical`; `--deny unknown` rejects every active finding.

Each finding includes its resolved Maven coordinate, advisory identifiers,
summary, normalized severity, known fixed versions, references, and the
shortest dependency path from each affected workspace module. JSON output is
stable and includes counters for total, displayed, active, suppressed, and
policy-denied findings.

## Provider and cache boundary

The `jman-audit` crate owns provider-neutral package, advisory, severity,
finding, and cache models. The initial adapter uses OSV's batch query endpoint
to discover advisory IDs and then retrieves the full OSV records. Maven
coordinates are submitted using the OSV `Maven` ecosystem name.

Database severity labels and numeric scores are normalized directly. CVSS 3.0
and 3.1 vectors are scored locally. Records that provide neither supported
scores nor recognized database labels remain `unknown`; JMAN does not invent a
severity. Withdrawn records are excluded.

Results for the exact sorted dependency graph are cached for one hour under
`$JMAN_CACHE_DIR/audit/osv/`, normally `~/.cache/jman/audit/osv/`. A normal
audit uses a fresh cache entry, `--refresh` forces provider revalidation, and a
provider failure falls back to a valid stale entry with a warning. `--offline`
never contacts the provider and fails when the exact graph has no cached
result. `JMAN_AUDIT_OSV_URL` may point to an OSV-compatible HTTPS service;
plain HTTP is accepted only for loopback testing.

## Suppressions

Suppressions belong in the root `jman.toml`. Every entry requires an advisory
ID or alias, a non-empty justification, and a valid calendar expiry date:

```toml
[audit]

[[audit.suppressions]]
id = "CVE-2026-12345"
reason = "The affected parser is not reachable from this deployment"
expires = "2026-12-31"
```

The suppression remains valid through its expiry date. Expired entries no
longer suppress findings and produce a warning. Active entries that match no
finding also produce a warning, preventing obsolete exceptions from silently
accumulating. Suppressed findings remain visible in both human and JSON reports
and never contribute to `--deny` failures.
