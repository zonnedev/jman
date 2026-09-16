# Resolver conformance tools

Compare JMAN's native resolver with Maven's selected dependency tree:

```bash
python3 tools/compare_resolution.py /path/to/maven/project
```

The harness works on a temporary project copy, runs `jman sync --report json`,
runs a pinned Maven Dependency Plugin `dependency:tree` JSON report, and compares
normalized group, artifact, type, classifier, and version coordinates. It also
reports median wall-clock time and the Maven-to-JMAN performance ratio for:

- a genuinely cold run using a new empty Maven local repository and JMAN cache
  for each sample;
- warm re-resolution using those populated caches (`jman --refresh` versus
  Maven's normal `dependency:tree` execution); and
- an unchanged-project rerun, which exercises JMAN's lockfile fast path.

For Maven reactors, the harness imports and compares every non-aggregator module
independently. The cached baseline contains one tree per module, and timing
ratios are suppressed unless every module graph matches exactly.

Three samples are collected by default. Use `--timing-runs N` to change the
sample count; use at least three for less noisy performance comparisons. Cold
Maven timing includes downloading the pinned Dependency Plugin into its empty
local repository, because Maven needs that plugin to produce the comparison
tree. Project import/setup is excluded from all measurements.
The harness builds and benchmarks the release-mode `jman` binary by default.

Maven is expensive, so a successful Maven tree and its best observed timings
are stored under
`.local/benchmark/<project-name>/maven-cache/`. The key includes all
project/module POMs, Maven wrapper configuration, and the Dependency Plugin
version. Later runs reuse that baseline and execute only jman. Use
`--refresh-maven` to rerun all Maven samples and replace the baseline, or
`--maven-cache PATH` to choose a different cache directory. Output always
distinguishes a cached historical Maven baseline from current jman measurements.
Refreshing replaces the tree but retains the faster timing for each scenario
when the prior cache key is still valid.

Exit codes:

- `0`: exact match
- `1`: dependency difference
- `2`: harness or tool failure

Use `--compare-edges` to include parent-child relationships and `--keep` to copy
the raw reports to `.jman-conformance-last` under the tested project.
