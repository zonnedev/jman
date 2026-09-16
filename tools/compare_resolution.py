#!/usr/bin/env python3
"""Compare JMAN's native dependency resolution with Maven's resolved tree."""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import tomllib
from typing import Any, Iterable, NamedTuple

DEFAULT_MAVEN_DEPENDENCY_PLUGIN = "3.11.0"
MAVEN_CACHE_SCHEMA = 3


class CommandResult(NamedTuple):
    stdout: str
    stderr: str
    elapsed_seconds: float


def canonical_coordinate(
    group: str,
    artifact: str,
    extension: str,
    version: str,
    classifier: str | None = None,
) -> str:
    if classifier:
        return f"{group}:{artifact}:{extension}:{classifier}:{version}"
    return f"{group}:{artifact}:{extension}:{version}"


def maven_tree_records(
    root: dict[str, Any], compare_edges: bool = False
) -> list[str]:
    records: set[str] = set()

    def visit(node: dict[str, Any], parent: str | None) -> None:
        coordinate = canonical_coordinate(
            node["groupId"],
            node["artifactId"],
            node.get("type") or "jar",
            node["version"],
            node.get("classifier") or None,
        )
        if parent is not None:
            records.add(f"NODE {coordinate}")
            if compare_edges:
                records.add(f"EDGE {parent} -> {coordinate}")
        for child in node.get("children", []):
            visit(child, coordinate)

    visit(root, None)
    return sorted(records)


def jman_tree_records(report: dict[str, Any], compare_edges: bool = False) -> list[str]:
    root = report["root"]
    root_coordinate = canonical_coordinate(
        root["group"],
        root["artifact"],
        root.get("extension") or "jar",
        root["version"],
        root.get("classifier"),
    )
    packages = [
        package
        for package in report["packages"]
        if set(package.get("scopes", [])) != {"processor"}
    ]
    by_coordinate = {
        jman_coordinate(package["coordinate"]): package for package in packages
    }
    records = {f"NODE {coordinate}" for coordinate in by_coordinate}
    if compare_edges:
        for child, package in by_coordinate.items():
            parent = normalize_jman_coordinate(package["parent"])
            if parent == root_coordinate or parent in by_coordinate:
                records.add(f"EDGE {parent} -> {child}")
    return sorted(records)


def jman_coordinate(coordinate: dict[str, Any]) -> str:
    return canonical_coordinate(
        coordinate["group"],
        coordinate["artifact"],
        coordinate.get("extension") or "jar",
        coordinate["version"],
        coordinate.get("classifier"),
    )


def normalize_jman_coordinate(value: str) -> str:
    parts = value.split(":")
    if len(parts) == 3:
        group, artifact, version = parts
        return canonical_coordinate(group, artifact, "jar", version)
    if len(parts) == 4:
        group, artifact, extension, version = parts
        return canonical_coordinate(group, artifact, extension, version)
    if len(parts) == 5:
        group, artifact, extension, classifier, version = parts
        return canonical_coordinate(group, artifact, extension, version, classifier)
    raise ValueError(f"unsupported JMAN coordinate: {value}")


def run_checked(
    command: list[str],
    *,
    cwd: Path,
    environment: dict[str, str] | None = None,
) -> CommandResult:
    started = time.perf_counter()
    result = subprocess.run(
        command,
        cwd=cwd,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    elapsed_seconds = time.perf_counter() - started
    if result.returncode != 0:
        rendered = " ".join(command)
        raise RuntimeError(
            f"command failed ({result.returncode}): {rendered}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return CommandResult(result.stdout, result.stderr, elapsed_seconds)


def find_jman(repo_root: Path, requested: str | None) -> Path:
    if requested:
        binary = Path(requested).expanduser().resolve()
    else:
        binary = repo_root / "target" / "release" / "jman"
    if not binary.is_file():
        run_checked(
            ["cargo", "build", "--release", "-p", "jman-cli"],
            cwd=repo_root,
        )
    if not binary.is_file():
        raise RuntimeError(f"jman binary was not found at {binary}")
    return binary


def maven_command(project: Path, requested: str | None) -> list[str]:
    if requested:
        return [requested]
    wrapper = project / ("mvnw.cmd" if os.name == "nt" else "mvnw")
    if wrapper.is_file():
        if os.name != "nt" and not os.access(wrapper, os.X_OK):
            return ["sh", str(wrapper)]
        return [str(wrapper)]
    executable = shutil.which("mvn")
    if executable:
        return [executable]
    raise RuntimeError("neither a Maven wrapper nor `mvn` was found")


def copy_project(source: Path, destination: Path) -> Path:
    project = destination / "project"
    shutil.copytree(
        source,
        project,
        ignore=shutil.ignore_patterns(
            ".git",
            ".jman",
            ".jman-conformance-last",
            "target",
            "build",
            "jman.toml",
            "jman.lock",
        ),
    )
    return project


def reactor_modules(project: Path) -> list[tuple[str, Path]]:
    root = project.resolve()
    pending = [(".", root)]
    discovered: set[Path] = set()
    modules: list[tuple[str, Path]] = []
    while pending:
        name, directory = pending.pop(0)
        canonical = directory.resolve()
        if canonical in discovered:
            raise RuntimeError(f"duplicate or cyclic JMAN module: {canonical}")
        if not canonical.is_relative_to(root):
            raise RuntimeError(f"JMAN module escapes workspace: {canonical}")
        discovered.add(canonical)
        manifest_path = canonical / "jman.toml"
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        modules.append((name, canonical))
        for child in manifest.get("project", {}).get("modules", []):
            child_path = (canonical / child).resolve()
            child_name = child_path.relative_to(root).as_posix()
            pending.append((child_name, child_path))
    return modules


def run_jman_reactor(
    jman: Path,
    repo_root: Path,
    modules: list[tuple[str, Path]],
    environment: dict[str, str],
    *,
    refresh: bool,
) -> tuple[dict[str, dict[str, Any]], float]:
    started = time.perf_counter()
    reports = {}
    for name, directory in modules:
        command = [str(jman), "sync", str(directory), "--report", "json"]
        if refresh:
            command.append("--refresh")
        result = run_checked(command, cwd=repo_root, environment=environment)
        reports[name] = json.loads(result.stdout)
    return reports, time.perf_counter() - started


def run_maven_reactor(
    maven: list[str],
    project: Path,
    modules: list[tuple[str, Path]],
    repository: Path,
    goal: str,
    output_directory: Path,
) -> tuple[dict[str, dict[str, Any]], float]:
    started = time.perf_counter()
    reports = {}
    for index, (name, directory) in enumerate(modules):
        if name == "." and len(modules) > 1:
            reports[name] = {
                "groupId": "reactor",
                "artifactId": "aggregator",
                "version": "0",
                "type": "pom",
                "children": [],
            }
            continue
        output = output_directory / f"maven-tree-{index}.json"
        if len(modules) > 1:
            project_selection = [
                "-f",
                str(project / "pom.xml"),
                "-pl",
                name,
                "-am",
            ]
        else:
            project_selection = ["-f", str(directory / "pom.xml")]
        run_checked(
            [
                *maven,
                "-q",
                f"-Dmaven.repo.local={repository}",
                *project_selection,
                goal,
                "-DoutputType=json",
                f"-DoutputFile={output}",
            ],
            cwd=project,
        )
        reports[name] = json.loads(output.read_text(encoding="utf-8"))
    return reports, time.perf_counter() - started


def reactor_records(
    reports: dict[str, dict[str, Any]],
    normalizer: Any,
    compare_edges: bool,
) -> list[str]:
    return sorted(
        f"{module} {record}"
        for module, report in reports.items()
        for record in normalizer(report, compare_edges)
    )


def default_maven_cache_dir(repo_root: Path, project_name: str) -> Path:
    return repo_root / ".local" / "benchmark" / project_name / "maven-cache"


def maven_cache_key(project: Path, plugin_version: str) -> str:
    digest = hashlib.sha256()
    digest.update(f"schema={MAVEN_CACHE_SCHEMA}\n".encode())
    digest.update(f"plugin={plugin_version}\n".encode())
    inputs = [
        path
        for path in project.rglob("*")
        if path.is_file()
        and (
            path.name == "pom.xml"
            or path.name in {"mvnw", "mvnw.cmd", "maven-wrapper.properties"}
        )
        and not {".git", "target", "build", ".jman", ".jman-conformance-last"}.intersection(
            path.relative_to(project).parts
        )
    ]
    for path in sorted(inputs):
        relative = path.relative_to(project).as_posix()
        digest.update(relative.encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def load_maven_cache(path: Path, expected_key: str) -> dict[str, Any] | None:
    try:
        cached = json.loads(path.read_text(encoding="utf-8"))
        if (
            cached.get("schema") == MAVEN_CACHE_SCHEMA
            and cached.get("key") == expected_key
            and isinstance(cached.get("report"), dict)
            and all(
                isinstance(cached.get("timings", {}).get(name), (int, float))
                for name in ("cold", "resolution", "no_change")
            )
        ):
            return cached
    except (OSError, json.JSONDecodeError):
        pass
    return None


def write_maven_cache(
    path: Path,
    *,
    key: str,
    report: dict[str, Any],
    cold: float,
    resolution: float,
    no_change: float,
) -> None:
    payload = {
        "schema": MAVEN_CACHE_SCHEMA,
        "key": key,
        "created_at": int(time.time()),
        "timings": {
            "cold": cold,
            "resolution": resolution,
            "no_change": no_change,
        },
        "report": report,
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(f".tmp-{os.getpid()}")
    temporary.write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def version_mismatches(
    maven_records: Iterable[str], jman_records: Iterable[str]
) -> list[str]:
    def versions(records: Iterable[str]) -> dict[str, str]:
        result: dict[str, str] = {}
        for record in records:
            if record.startswith("NODE "):
                coordinate = record.removeprefix("NODE ")
                module = ""
            elif " NODE " in record:
                module, coordinate = record.split(" NODE ", 1)
                module = f"{module} "
            else:
                continue
            identity, version = coordinate.rsplit(":", 1)
            result[f"{module}{identity}"] = version
        return result

    maven = versions(maven_records)
    jman = versions(jman_records)
    return [
        f"{identity}: Maven={maven[identity]} JMAN={jman[identity]}"
        for identity in sorted(maven.keys() & jman.keys())
        if maven[identity] != jman[identity]
    ]


def performance_comparison(maven_seconds: float, jman_seconds: float) -> str:
    if maven_seconds == 0 and jman_seconds == 0:
        return "equal at timer resolution"
    if jman_seconds == 0:
        return "JMAN faster (below timer resolution)"
    if maven_seconds == 0:
        return "JMAN slower (Maven below timer resolution)"
    if jman_seconds <= maven_seconds:
        return f"JMAN {maven_seconds / jman_seconds:.2f}x faster"
    return f"JMAN {jman_seconds / maven_seconds:.2f}x slower"


def print_timing_summary(
    *,
    runs: int,
    maven_cold: list[float],
    jman_cold: list[float],
    maven_resolution: list[float],
    jman_resolution: list[float],
    maven_no_change: list[float],
    jman_no_change: list[float],
    maven_cached: bool,
) -> None:
    cold_maven = min(maven_cold)
    cold_jman = statistics.median(jman_cold)
    resolution_maven = min(maven_resolution)
    resolution_jman = statistics.median(jman_resolution)
    no_change_maven = min(maven_no_change)
    no_change_jman = statistics.median(jman_no_change)
    maven_label = (
        "cached best Maven baseline"
        if maven_cached
        else "best observed Maven baseline"
    )
    print(
        f"Timing (JMAN median of {runs} current wall-clock "
        f"run{'s' if runs != 1 else ''}; {maven_label}):"
    )
    print(
        f"  Cold cache: Maven {cold_maven:.3f}s | JMAN {cold_jman:.3f}s "
        f"| {performance_comparison(cold_maven, cold_jman)}"
    )
    print(
        f"  Re-resolve: Maven {resolution_maven:.3f}s | JMAN {resolution_jman:.3f}s "
        f"| {performance_comparison(resolution_maven, resolution_jman)}"
    )
    print(
        f"  No change:  Maven {no_change_maven:.3f}s | JMAN {no_change_jman:.3f}s "
        f"| {performance_comparison(no_change_maven, no_change_jman)}"
    )
    print()


def compare(args: argparse.Namespace) -> int:
    repo_root = Path(__file__).resolve().parents[1]
    source_project = args.project.expanduser().resolve()
    if not (source_project / "pom.xml").is_file():
        raise RuntimeError(f"{source_project} does not contain pom.xml")
    jman = find_jman(repo_root, args.jman)
    cache_key = maven_cache_key(source_project, args.maven_plugin_version)
    cache_root = (
        args.maven_cache.expanduser().resolve()
        if args.maven_cache
        else default_maven_cache_dir(repo_root, source_project.name)
    )
    cache_path = cache_root / f"{cache_key}.json"
    existing_maven = load_maven_cache(cache_path, cache_key)
    cached_maven = None if args.refresh_maven else existing_maven

    temporary_context = tempfile.TemporaryDirectory(prefix="jman-conformance-")
    temporary = Path(temporary_context.name)
    try:
        project = copy_project(source_project, temporary)
        setup_environment = os.environ.copy()
        setup_cache = temporary / "setup-jman-cache"
        setup_environment["JMAN_CACHE_DIR"] = str(setup_cache)
        if not (project / "jman.toml").is_file():
            run_checked(
                [str(jman), "init", "--import", str(project)],
                cwd=repo_root,
                environment=setup_environment,
            )
        modules = reactor_modules(project)
        benchmark_modules = modules if len(modules) == 1 else modules[1:]
        maven = maven_command(project, args.maven) if cached_maven is None else []
        goal = (
            "org.apache.maven.plugins:maven-dependency-plugin:"
            f"{args.maven_plugin_version}:tree"
        )

        maven_cold: list[float] = (
            [] if cached_maven is None else [cached_maven["timings"]["cold"]]
        )
        jman_cold: list[float] = []
        maven_resolution: list[float] = (
            []
            if cached_maven is None
            else [cached_maven["timings"]["resolution"]]
        )
        jman_resolution: list[float] = []
        maven_no_change: list[float] = (
            []
            if cached_maven is None
            else [cached_maven["timings"]["no_change"]]
        )
        jman_no_change: list[float] = []
        jman_reports: dict[str, dict[str, Any]] | None = None
        maven_reports: dict[str, dict[str, Any]] | None = None
        for sample in range(args.timing_runs):
            sample_root = temporary / "benchmark-caches" / str(sample + 1)
            jman_cache = sample_root / "jman"
            maven_cache = sample_root / "maven"
            environment = os.environ.copy()
            environment["JMAN_CACHE_DIR"] = str(jman_cache)
            if cached_maven is None:
                maven_reports, elapsed = run_maven_reactor(
                    maven,
                    project,
                    benchmark_modules,
                    maven_cache,
                    goal,
                    temporary,
                )
                maven_cold.append(elapsed)
            jman_reports, elapsed = run_jman_reactor(
                jman, repo_root, benchmark_modules, environment, refresh=True
            )
            jman_cold.append(elapsed)

            if cached_maven is None:
                maven_reports, elapsed = run_maven_reactor(
                    maven,
                    project,
                    benchmark_modules,
                    maven_cache,
                    goal,
                    temporary,
                )
                maven_resolution.append(elapsed)
            jman_reports, elapsed = run_jman_reactor(
                jman, repo_root, benchmark_modules, environment, refresh=True
            )
            jman_resolution.append(elapsed)

            if cached_maven is None:
                maven_reports, elapsed = run_maven_reactor(
                    maven,
                    project,
                    benchmark_modules,
                    maven_cache,
                    goal,
                    temporary,
                )
                maven_no_change.append(elapsed)
            jman_reports, elapsed = run_jman_reactor(
                jman, repo_root, benchmark_modules, environment, refresh=False
            )
            jman_no_change.append(elapsed)

        assert jman_reports is not None
        if cached_maven is None:
            assert maven_reports is not None
            maven_report = {"modules": maven_reports}
            previous_timings = (
                existing_maven["timings"] if existing_maven is not None else {}
            )
            best_cold = min(
                min(maven_cold), previous_timings.get("cold", float("inf"))
            )
            best_resolution = min(
                min(maven_resolution),
                previous_timings.get("resolution", float("inf")),
            )
            best_no_change = min(
                min(maven_no_change),
                previous_timings.get("no_change", float("inf")),
            )
            write_maven_cache(
                cache_path,
                key=cache_key,
                report=maven_report,
                cold=best_cold,
                resolution=best_resolution,
                no_change=best_no_change,
            )
            maven_cold = [best_cold]
            maven_resolution = [best_resolution]
            maven_no_change = [best_no_change]
            print(f"Maven baseline: refreshed and cached at {cache_path}")
        else:
            maven_report = cached_maven["report"]
            print(f"Maven baseline: cache hit at {cache_path}")

        compared_maven_reports = {
            name: maven_report["modules"][name]
            for name, _ in benchmark_modules
        }
        maven_records = reactor_records(
            compared_maven_reports, maven_tree_records, args.compare_edges
        )
        jman_records = reactor_records(
            jman_reports, jman_tree_records, args.compare_edges
        )
        if maven_records == jman_records:
            print_timing_summary(
                runs=args.timing_runs,
                maven_cold=maven_cold,
                jman_cold=jman_cold,
                maven_resolution=maven_resolution,
                jman_resolution=jman_resolution,
                maven_no_change=maven_no_change,
                jman_no_change=jman_no_change,
                maven_cached=cached_maven is not None,
            )
            print(
                f"MATCH: {len(benchmark_modules)} module graph(s), "
                f"{len([r for r in jman_records if ' NODE ' in r])} "
                "resolved module dependencies are identical"
            )
            return 0

        print("Timing suppressed: Maven and JMAN dependency graphs differ.\n")
        mismatches = version_mismatches(maven_records, jman_records)
        if mismatches:
            print("Version mismatches:")
            for mismatch in mismatches:
                print(f"  {mismatch}")
            print()
        print(
            "\n".join(
                difflib.unified_diff(
                    maven_records,
                    jman_records,
                    fromfile="maven",
                    tofile="jman",
                    lineterm="",
                )
            )
        )
        return 1
    finally:
        if args.keep:
            kept = source_project / ".jman-conformance-last"
            if kept.exists():
                shutil.rmtree(kept)
            shutil.copytree(
                temporary,
                kept,
                ignore=shutil.ignore_patterns(
                    "benchmark-caches", "setup-jman-cache"
                ),
            )
            print(f"Kept conformance artifacts at {kept}", file=sys.stderr)
        temporary_context.cleanup()


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project", type=Path, help="Maven project directory")
    parser.add_argument(
        "--jman",
        dest="jman",
        help="path to the jman executable",
    )
    parser.add_argument("--maven", help="path to mvn or mvnw")
    parser.add_argument(
        "--maven-plugin-version",
        default=DEFAULT_MAVEN_DEPENDENCY_PLUGIN,
        help="pinned Maven Dependency Plugin version",
    )
    parser.add_argument(
        "--maven-cache",
        type=Path,
        help="directory for persistent Maven tree and timing baselines",
    )
    parser.add_argument(
        "--refresh-maven",
        action="store_true",
        help="ignore and replace the cached Maven baseline",
    )
    parser.add_argument(
        "--timing-runs",
        type=positive_integer,
        default=3,
        metavar="N",
        help="number of samples used for median timing (default: 3)",
    )
    parser.add_argument(
        "--compare-edges",
        action="store_true",
        help="also compare normalized parent-child edges",
    )
    parser.add_argument(
        "--keep",
        action="store_true",
        help="copy raw reports to PROJECT/.jman-conformance-last",
    )
    return parser.parse_args(argv)


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return parsed


def main() -> int:
    try:
        return compare(parse_args())
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
