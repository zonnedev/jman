import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).parents[1] / "compare_resolution.py"
SPEC = importlib.util.spec_from_file_location("compare_resolution", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class NormalizationTests(unittest.TestCase):
    def test_maven_and_jman_nodes_normalize_identically(self):
        maven = {
            "groupId": "com.example",
            "artifactId": "app",
            "version": "1",
            "type": "jar",
            "children": [
                {
                    "groupId": "org.example",
                    "artifactId": "library",
                    "version": "2",
                    "type": "jar",
                    "scope": "compile",
                }
            ],
        }
        jman = {
            "root": {
                "group": "com.example",
                "artifact": "app",
                "version": "1",
                "extension": "jar",
                "classifier": None,
            },
            "root_dependencies": ["org.example:library:2"],
            "packages": [
                {
                    "coordinate": {
                        "group": "org.example",
                        "artifact": "library",
                        "version": "2",
                        "extension": "jar",
                        "classifier": None,
                    },
                    "scopes": ["compile"],
                    "parent": "com.example:app:1",
                    "dependencies": [],
                }
            ],
        }

        self.assertEqual(
            MODULE.maven_tree_records(maven),
            MODULE.jman_tree_records(jman),
        )
        self.assertEqual(
            MODULE.maven_tree_records(maven, compare_edges=True),
            MODULE.jman_tree_records(jman, compare_edges=True),
        )

    def test_processor_only_packages_are_not_compared_to_maven_project_tree(self):
        report = {
            "root": {
                "group": "com.example",
                "artifact": "app",
                "version": "1",
                "extension": "jar",
                "classifier": None,
            },
            "root_dependencies": [],
            "packages": [
                {
                    "coordinate": {
                        "group": "org.example",
                        "artifact": "processor",
                        "version": "1",
                        "extension": "jar",
                        "classifier": None,
                    },
                    "scopes": ["processor"],
                    "parent": "com.example:app:1",
                    "dependencies": [],
                }
            ],
        }

        self.assertEqual(MODULE.jman_tree_records(report), [])

    def test_version_mismatch_is_reported_explicitly(self):
        mismatches = MODULE.version_mismatches(
            ["NODE org.example:library:jar:1"],
            ["NODE org.example:library:jar:2"],
        )

        self.assertEqual(
            mismatches,
            ["org.example:library:jar: Maven=1 JMAN=2"],
        )

    def test_performance_comparison_reports_direction_and_ratio(self):
        self.assertEqual(
            MODULE.performance_comparison(2.0, 0.5),
            "JMAN 4.00x faster",
        )
        self.assertEqual(
            MODULE.performance_comparison(0.5, 1.0),
            "JMAN 2.00x slower",
        )

    def test_timing_runs_must_be_positive(self):
        self.assertEqual(MODULE.positive_integer("3"), 3)
        with self.assertRaises(MODULE.argparse.ArgumentTypeError):
            MODULE.positive_integer("0")

    def test_maven_cache_key_changes_with_any_module_pom(self):
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory)
            (project / "pom.xml").write_text("<project/>", encoding="utf-8")
            module = project / "module"
            module.mkdir()
            child = module / "pom.xml"
            child.write_text("<project><version>1</version></project>", encoding="utf-8")
            first = MODULE.maven_cache_key(project, "3.11.0")

            child.write_text("<project><version>2</version></project>", encoding="utf-8")
            second = MODULE.maven_cache_key(project, "3.11.0")

        self.assertNotEqual(first, second)

    def test_maven_cache_key_ignores_kept_conformance_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory)
            (project / "pom.xml").write_text("<project/>", encoding="utf-8")
            first = MODULE.maven_cache_key(project, "3.11.0")
            kept = project / ".jman-conformance-last" / "project"
            kept.mkdir(parents=True)
            (kept / "pom.xml").write_text("<changed/>", encoding="utf-8")

            second = MODULE.maven_cache_key(project, "3.11.0")

        self.assertEqual(first, second)

    def test_default_maven_cache_is_project_scoped_and_repository_local(self):
        self.assertEqual(
            MODULE.default_maven_cache_dir(Path("/repo"), "demo"),
            Path("/repo/.local/benchmark/demo/maven-cache"),
        )

    def test_maven_cache_round_trip_and_corruption_recovery(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "baseline.json"
            report = {
                "groupId": "com.example",
                "artifactId": "app",
                "version": "1",
            }
            MODULE.write_maven_cache(
                path,
                key="key",
                report=report,
                cold=3.0,
                resolution=2.0,
                no_change=1.0,
            )

            cached = MODULE.load_maven_cache(path, "key")
            self.assertIsNotNone(cached)
            self.assertEqual(cached["report"], report)
            self.assertIsNone(MODULE.load_maven_cache(path, "different"))
            path.write_text("{broken", encoding="utf-8")
            self.assertIsNone(MODULE.load_maven_cache(path, "key"))

    def test_reactor_modules_are_discovered_in_declared_order(self):
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory)
            child = project / "child"
            child.mkdir()
            (project / "jman.toml").write_text(
                '[project]\nmodules = ["child"]\n', encoding="utf-8"
            )
            (child / "jman.toml").write_text("[project]\n", encoding="utf-8")

            modules = MODULE.reactor_modules(project)

        self.assertEqual([name for name, _ in modules], [".", "child"])

    def test_project_copy_drops_existing_jman_generated_files(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source"
            destination = Path(directory) / "destination"
            source.mkdir()
            destination.mkdir()
            (source / "pom.xml").write_text("<project/>", encoding="utf-8")
            (source / "jman.toml").write_text("stale", encoding="utf-8")
            (source / "jman.lock").write_text("stale", encoding="utf-8")

            copied = MODULE.copy_project(source, destination)

            self.assertTrue((copied / "pom.xml").is_file())
            self.assertFalse((copied / "jman.toml").exists())
            self.assertFalse((copied / "jman.lock").exists())

    def test_reactor_records_keep_module_graphs_isolated(self):
        reports = {
            "one": {"value": "a"},
            "two": {"value": "a"},
        }

        records = MODULE.reactor_records(
            reports,
            lambda report, _edges: [f"NODE {report['value']}"],
            False,
        )

        self.assertEqual(records, ["one NODE a", "two NODE a"])

    def test_maven_reactor_uses_root_workspace_for_child_resolution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            project = root / "project"
            child = project / "child"
            output = root / "output"
            repository = root / "repository"
            child.mkdir(parents=True)
            output.mkdir()
            (project / "pom.xml").write_text("<project/>", encoding="utf-8")
            (child / "pom.xml").write_text("<project/>", encoding="utf-8")
            commands: list[list[str]] = []

            def fake_run(command, **_kwargs):
                commands.append(command)
                output_argument = next(
                    argument
                    for argument in command
                    if argument.startswith("-DoutputFile=")
                )
                Path(output_argument.split("=", 1)[1]).write_text(
                    '{"groupId":"com.example","artifactId":"child","version":"1"}',
                    encoding="utf-8",
                )
                return MODULE.CommandResult("", "", 0.0)

            with patch.object(MODULE, "run_checked", side_effect=fake_run):
                MODULE.run_maven_reactor(
                    ["mvn"],
                    project,
                    [(".", project), ("child", child)],
                    repository,
                    "dependency:tree",
                    output,
                )

        child_command = commands[0]
        self.assertEqual(
            child_command[
                child_command.index("-f") : child_command.index("-f") + 5
            ],
            ["-f", str(project / "pom.xml"), "-pl", "child", "-am"],
        )


if __name__ == "__main__":
    unittest.main()
