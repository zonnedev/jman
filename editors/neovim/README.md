# JMAN for Neovim

Native Neovim 0.11+ integration for the JMAN Java build tool and language server.

```lua
{
  dir = "/path/to/jman/editors/neovim",
  ft = "java",
  opts = {
    cmd = "/path/to/jman",
    build_sync = "prompt", -- manual, prompt, or automatic
  },
}
```

The plugin prefers `jman.toml` over Gradle and Maven markers, starts
`jman lsp --stdio`, and uses Neovim's built-in LSP client.

## Commands

- `:JmanStatus`
- `:JmanSync`
- `:JmanCheck`
- `:JmanBuild`
- `:JmanRun`
- `:JmanTest`
- `:JmanTests` (discover and select tests through the LSP)
- `:JmanTestPattern [pattern]`
- `:JmanTestNearest`
- `:JmanRebuildIndex`
- `:JmanClearCache`
- `:JmanRestartLsp`
- `:JmanChangeSignature`

Default keymaps use the `<leader>j` prefix. `require("jman").status()` returns a
compact statusline component such as `JMAN:ready`.

JUnit methods also receive standard LSP CodeLens actions. Use
`vim.lsp.codelens.run()` on a `Run Test` lens to execute the exact JMAN selector.

Run `:checkhealth jman` to validate Neovim and the configured JMAN executable.

Build commands are intentionally enabled only for native `jman.toml` workspaces.
Maven and Gradle projects still receive the full JMAN Java language server, model
sync, navigation, completion, diagnostics, and refactoring support.
