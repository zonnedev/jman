# Compatibility matrix

The matrix is an executable contract, not only a documentation table. Run it
with:

```bash
make test-compatibility-matrix
```

| Surface | Verified versions |
| --- | --- |
| Java source semantics | 8, 11, 17, 21, 25, plus Java 25 preview |
| Build JVM | Temurin 17, OpenJDK 21, OpenJDK 25 |
| Maven | 3.9.9 on JDK 17, 21, and 25 |
| Gradle | 8.7 and 8.14.1 on JDK 17/21; 9.1.0 on JDK 17/21/25 |
| JPMS | named/automatic modules, transitive readability, qualified exports/opens, services/providers, compiler overrides, split/duplicate detection, unsaved descriptors, same-module sources, multi-release modular JARs |

Each build-tool cell imports a real minimal modular project and asserts the
normalized compile model's release, source ownership, module descriptor,
module path, project-dependency identity across the Gradle 8.11 API boundary,
and resolution status. The frontend test suite separately parses
syntax at every listed language release and validates Java 25 preview parsing.

This is the currently tested floor, not a claim that unlisted versions are
incompatible. A version is added to the supported table only after its matrix
cell is reproducible in CI/development environments.
