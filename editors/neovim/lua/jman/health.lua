local M = {}

function M.check()
	vim.health.start("jman.nvim")
	if vim.fn.has("nvim-0.11") == 1 then
		vim.health.ok("Neovim supports the native vim.lsp.config API")
	else
		vim.health.error("Neovim 0.11 or newer is required")
	end

	local ok, plugin = pcall(require, "jman")
	local jman = ok and plugin.executable()
		or vim.env.JMAN_BIN
		or vim.env.JMAN_BIN
		or vim.fn.exepath("jman")
	if jman and jman ~= "" and vim.uv.fs_stat(jman) then
		vim.health.ok("JMAN executable: " .. jman)
	elseif jman == "jman" or jman == "" then
		vim.health.warn("JMAN was not found on PATH; configure require('jman').setup({ cmd = ... })")
	else
		vim.health.error("Configured JMAN executable does not exist: " .. jman)
	end

	if vim.fn.executable("java") == 1 or (vim.env.JAVA_HOME and vim.env.JAVA_HOME ~= "") then
		vim.health.ok("Java toolchain environment is available")
	else
		vim.health.info("JMAN can install the project JDK automatically when required")
	end
end

return M
