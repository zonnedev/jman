local M = {}

function M.check()
	vim.health.start("jman.nvim")
	if vim.fn.has("nvim-0.11") == 1 then
		vim.health.ok("Neovim supports the native vim.lsp.config API")
	else
		vim.health.error("Neovim 0.11 or newer is required")
	end

	local ok, plugin = pcall(require, "jman")
	local jman = ok and plugin.executable() or vim.env.JMAN_BIN or vim.fn.exepath("jman")
	if jman and jman ~= "" and vim.fn.executable(jman) == 1 then
		vim.health.ok("JMAN executable: " .. jman)
	else
		vim.health.error("JMAN executable is unavailable: " .. tostring(jman), {
			"Install JMAN on PATH",
			"Or configure require('jman').setup({ cmd = '/absolute/path/to/jman' })",
		})
	end

	local options = ok and plugin.config() or {}
	local java_home = options.java_home or vim.env.JAVA_HOME
	if java_home and java_home ~= "" and vim.fn.executable(java_home .. "/bin/java") == 1 then
		vim.health.ok("Language-server Java home: " .. java_home)
	elseif vim.fn.executable("java") == 1 then
		vim.health.ok("Java is available on PATH")
	else
		vim.health.warn("Java is not available in JAVA_HOME or on PATH")
	end

	if options.build_java_home and options.build_java_home ~= "" then
		if vim.fn.executable(options.build_java_home .. "/bin/java") == 1 then
			vim.health.ok("External build-tool Java home: " .. options.build_java_home)
		else
			vim.health.error("build_java_home has no executable bin/java: " .. options.build_java_home)
		end
	else
		vim.health.info("External Maven and Gradle builds will use the runtime selected by JMAN")
	end

	local buffer = vim.api.nvim_get_current_buf()
	local name = vim.api.nvim_buf_get_name(buffer)
	if name ~= "" and ok then
		local root, system = plugin._test.root_and_system(name)
		if root then
			vim.health.ok(string.format("Detected %s workspace: %s", system, root))
		else
			vim.health.info("The current buffer is not inside a JMAN, Maven, or Gradle workspace")
		end
	end
end

return M
