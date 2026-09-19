vim.opt.runtimepath:prepend(vim.fn.getcwd())

local runtime = vim.fn.getcwd() .. "/editors/neovim"
local jman = require("jman")
assert(vim.tbl_contains(vim.opt.runtimepath:get(), runtime), "repository bridge did not expose the Neovim runtime")
assert(type(jman.setup) == "function", "require('jman') must work when a plugin manager installs the repository")
vim.cmd("help jman.nvim")
assert(vim.api.nvim_buf_get_name(0):match("/editors/neovim/doc/jman.txt$"), "JMAN help must be available")

vim.cmd("qa!")
