vim.opt.runtimepath:prepend(vim.fn.getcwd() .. "/editors/neovim")

local jman = require("jman")
jman.setup({
	cmd = "/bin/false",
	java_home = "/language-jdk",
	build_java_home = "/build-jdk",
	build_sync = "manual",
	extra_env = { JMAN_TEST_ENVIRONMENT = "configured" },
	keymaps = false,
	notify = false,
})

local configuration = jman.config()
assert(configuration.cmd == "/bin/false", "Configured executable was not retained")
assert(configuration.build_java_home == "/build-jdk", "Configured build JDK was not retained")
assert(configuration.build_sync == "manual", "Configured synchronization mode was not retained")
local server_environment = jman._test.server_environment()
assert(server_environment.JAVA_HOME == "/language-jdk", "Language-server JAVA_HOME was not exported")
assert(server_environment.JMAN_JAVA_LSP_BUILD_JAVA_HOME == "/build-jdk", "Language-server build JAVA_HOME was not exported")
assert(server_environment.JMAN_TEST_ENVIRONMENT == "configured", "Additional server environment was not exported")

for _, command in ipairs({
	"JmanStatus",
	"JmanSync",
	"JmanCheck",
	"JmanBuild",
	"JmanRun",
	"JmanTest",
	"JmanCoverage",
	"JmanCoveragePattern",
	"JmanCoverageNearest",
	"JmanTests",
	"JmanTestPattern",
	"JmanTestNearest",
	"JmanCodeAction",
	"JmanOrganizeImports",
	"JmanChangeSignature",
	"JmanRebuildIndex",
	"JmanClearCache",
	"JmanRestartLsp",
}) do
	assert(vim.fn.exists(":" .. command) == 2, command .. " was not registered")
end

local health_ok, health_error = pcall(require("jman.health").check)
assert(health_ok, "Health provider failed: " .. tostring(health_error))

vim.cmd("qa!")
