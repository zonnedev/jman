vim.opt.runtimepath:prepend(vim.fn.getcwd() .. "/editors/neovim")

local jman = require("jman")
local temporary = vim.fn.tempname()
vim.fn.mkdir(temporary .. "/module/src/test/java/demo", "p")
vim.fn.writefile({ "manifest-version = 1" }, temporary .. "/jman.toml")
vim.fn.writefile({ "<project/>" }, temporary .. "/module/pom.xml")
local java = temporary .. "/module/src/test/java/demo/GreetingTest.java"
vim.fn.writefile({
	"package demo;",
	"final class GreetingTest {",
	"  @org.junit.jupiter.api.Test",
	"  void greets() {}",
	"}",
}, java)

local root, system = jman._test.root_and_system(java)
assert(root == temporary, "JMAN root must win over nested Maven markers")
assert(system == "jman", "JMAN build system must win")

vim.cmd("edit " .. vim.fn.fnameescape(java))
local selector = jman._test.test_selector(0, 4)
assert(selector == "demo.GreetingTest#greets", "nearest test selector was " .. tostring(selector))

local namespace = vim.api.nvim_create_namespace("jman-neovim-test")
local lsp_diagnostic = {
	range = {
		start = { line = 1, character = 12 },
		["end"] = { line = 1, character = 24 },
	},
	severity = 1,
	message = "cannot find symbol: Singleton",
	source = "javac",
	code = "compiler.err.cant.resolve",
}
vim.diagnostic.set(namespace, 0, {
	{
		lnum = 1,
		col = 12,
		end_lnum = 1,
		end_col = 24,
		severity = vim.diagnostic.severity.ERROR,
		message = lsp_diagnostic.message,
		user_data = { lsp = lsp_diagnostic },
	},
})
local diagnostics = jman._test.lsp_diagnostics(0)
assert(#diagnostics == 1, "expected one LSP diagnostic in code-action context")
assert(diagnostics[1].code == "compiler.err.cant.resolve", "code-action diagnostic lost its compiler code")

vim.cmd("qa!")
