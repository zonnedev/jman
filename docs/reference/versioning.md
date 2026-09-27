# Version identity

JMAN releases are identified by annotated semantic-version Git tags. Static
Cargo and editor manifests retain the matching release version because those
ecosystems require package versions before compilation or publication.

The executable adds source-control identity when it is built from commits
after a release:

| Source state | `jman --version` |
| --- | --- |
| Exact annotated tag `v1.4.2` | `jman 1.4.2` |
| Three commits after `v1.4.2` | `jman 1.4.2+dev.3.g8fa23cd12345` |
| Same commit with tracked changes | `jman 1.4.2+dev.3.g8fa23cd12345.dirty` |

The development suffix is semantic-version build metadata. It identifies the
source precisely but intentionally does not change version precedence. JMAN
does not guess whether the next release will be a patch, minor, or major
version.

Only annotated tags containing valid semantic versions participate in version
discovery. Lightweight and malformed tags are ignored. Discovery follows the
first-parent history so a release tag on a merged topic branch cannot replace
the main release lineage.

When Git metadata is unavailable, such as in an exported source tree, JMAN
uses the Cargo workspace version. Maintainers can set `JMAN_BUILD_VERSION` to
a valid semantic version when an external reproducible build needs an explicit
identity.

Release automation verifies that the annotated tag, Rust workspace, VS Code
manifest, packaged binary, and generated artifacts all carry the exact same
version. Tagged releases never contain a development or dirty suffix.
