# Third-party software notices

JMAN Java contains or bundles third-party open-source software. Each component
remains subject to its own license; the JMAN Apache-2.0 license does not replace
those terms.

## Bundled components

- **vscode-languageclient** and its runtime dependencies — MIT; Copyright
  Microsoft Corporation and contributors;
  <https://github.com/microsoft/vscode-languageserver-node>
- **xml2js** and its runtime dependencies — MIT; Copyright Leonidas
  Tsampros and contributors;
  <https://github.com/Leonidas-from-XIV/node-xml2js>
- **Vineflower** — Apache-2.0; Copyright the Vineflower contributors;
  <https://github.com/Vineflower/vineflower>
- **JaCoCo 0.8.15** — EPL-2.0; Copyright the JaCoCo contributors;
  <https://www.jacoco.org/jacoco/>
- **Rust crates linked into JMAN and the native compiler frontend** — licenses
  and exact versions are recorded by `Cargo.lock`; package sources and license
  metadata are available through <https://crates.io/>.

## JavaScript runtime inventory

| Package | Version | License |
| --- | --- | --- |
| `balanced-match` | 4.0.4 | MIT |
| `brace-expansion` | 5.0.12 | MIT |
| `minimatch` | 10.2.6 | BlueOak-1.0.0 |
| `sax` | 1.6.1 | BlueOak-1.0.0 |
| `semver` | 7.8.5 | ISC |
| `vscode-jsonrpc` | 9.0.2 | MIT |
| `vscode-languageclient` | 10.1.1 | MIT |
| `vscode-languageserver-protocol` | 3.18.3 | MIT |
| `vscode-languageserver-textdocument` | 1.0.14 | MIT |
| `vscode-languageserver-types` | 3.18.3 | MIT |
| `xml2js` | 0.6.2 | MIT |
| `xmlbuilder` | 11.0.1 | MIT |

The exact JavaScript dependency graph is recorded by `package-lock.json`. The
exact Rust dependency graph is recorded by the repository's `Cargo.lock`.
Source code for JMAN and its dependency manifests is available at
<https://github.com/zonnedev/jman>.

The complete Apache License 2.0 text is included in `LICENSE.txt` within the
extension package. MIT and other permissive license texts and copyright notices
remain available in the respective upstream source distributions. This notice
is informational and does not modify any component's license.
