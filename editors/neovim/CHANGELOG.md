# Changelog

## Unreleased

## 0.8.1 - 2026-09-26

## 0.8.0 - 2026-09-26

- Added `:JmanFormat`, the `<Leader>jf` mapping, and opt-in synchronous
  `format_on_save` support backed exclusively by JMAN's JJFS formatter.

## 0.7.1 - 2026-09-22

## 0.7.0 - 2026-09-22

## 0.6.0 - 2026-09-20

- Added all-tests, nearest-test, and exact-pattern coverage commands for native
  JMAN workspaces.

## 0.5.2 - 2026-09-20

## 0.5.1 - 2026-09-20

- Added Maven and Gradle check, build, run, and test operations with wrapper
  preference and the language server's compatible build JDK.
- Added LSP-prepared class and method test execution for every supported build
  system while retaining JMAN's live human test report.
- Added richer status reporting, synchronization-state events, configuration
  validation, safer cache clearing, code-action commands, and expanded health
  checks.
- Added direct installation from the JMAN repository, complete user
  documentation, and broader headless integration tests.
- Handle nullable LSP status and command responses without indexing
  `vim.NIL`.
- Prefer enclosing Gradle settings and wrapper roots over nested module build
  files so multi-module workspaces use the correct wrapper and build JDK.
