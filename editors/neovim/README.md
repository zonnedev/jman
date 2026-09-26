# JMAN for Neovim

Native Neovim 0.11+ Java support for JMAN, Maven, and Gradle workspaces. The
plugin uses Neovim's built-in LSP client and the same JMAN language server,
project model, refactorings, build synchronization, and test descriptors as the
VS Code extension.

## Requirements

- Neovim 0.11 or newer.
- A `jman` release on `PATH`, `JMAN_BIN`, or configured through `cmd`.
- A Java workspace containing `jman.toml`, Gradle build files, or `pom.xml`.

Run `:checkhealth jman` after installation to verify the executable, Java
environment, configured build JDK, and current workspace detection.

## Installation

The repository contains a small runtime bridge, so plugin managers can install
JMAN directly from the main repository.

With lazy.nvim:

```lua
{
  "zonnedev/jman",
  ft = "java",
  opts = {
    build_sync = "prompt",
  },
}
```

With Neovim 0.12's built-in package manager:

```lua
vim.pack.add({ "https://github.com/zonnedev/jman" })
require("jman").setup()
```

On Neovim 0.11, use lazy.nvim or clone the repository under a native
`pack/*/start/` directory.

For a local checkout:

```lua
vim.opt.runtimepath:prepend("/path/to/jman/editors/neovim")
require("jman").setup({ cmd = "/path/to/jman/target/release/jman" })
```

## Configuration

```lua
require("jman").setup({
  cmd = nil,                 -- absolute executable; falls back to JMAN_BIN/PATH
  java_home = nil,           -- Java home inherited by the language server
  build_java_home = nil,     -- optional Maven/Gradle runtime override
  build_system = "auto",     -- auto, jman, gradle, or maven
  build_sync = "prompt",     -- manual, prompt, or automatic
  format_on_save = false,     -- format Java buffers with JJFS before writing
  format_timeout_ms = 5000,   -- synchronous formatting timeout
  extra_env = {},            -- additional language-server and task environment
  notify = true,
  keymaps = true,
})
```

Automatic workspace detection prefers `jman.toml`, followed by Gradle and
Maven. An explicit `build_system` restricts detection to that model. Maven and
Gradle operations prefer project-local `mvnw` and `gradlew` wrappers and use the
compatible build JDK selected by the language server.

## Commands

| Command | Purpose |
| --- | --- |
| `:JmanStatus` | Show indexing, semantic, sync, cache, and build-runtime status |
| `:JmanSync` | Synchronize the native or external project model |
| `:JmanCheck` | Check with JMAN, Gradle, or Maven |
| `:JmanBuild` | Build with JMAN, Gradle, or Maven |
| `:JmanRun` | Run with JMAN or Gradle when supported |
| `:JmanTest` | Run all tests with the active build system |
| `:JmanTests` | Discover and select a test class or method through the LSP |
| `:JmanTestPattern [selector]` | Run an exact LSP-prepared test selector |
| `:JmanTestNearest` | Run the test nearest the cursor |
| `:JmanCoverage` | Run all native JMAN tests with coverage |
| `:JmanCoveragePattern [selector]` | Cover an exact LSP-prepared test selector |
| `:JmanCoverageNearest` | Cover the test nearest the cursor |
| `:JmanCodeAction` | Select a JMAN code action |
| `:JmanFormat` | Format the current Java buffer with JJFS |
| `:JmanOrganizeImports` | Apply the organize-imports source action |
| `:JmanChangeSignature` | Change a method signature and update call sites |
| `:JmanRebuildIndex` | Rebuild the workspace index |
| `:JmanClearCache` | Confirm, clear, and rebuild the workspace cache |
| `:JmanRestartLsp` | Restart JMAN Java clients |

JUnit classes and methods receive standard LSP CodeLens actions. Running a
CodeLens, selecting a test, or using `:JmanTestNearest` asks the server for the
correct JMAN, Maven, or Gradle invocation and build JDK. Native JMAN tests keep
their live human-readable test tree in the terminal.

## Default keymaps

| Mapping | Action |
| --- | --- |
| `<leader>js` | Synchronize |
| `<leader>jc` | Check |
| `<leader>jb` | Build |
| `<leader>jr` | Run |
| `<leader>jt` | Test nearest |
| `<leader>jT` | Test all |
| `<leader>jv` | Cover nearest test |
| `<leader>jV` | Cover all tests |
| `<leader>jl` | Select test |
| `<leader>ji` | Show status |
| `<leader>ja` | Code action |
| `<leader>jf` | Format with JJFS |
| `<leader>jo` | Organize imports |

Set `keymaps = false` to leave mappings entirely to your configuration.
Set `format_on_save = true` to run the same full-document JJFS formatter before
each Java buffer is written. Formatting is restricted to the attached JMAN
client, so another LSP cannot provide competing edits.

## Statusline

`require("jman").status()` returns a compact value such as `JMAN:ready`,
`JMAN:required`, or `JMAN:off`. The plugin emits the `User JmanStatusChanged`
autocommand whenever synchronization state changes, allowing statusline plugins
to refresh without polling.

```lua
vim.o.statusline = "%f %m %= %{v:lua.require'jman'.status()}"
```

See `:help jman.nvim` for the complete in-editor reference.
